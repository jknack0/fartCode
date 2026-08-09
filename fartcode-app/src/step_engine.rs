//! Generalized step engine (E18-04/E18-05, #63/#64; ADR-0037 items 2–4).
//!
//! The board is a pipeline runner: entering an `agent_step` column either
//! launches the step (`on_enter: run`) or parks it behind the confirm
//! overlay (`on_enter: queue`, fired later by `step_confirm`); when the
//! step's agent settles, the column's `on_settle` either holds the card
//! for a human drag or advances it — through the same enter primitive, so
//! chains are legal (advancing into a run-mode step fires it; shelves and
//! human gates are inert).
//!
//! One task per card, for the card's whole life (ADR-0037 item 2): the
//! first `agent_step` entry provisions the worktree + linked task via the
//! SAME machinery as board dispatch (`provision_issue_task` — shared, not
//! forked); re-entering the current column REATTACHES, never respawns
//! (ADR-0032 item 3); a later step launches a NEW agent session in the
//! same task/worktree with the column's step config.
//!
//! **Session-scoped settle** (fix round): settle triggers are task-keyed
//! (ACP turn-complete, agent-PTY exit) and keep firing for a task long
//! after one step finished — old sessions stay alive because the board
//! never kills. The engine therefore keeps an in-memory LAUNCH REGISTRY
//! (same doctrine as parks: derived-state helpers, nothing stored): every
//! real launch records `{issue → column, session: unbound}`; the first
//! settle for that column binds the settling session's identity
//! (`acp:<conversation>` / `pty:<terminal>` — the backend cannot know the
//! id at launch time, since the frontend opens the session), acts, and
//! TOMBSTONES the entry; a consumed session's later triggers, a bound
//! entry's other sessions, and tombstoned entries all no-op. A settle
//! NEVER acts on an issue with a pending park — a parked step never
//! launched, so it cannot have settled (that would be a confirm-gate
//! bypass). With NO registry entry (restart — the registry is
//! memory-only — or a legacy `issue_dispatch` launch), settle acts iff
//! the current column is an agent_step with no pending park, reproducing
//! E17's restart auto-flip while still refusing to bypass a gate; the
//! act inserts a tombstoned entry so duplicates no-op.
//!
//! **Restart contract** (final round): after a restart the registry and
//! parks are empty, so the heuristic is the only path — and it must
//! never open a confirm gate. A registry-less settle on a column whose
//! `on_enter` is Queue therefore RE-PARKS the card (emitting `StepQueued`
//! so the confirm overlay visibly restores) instead of acting; run-mode
//! agent steps settle normally (E17 restart parity — seeded In Progress
//! is run-mode). The same rule closes the in-process window between the
//! enter DB-write and the park insert. Consumed-session sets are scoped
//! to a SETTLE EPOCH: every user-initiated column entry (engine enter,
//! lane drag, dispatch) starts a fresh epoch (consumed + registry entry
//! cleared) so the E17 rework loop — flip, re-drag back, same session
//! settles again — keeps working; settle-chained enters stay inside the
//! epoch that advanced them. Residual, by design: an unbound entry can be
//! bound by a stale-but-never-consumed session that beats the real one to
//! the first settle — closing that needs launch-time session identity,
//! which arrives when session opening moves into the backend.
//!
//! **The board never kills** (ADR-0037 item 11): no path in this engine
//! stops an agent. Moves only move state and emit events; superseding a
//! parked step drops a *pending* launch, never a running one.
//!
//! "Launching" here means emitting the `StepLaunch` directive event and
//! returning the same payload to the calling command — the agent terminal
//! is a UI resource, so the frontend performs the actual open. A launch
//! that reached a command outcome is marked DELIVERED; a settle-chained
//! launch stays undelivered (no listener until the UI wave) and re-entry
//! into its column re-emits the directive instead of misclassifying the
//! entry as a reattach.
//!
//! Parked steps are in-memory state, keyed by issue id. Deliberate: there
//! is no stored step status anywhere (doctrine), so a restart drops
//! pending confirms — the card is already IN the queue column, and
//! re-entering (or re-dragging) it re-parks. A park is dropped by: any
//! entry into a different column (engine enter, legacy lane drag, legacy
//! dispatch), launch via confirm, issue/project deletion, a column edit
//! that removes queue-ness, or the stale-park backstop at confirm time.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use fartcode_core::events::{EventBus, InternalEvent};
use fartcode_core::issues::columns::{
    compose_step_prompt, BoardColumn, ColumnKind, OnEnter, OnSettle,
};
use fartcode_core::issues::{build_dispatch_prompt, Issue, Lane};
use fartcode_core::settings::DEFAULT_AGENT;
use fartcode_core::tasks::{TaskDto, TaskStore};
use fartcode_core::Error;

use crate::app::App;
use crate::dispatch::provision_issue_task;

/// A queue-mode entry awaiting `step_confirm`.
#[derive(Debug, Clone)]
pub struct ParkedStep {
    pub issue_id: String,
    pub project_id: String,
    pub column_id: String,
}

/// One recorded launch (or heuristic settle) per issue — the most recent.
#[derive(Debug, Clone)]
struct LaunchEntry {
    column_id: String,
    project_id: String,
    /// Settling-session identity, bound at the FIRST settle for this
    /// entry (`None` until then).
    session: Option<String>,
    /// Tombstone: this entry already settled once; further settles no-op.
    settled: bool,
    /// The launch directive reached a command outcome (the frontend had a
    /// chance to open the session). Settle-chained launches stay `false`
    /// until re-entry re-emits them through a command.
    delivered: bool,
}

#[derive(Default)]
struct EngineState {
    parked: HashMap<String, ParkedStep>,
    launches: HashMap<String, LaunchEntry>,
    /// issue id → session identities that have already settled that issue
    /// once. A consumed session's every later trigger is stale.
    consumed: HashMap<String, HashSet<String>>,
}

/// What a settle trigger may do for one issue (decided atomically in
/// [`StepEngine::begin_settle`]).
enum SettleDecision {
    /// Apply the column's `on_settle`.
    Act,
    /// Stale / duplicate / parked — nothing happens.
    Skip,
    /// Registry-less settle on a queue column: the park was restored
    /// (restart contract) — the caller emits `StepQueued`.
    Repark,
}

/// Outcome of the atomic park-take inside `confirm_step`.
enum ParkTake {
    /// Park existed and matched the card's current column — taken, and a
    /// fresh launch entry begun in the same critical section.
    Taken(ParkedStep),
    /// Park existed but the card has left its column — dropped.
    Stale(ParkedStep),
    /// Nothing parked.
    None,
}

/// In-memory engine state: parks + launch registry, one mutex.
pub struct StepEngine {
    state: Mutex<EngineState>,
}

impl StepEngine {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(EngineState::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, EngineState> {
        self.state.lock().expect("step engine mutex poisoned")
    }

    fn park(&self, step: ParkedStep) {
        self.lock().parked.insert(step.issue_id.clone(), step);
    }

    pub fn peek_park(&self, issue_id: &str) -> Option<ParkedStep> {
        self.lock().parked.get(issue_id).cloned()
    }

    /// Read-only snapshot of a project's parks (E18-09 rehydration —
    /// `step_parked_list`). No state change, no events.
    pub fn parks_for_project(&self, project_id: &str) -> Vec<ParkedStep> {
        self.lock()
            .parked
            .values()
            .filter(|p| p.project_id == project_id)
            .cloned()
            .collect()
    }

    fn remove_park(&self, issue_id: &str) -> Option<ParkedStep> {
        self.lock().parked.remove(issue_id)
    }

    /// Records a real launch: a fresh, unbound, unsettled entry.
    fn record_launch(&self, issue_id: &str, project_id: &str, column_id: &str) {
        self.lock().launches.insert(
            issue_id.to_string(),
            LaunchEntry {
                column_id: column_id.to_string(),
                project_id: project_id.to_string(),
                session: None,
                settled: false,
                delivered: false,
            },
        );
    }

    /// Marks the issue's current launch as delivered (returned through a
    /// command outcome — the frontend saw it).
    fn mark_delivered(&self, issue_id: &str) {
        if let Some(entry) = self.lock().launches.get_mut(issue_id) {
            entry.delivered = true;
        }
    }

    /// Is the issue's current launch an UNDELIVERED one for `column_id`?
    fn undelivered_launch_for(&self, issue_id: &str, column_id: &str) -> bool {
        self.lock()
            .launches
            .get(issue_id)
            .is_some_and(|e| e.column_id == column_id && !e.delivered && !e.settled)
    }

    /// The settle decision + all its state transitions, atomically.
    fn begin_settle(
        &self,
        issue_id: &str,
        column: &BoardColumn,
        session: Option<&str>,
    ) -> SettleDecision {
        let mut st = self.lock();
        // Never act on a parked issue: the step never launched, so it
        // cannot have settled — acting would bypass the confirm gate.
        if st.parked.contains_key(issue_id) {
            return SettleDecision::Skip;
        }
        // A consumed session's triggers are stale, whatever the column.
        if let Some(s) = session {
            if st.consumed.get(issue_id).is_some_and(|set| set.contains(s)) {
                return SettleDecision::Skip;
            }
        }
        let matches_current = st.launches.get(issue_id).map(|e| e.column_id == column.id);
        match matches_current {
            Some(true) => {
                let entry = st.launches.get_mut(issue_id).expect("checked above");
                if entry.settled {
                    return SettleDecision::Skip; // tombstoned — duplicate no-ops
                }
                if let (Some(bound), Some(s)) = (entry.session.as_deref(), session) {
                    if bound != s {
                        return SettleDecision::Skip; // another session's launch
                    }
                }
                if let Some(s) = session {
                    entry.session.get_or_insert_with(|| s.to_string());
                }
                entry.settled = true; // tombstone before acting
            }
            // Entry for a column the card has left, or no entry at all
            // (restart — registry is memory-only — or a legacy
            // issue_dispatch launch that never touched the engine).
            Some(false) | None => {
                // Restart contract: a registry-less settle must never open
                // a confirm gate. A queue column's step can only fire via
                // step_confirm — RESTORE the park (the caller announces it
                // with StepQueued) instead of acting. The trigger is not
                // consumed: it did nothing.
                if column.on_enter == OnEnter::Queue {
                    st.parked.insert(
                        issue_id.to_string(),
                        ParkedStep {
                            issue_id: issue_id.to_string(),
                            project_id: column.project_id.clone(),
                            column_id: column.id.clone(),
                        },
                    );
                    return SettleDecision::Repark;
                }
                // Run-mode: act once via the heuristic and leave a
                // tombstoned entry so duplicates no-op. This reproduces
                // E17's restart auto-flip.
                st.launches.insert(
                    issue_id.to_string(),
                    LaunchEntry {
                        column_id: column.id.clone(),
                        project_id: column.project_id.clone(),
                        session: session.map(str::to_string),
                        settled: true,
                        delivered: true, // a session evidently ran here
                    },
                );
            }
        }
        if let Some(s) = session {
            st.consumed
                .entry(issue_id.to_string())
                .or_default()
                .insert(s.to_string());
        }
        SettleDecision::Act
    }

    /// Starts a fresh settle epoch for an issue (fix round item 3): a
    /// USER-initiated column entry clears the consumed-session set and
    /// the registry entry, so a re-entered step can settle again (E17
    /// rework loop). Settle-chained enters never call this — the session
    /// that advanced the card stays consumed for its epoch.
    fn begin_entry_epoch(&self, issue_id: &str) {
        let mut st = self.lock();
        st.consumed.remove(issue_id);
        st.launches.remove(issue_id);
    }

    /// Atomic confirm take (fix round, finding 7): verify-and-take the
    /// park AND begin the launch entry under ONE lock acquisition, so a
    /// racing settle either finds the park still present (and no-ops on
    /// the park guard) or finds it gone with the launch entry already in
    /// place.
    fn take_park_for_launch(&self, issue_id: &str, current_column: Option<&str>) -> ParkTake {
        let mut st = self.lock();
        let Some(parked) = st.parked.get(issue_id).cloned() else {
            return ParkTake::None;
        };
        if current_column != Some(parked.column_id.as_str()) {
            st.parked.remove(issue_id);
            return ParkTake::Stale(parked);
        }
        st.parked.remove(issue_id);
        st.launches.insert(
            issue_id.to_string(),
            LaunchEntry {
                column_id: parked.column_id.clone(),
                project_id: parked.project_id.clone(),
                session: None,
                settled: false,
                delivered: false,
            },
        );
        ParkTake::Taken(parked)
    }

    /// Sweeps every trace of an issue (park + registry + consumed).
    /// Returns the dropped park, if any, for the cleared event.
    fn forget_issue(&self, issue_id: &str) -> Option<ParkedStep> {
        let mut st = self.lock();
        st.launches.remove(issue_id);
        st.consumed.remove(issue_id);
        st.parked.remove(issue_id)
    }

    /// Sweeps every trace of a project's issues — parked AND
    /// launched-but-unparked (fix round item 4a: the consumed sets of
    /// launched issues must go too; `consumed` is only ever populated
    /// alongside a registry entry, so the two collections cover it).
    /// Returns dropped parks.
    fn forget_project(&self, project_id: &str) -> Vec<ParkedStep> {
        let mut st = self.lock();
        let launched: Vec<String> = st
            .launches
            .iter()
            .filter(|(_, e)| e.project_id == project_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in launched {
            st.launches.remove(&id);
            st.consumed.remove(&id);
        }
        let dropped: Vec<String> = st
            .parked
            .iter()
            .filter(|(_, p)| p.project_id == project_id)
            .map(|(id, _)| id.clone())
            .collect();
        let mut parks = Vec::new();
        for id in dropped {
            st.consumed.remove(&id);
            if let Some(p) = st.parked.remove(&id) {
                parks.push(p);
            }
        }
        parks
    }

    /// Test-only visibility: does ANY engine state exist for the issue?
    #[cfg(test)]
    fn has_state(&self, issue_id: &str) -> bool {
        let st = self.lock();
        st.parked.contains_key(issue_id)
            || st.launches.contains_key(issue_id)
            || st.consumed.contains_key(issue_id)
    }

    /// Takes every park held on `column_id` (a column edit removed its
    /// queue-ness). When `still_runnable`, each affected issue keeps an
    /// UNDELIVERED launch entry so a same-column re-entry launches fresh
    /// instead of "reattaching" to a session that never started.
    fn take_parks_for_column(&self, column_id: &str, still_runnable: bool) -> Vec<ParkedStep> {
        let mut st = self.lock();
        let ids: Vec<String> = st
            .parked
            .iter()
            .filter(|(_, p)| p.column_id == column_id)
            .map(|(id, _)| id.clone())
            .collect();
        let mut parks = Vec::new();
        for id in ids {
            if let Some(p) = st.parked.remove(&id) {
                if still_runnable {
                    st.launches.insert(
                        id.clone(),
                        LaunchEntry {
                            column_id: p.column_id.clone(),
                            project_id: p.project_id.clone(),
                            session: None,
                            settled: false,
                            delivered: false,
                        },
                    );
                }
                parks.push(p);
            }
        }
        parks
    }
}

impl Default for StepEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Launch payload — mirrors `DispatchOutcome`'s contract (empty prompt +
/// `reattached` on reattach) and rides both the command return value and
/// the `StepLaunch` event.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepLaunchInfo {
    pub task: TaskDto,
    pub prompt: String,
    pub provider: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub reattached: bool,
}

/// What `issue_enter_column` / `step_confirm` did.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnterOutcome {
    pub issue: Issue,
    /// `"launched"` | `"queued"` | `"reattached"` | `"inert"` (shelf /
    /// human gate — nothing to do).
    pub step: String,
    pub launch: Option<StepLaunchInfo>,
}

/// One parked (queue-mode) step, as `step_parked_list` returns it —
/// the same fields `step:queued` carries (agent config re-resolved at
/// query time, honoring column edits made while parked), so the frontend
/// reuses its event reducer to rehydrate after a webview reload. Parks
/// precede provisioning, so there is no task id to carry.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParkedStepDto {
    pub issue_id: String,
    pub project_id: String,
    pub column_id: String,
    pub provider: String,
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// The project's current parks (E18-09 rehydration): a pure read of the
/// in-memory registry plus DB lookups to resolve each park's agent config.
/// Mutates nothing and emits nothing. A park whose issue or column has
/// vanished underneath it is skipped, not an error — deletion sweeps own
/// the cleanup, and a query must never fail the whole list for one
/// stale entry.
pub fn parked_list(app: &App, project_id: &str) -> Result<Vec<ParkedStepDto>, String> {
    let mut out = Vec::new();
    for parked in app.steps.parks_for_project(project_id) {
        let Some(issue) = app.issues.get(&parked.issue_id).map_err(String::from)? else {
            continue;
        };
        let Some(column) = app.columns.get(&parked.column_id).map_err(String::from)? else {
            continue;
        };
        let (provider, model, effort) = resolve_agent(app, &issue, &column)?;
        out.push(ParkedStepDto {
            issue_id: parked.issue_id,
            project_id: parked.project_id,
            column_id: parked.column_id,
            provider,
            model,
            effort,
        });
    }
    Ok(out)
}

fn cleared_event(parked: ParkedStep) -> InternalEvent {
    InternalEvent::StepQueueCleared {
        issue_id: parked.issue_id,
        project_id: parked.project_id,
        column_id: parked.column_id,
    }
}

/// Resolves the step's agent config. A NULL column field falls back to
/// today's dispatch resolution — issue override, then the project default
/// agent — so the seeded In Progress column (all NULLs) behaves
/// byte-identically to E17-03 dispatch. Effort has no issue-level
/// override; it is the column's or absent.
fn resolve_agent(
    app: &App,
    issue: &Issue,
    column: &BoardColumn,
) -> Result<(String, Option<String>, Option<String>), String> {
    let provider = match &column.step_provider {
        Some(p) => p.clone(),
        None => match &issue.provider {
            Some(p) => p.clone(),
            None => app.settings.get(&DEFAULT_AGENT).map_err(String::from)?,
        },
    };
    let model = column.step_model.clone().or_else(|| issue.model.clone());
    let effort = column.step_effort.clone();
    Ok((provider, model, effort))
}

/// The step's prompt: the reference packet (built by the SAME builder as
/// dispatch), framed by the column's step_prompt when one is set, and — for
/// a consenting project whose card has a dossier — ending with the
/// feature-log append instruction (E19-02, #71; ADR-0038 item 2).
///
/// The instruction is injected HERE rather than seeded into
/// `SEED_COLUMNS[].step_prompt` at project-create time, which is where the
/// ticket first pointed. A stored prompt is written before consent is
/// asked, survives consent being revoked, is edited by users, and cannot
/// follow a [`fartcode_core::skills::FEATURE_LOG_VERSION`] bump without a
/// migration — four ways for a project that declined to end up holding an
/// instruction to write dossiers. Assembly time is the only place that can
/// read today's answer, so the seeded columns keep their NULL prompts (=
/// the built-in dispatch packet) and the instruction rides the packet.
fn step_prompt_for(app: &App, column: &BoardColumn, issue: &Issue) -> String {
    let finished: Vec<String> = issue
        .blockers
        .iter()
        .filter(|b| b.counts_as_done)
        .map(|b| b.title.clone())
        .collect();
    let packet = build_dispatch_prompt(issue, &finished);
    let composed = compose_step_prompt(column.step_prompt.as_deref(), &packet);
    crate::skills::with_append_instruction(app, issue, &column.name, composed)
}

/// The engine's move: enter `column_id`, then do what the column says.
///
/// State first (mirror + lane sync via the core primitive, one
/// `IssueUpdated`), then `on_enter` for agent steps: `run` launches
/// immediately; `queue` parks and emits `StepQueued` for the confirm
/// overlay. Shelves and human gates are inert. Any entry supersedes a
/// pending park (item 5) — announced with `StepQueueCleared` unless the
/// entry re-targets the parked column (a re-queue or launch immediately
/// replaces it).
///
/// Settle calls this directly (staying inside the advancing session's
/// settle epoch); command paths go through [`enter_column_from_command`],
/// which starts a fresh epoch and marks the returned launch as delivered.
pub fn enter_column(
    app: &App,
    issue_id: &str,
    column_id: &str,
    position: Option<i64>,
) -> Result<EnterOutcome, String> {
    enter_column_inner(app, issue_id, column_id, position, false)
}

fn enter_column_inner(
    app: &App,
    issue_id: &str,
    column_id: &str,
    position: Option<i64>,
    user_epoch: bool,
) -> Result<EnterOutcome, String> {
    let issue = app
        .issues
        .get(issue_id)
        .map_err(String::from)?
        .ok_or_else(|| format!("issue not found: {issue_id}"))?;
    let column = app
        .columns
        .get(column_id)
        .map_err(String::from)?
        .ok_or_else(|| String::from(Error::BoardColumnNotFound(column_id.into())))?;
    // Previous column via the SAME resolution settle uses (the
    // authoritative column_id since E18-07) — a card re-entering its own
    // column must reattach, never respawn (ADR-0032 item 3).
    let prev_column_id = current_column(app, &issue).map(|c| c.id);
    // Captured BEFORE any epoch reset: an undelivered chained launch for
    // the target column must be re-emitted (finding 10), even though a
    // user entry then starts a fresh epoch.
    let undelivered_here = app.steps.undelivered_launch_for(issue_id, &column.id);

    // State write FIRST (final round item 4b): an errored move must leave
    // the park untouched — no silent destruction.
    let issue = app
        .issues
        .enter_column(issue_id, &column.id, position)
        .map_err(String::from)?;

    if user_epoch {
        // A user gesture opens a new settle epoch (final round item 3):
        // consumed sessions and the stale registry entry are gone, so a
        // re-entered step can settle again (E17 rework loop).
        app.steps.begin_entry_epoch(issue_id);
    }

    let removed_park = app.steps.remove_park(issue_id);
    let had_park_here = match removed_park {
        Some(p) if p.column_id == column.id => true,
        Some(p) => {
            app.event_bus.send(cleared_event(p));
            false
        }
        None => false,
    };

    if column.kind != ColumnKind::AgentStep {
        return Ok(EnterOutcome {
            issue,
            step: "inert".into(),
            launch: None,
        });
    }
    let (provider, model, effort) = resolve_agent(app, &issue, &column)?;
    match column.on_enter {
        OnEnter::Queue => {
            app.steps.park(ParkedStep {
                issue_id: issue.id.clone(),
                project_id: issue.project_id.clone(),
                column_id: column.id.clone(),
            });
            app.event_bus.send(InternalEvent::StepQueued {
                issue_id: issue.id.clone(),
                project_id: issue.project_id.clone(),
                column_id: column.id.clone(),
                provider,
                model,
                effort,
            });
            Ok(EnterOutcome {
                issue,
                step: "queued".into(),
                launch: None,
            })
        }
        OnEnter::Run => {
            // Reattach requires evidence a session was opened for this
            // column: same-column re-entry, AND no park was pending here
            // (a park means this entry's session never started — finding
            // 14), AND no undelivered launch is waiting here (a chained
            // directive nobody opened must be re-emitted, not swallowed —
            // finding 10; captured before the epoch reset).
            let reattach_ok = prev_column_id.as_deref() == Some(column.id.as_str())
                && !had_park_here
                && !undelivered_here;
            let (issue, launch) =
                launch_step(app, issue, &column, reattach_ok, provider, model, effort)?;
            let step = if launch.reattached {
                "reattached"
            } else {
                "launched"
            };
            Ok(EnterOutcome {
                issue,
                step: step.into(),
                launch: Some(launch),
            })
        }
    }
}

/// Command-path enter: same as [`enter_column`], but as a USER gesture —
/// it starts a fresh settle epoch, and the returned launch counts as
/// DELIVERED (the frontend received the outcome and opens the session
/// from it).
pub fn enter_column_from_command(
    app: &App,
    issue_id: &str,
    column_id: &str,
    position: Option<i64>,
) -> Result<EnterOutcome, String> {
    let outcome = enter_column_inner(app, issue_id, column_id, position, true)?;
    if outcome.step == "launched" {
        app.steps.mark_delivered(issue_id);
    }
    Ok(outcome)
}

/// Starts a fresh settle epoch for an issue — for user-gesture paths that
/// bypass the engine's enter (legacy lane drag, legacy dispatch).
pub fn begin_entry_epoch(app: &App, issue_id: &str) {
    app.steps.begin_entry_epoch(issue_id);
}

/// Fires a parked step (E18-04 item 2). Single-shot: the park is
/// verified against the card's current column and taken atomically —
/// one lock acquisition covers check + take + launch-entry begin (fix
/// round, finding 7) — so a second confirm, or one racing a
/// settle-advance, gets the typed [`Error::NoParkedStep`] instead of a
/// double launch. The agent config is re-resolved at fire time, honoring
/// column edits made while parked.
pub fn confirm_step(app: &App, issue_id: &str) -> Result<EnterOutcome, String> {
    let issue = match app.issues.get(issue_id).map_err(String::from)? {
        Some(issue) => issue,
        None => {
            // Issue vanished under the park (deletion sweeps should have
            // cleared it; backstop anyway).
            if let Some(parked) = app.steps.remove_park(issue_id) {
                app.event_bus.send(cleared_event(parked));
            }
            return Err(String::from(Error::NoParkedStep(issue_id.into())));
        }
    };
    let parked = match app
        .steps
        .take_park_for_launch(issue_id, issue.column_id.as_deref())
    {
        ParkTake::Taken(parked) => parked,
        ParkTake::Stale(parked) => {
            app.event_bus.send(cleared_event(parked));
            return Err(String::from(Error::NoParkedStep(issue_id.into())));
        }
        ParkTake::None => return Err(String::from(Error::NoParkedStep(issue_id.into()))),
    };
    let column = app
        .columns
        .get(&parked.column_id)
        .map_err(String::from)?
        .ok_or_else(|| String::from(Error::NoParkedStep(issue_id.into())))?;
    let (provider, model, effort) = resolve_agent(app, &issue, &column)?;
    // Never "reattach" here: a parked entry means THIS entry's session was
    // never started — confirm always launches (provision or new session).
    let (issue, launch) = launch_step(app, issue, &column, false, provider, model, effort)?;
    app.steps.mark_delivered(&issue.id);
    Ok(EnterOutcome {
        issue,
        step: "launched".into(),
        launch: Some(launch),
    })
}

/// Launches (or reattaches) the step's agent and emits `StepLaunch`.
///
/// Existing linked task → reattach when `reattach_ok` (same-column
/// re-entry with an opened session: empty prompt, focus semantics —
/// `DispatchOutcome` parity), else a NEW session in the SAME
/// task/worktree with the column's prompt. No linked task (or its task
/// was deleted) → today's dispatch provisioning, via the shared helper.
/// Real launches (re)start the issue's registry entry, unbound.
fn launch_step(
    app: &App,
    issue: Issue,
    column: &BoardColumn,
    reattach_ok: bool,
    provider: String,
    model: Option<String>,
    effort: Option<String>,
) -> Result<(Issue, StepLaunchInfo), String> {
    let existing_task = match &issue.linked_task_id {
        Some(task_id) => app.tasks.get(task_id).map_err(String::from)?,
        None => None,
    };
    let (task, issue, prompt, reattached) = match existing_task {
        Some(task) if reattach_ok => (TaskDto::from(&task), issue, String::new(), true),
        Some(task) => {
            let prompt = step_prompt_for(app, column, &issue);
            (TaskDto::from(&task), issue, prompt, false)
        }
        None => {
            let (task, issue) = provision_issue_task(app, &issue)?;
            let prompt = step_prompt_for(app, column, &issue);
            (task, issue, prompt, false)
        }
    };
    if !reattached {
        app.steps
            .record_launch(&issue.id, &issue.project_id, &column.id);
    }
    let info = StepLaunchInfo {
        task,
        prompt,
        provider,
        model,
        effort,
        reattached,
    };
    app.event_bus.send(InternalEvent::StepLaunch {
        issue_id: issue.id.clone(),
        project_id: issue.project_id.clone(),
        column_id: column.id.clone(),
        task_id: info.task.id.clone(),
        prompt: info.prompt.clone(),
        provider: info.provider.clone(),
        model: info.model.clone(),
        effort: info.effort.clone(),
        reattached: info.reattached,
    });
    Ok((issue, info))
}

/// Agent-settle hook (E18-05, #64) — the generalization of the E17-03
/// auto-flip, reached from BOTH triggers through
/// `dispatch::flip_issues_for_task` and friends. `session` is the
/// settling session's identity (`acp:<conversation>` / `pty:<terminal>`),
/// `None` when the caller has no identity (legacy wrapper).
///
/// For every issue linked to the task, reads the CURRENT column config
/// and consults the launch registry ([`StepEngine::begin_settle`] — park
/// guard, session correlation, tombstones, restart heuristic). When the
/// registry says act:
/// - `on_settle: hold` → emit `StepSettled` (step-done is DERIVED state —
///   nothing stored);
/// - `on_settle: advance` → enter `advance_to` (or the next column by
///   position) THROUGH the engine's enter — chains are legal. Advancing
///   off the end of the board degrades to hold.
///
/// Returns the number of issues settled (held or advanced).
pub fn settle_issues_for_task(app: &App, task_id: &str, session: Option<&str>) -> usize {
    let issues = match app.issues.list_by_linked_task(task_id) {
        Ok(issues) => issues,
        Err(e) => {
            tracing::warn!(task_id, error = %e, "settle lookup failed");
            return 0;
        }
    };
    let mut settled = 0;
    for issue in issues {
        let Some(column) = current_column(app, &issue) else {
            continue;
        };
        if column.kind != ColumnKind::AgentStep {
            continue; // dragged elsewhere → stays put (E17-03 parity)
        }
        match app.steps.begin_settle(&issue.id, &column, session) {
            SettleDecision::Skip => continue, // parked / stale / tombstoned
            SettleDecision::Repark => {
                // Restart contract: the confirm gate was restored — tell
                // the overlay (park already re-inserted atomically).
                match resolve_agent(app, &issue, &column) {
                    Ok((provider, model, effort)) => {
                        app.event_bus.send(InternalEvent::StepQueued {
                            issue_id: issue.id.clone(),
                            project_id: issue.project_id.clone(),
                            column_id: column.id.clone(),
                            provider,
                            model,
                            effort,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(issue = %issue.id, error = %e, "repark resolve failed")
                    }
                }
                continue; // nothing settled
            }
            SettleDecision::Act => {}
        }
        match column.on_settle {
            OnSettle::Hold => {
                app.event_bus.send(InternalEvent::StepSettled {
                    issue_id: issue.id.clone(),
                    project_id: issue.project_id.clone(),
                    column_id: column.id.clone(),
                    task_id: task_id.to_string(),
                });
                settled += 1;
            }
            OnSettle::Advance => {
                let target = column
                    .advance_to
                    .clone()
                    .or_else(|| next_column_id(app, &column));
                match target {
                    None => {
                        // Last column and no explicit target: nowhere to
                        // advance — hold semantics.
                        app.event_bus.send(InternalEvent::StepSettled {
                            issue_id: issue.id.clone(),
                            project_id: issue.project_id.clone(),
                            column_id: column.id.clone(),
                            task_id: task_id.to_string(),
                        });
                        settled += 1;
                    }
                    Some(target) => match enter_column(app, &issue.id, &target, None) {
                        Ok(_) => settled += 1,
                        Err(e) => {
                            tracing::warn!(issue = %issue.id, error = %e, "settle advance failed")
                        }
                    },
                }
            }
        }
    }
    settled
}

/// The issue's current column: the authoritative `column_id` (E18-07,
/// #66 — migration 0008 backfilled every row, so the pre-flip seed_lane
/// fallback is gone). `None` only for out-of-contract rows, which fail
/// toward INERT: no settle, no launch.
fn current_column(app: &App, issue: &Issue) -> Option<BoardColumn> {
    let column_id = issue.column_id.as_ref()?;
    let column = app.columns.get(column_id).ok()??;
    if column.project_id == issue.project_id {
        Some(column)
    } else {
        None
    }
}

/// The next column by board position (None at the end of the board).
fn next_column_id(app: &App, column: &BoardColumn) -> Option<String> {
    app.columns
        .list_for_project(&column.project_id)
        .ok()?
        .into_iter()
        .filter(|c| c.position > column.position)
        .min_by_key(|c| c.position)
        .map(|c| c.id)
}

/// Manual lane drag (legacy `issue_move`): a CROSS-lane drag is a user
/// gesture that starts a fresh settle epoch and supersedes a parked step
/// — unless the drag stays where the card already is: a SAME-lane
/// reorder (fix round, finding 13 — the card isn't leaving its column,
/// seeded or not; no epoch change either) or a drag targeting the parked
/// column's own lane (park kept; the epoch still resets — the card is
/// arriving fresh as far as settles are concerned).
pub fn on_lane_move(app: &App, issue_id: &str, lane: Lane) {
    // Same-lane reorder: position-only, nothing changes here.
    if let Ok(Some(issue)) = app.issues.get(issue_id) {
        if issue.lane == lane {
            return;
        }
    }
    on_lane_move_committed(app, issue_id, lane)
}

/// [`on_lane_move`]'s body once the pre-move lane check has been decided
/// by the caller. Fail-closed ordering fix (#66): `issue_move` now runs
/// the fallible `move_to` FIRST and calls this only on success, passing
/// the PRE-move lane comparison result via the wrapper above being
/// skipped — a refused move (seeded target column deleted) must leave
/// the epoch registry and any park untouched. The same-lane early
/// return keys on the PRE-move lane, so post-move callers cannot use
/// the self-reading wrapper.
pub fn on_lane_move_committed(app: &App, issue_id: &str, lane: Lane) {
    app.steps.begin_entry_epoch(issue_id);
    let Some(parked) = app.steps.peek_park(issue_id) else {
        return;
    };
    let staying = app
        .columns
        .list_for_project(&parked.project_id)
        .ok()
        .and_then(|cols| {
            cols.into_iter()
                .find(|c| c.seed_lane.as_deref() == Some(lane.as_str()))
        })
        .is_some_and(|c| c.id == parked.column_id);
    if staying {
        return;
    }
    drop_parked_step(app, issue_id);
}

/// Unconditionally drops a park (dispatch entry). Emits
/// `StepQueueCleared` when one existed.
pub fn drop_parked_step(app: &App, issue_id: &str) {
    if let Some(parked) = app.steps.remove_park(issue_id) {
        app.event_bus.send(cleared_event(parked));
    }
}

/// Issue deleted (fix round, finding 4): sweep the park (announcing the
/// clear) and every registry trace, so nothing leaks and no overlay is
/// left waiting on a dead card.
pub fn on_issue_deleted(app: &App, issue_id: &str) {
    if let Some(parked) = app.steps.forget_issue(issue_id) {
        app.event_bus.send(cleared_event(parked));
    }
}

/// Project deleted (fix round, finding 4): the FK cascade deletes the
/// issues without per-issue events, so sweep by project.
pub fn on_project_deleted(app: &App, project_id: &str) {
    for parked in app.steps.forget_project(project_id) {
        app.event_bus.send(cleared_event(parked));
    }
}

/// Column edit removed its queue-ness (fix round, finding 14 —
/// `on_enter` queue→run or kind away from agent_step): every park held on
/// the column is dropped with `StepQueueCleared`. When the column is
/// still a runnable step, each affected issue keeps an UNDELIVERED launch
/// entry so a same-column re-entry launches fresh instead of
/// "reattaching" to a session that never started.
pub fn on_column_lost_queue(app: &App, column_id: &str, still_runnable: bool) {
    for parked in app.steps.take_parks_for_column(column_id, still_runnable) {
        app.event_bus.send(cleared_event(parked));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use fartcode_core::issues::columns::{ColumnPatch, ColumnStore, NewColumn};
    use fartcode_core::issues::NewIssue;

    fn fixture() -> Arc<App> {
        let app = App::init(Some(":memory:")).unwrap();
        {
            let conn = app.db.conn().lock().unwrap();
            conn.execute_batch(
                "INSERT INTO projects (id, name, path) VALUES ('p1', 'proj', '/tmp/proj');",
            )
            .unwrap();
            fartcode_core::issues::columns::seed_default_columns(&conn, "p1").unwrap();
        }
        app
    }

    fn with_task(app: &App, task_id: &str) {
        app.db
            .conn()
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO tasks (id, project_id, name, status)
                 VALUES (?1, 'p1', 't', 'in_progress')",
                [task_id],
            )
            .unwrap();
    }

    fn new_issue(app: &App, title: &str) -> Issue {
        new_issue_in(app, title, None)
    }

    fn new_issue_in(app: &App, title: &str, lane: Option<Lane>) -> Issue {
        app.issues
            .create(NewIssue {
                project_id: "p1".into(),
                title: title.into(),
                body: None,
                acceptance: Vec::new(),
                lane,
                provider: None,
                model: None,
                prd_path: None,
                prd_section: None,
                external_ref: None,
                dossier_path: None,
            })
            .unwrap()
    }

    fn column(app: &App, name: &str) -> BoardColumn {
        ColumnStore::new(app.db.clone())
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no column named {name}"))
    }

    fn make_step(
        app: &App,
        name: &str,
        on_enter: OnEnter,
        on_settle: OnSettle,
        advance_to: Option<String>,
    ) -> BoardColumn {
        ColumnStore::new(app.db.clone())
            .create(NewColumn {
                project_id: "p1".into(),
                name: name.into(),
                kind: ColumnKind::AgentStep,
                counts_as_done: false,
                is_landing: false,
                on_enter: Some(on_enter),
                on_settle: Some(on_settle),
                advance_to,
                step_prompt: None,
                step_provider: None,
                step_model: None,
                step_effort: None,
                step_tools: None,
            })
            .unwrap()
    }

    /// Drains the bus into only the step-engine events.
    fn step_events(rx: &mut tokio::sync::broadcast::Receiver<InternalEvent>) -> Vec<InternalEvent> {
        let mut out = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if matches!(
                event,
                InternalEvent::StepLaunch { .. }
                    | InternalEvent::StepQueued { .. }
                    | InternalEvent::StepQueueCleared { .. }
                    | InternalEvent::StepSettled { .. }
            ) {
                out.push(event);
            }
        }
        out
    }

    fn task_count(app: &App) -> i64 {
        app.db
            .conn()
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
            .unwrap()
    }

    fn issue_column_id(app: &App, issue_id: &str) -> Option<String> {
        app.issues.get(issue_id).unwrap().unwrap().column_id.clone()
    }

    #[test]
    fn enter_shelf_is_inert_and_run_step_launches_in_the_linked_task() {
        let app = fixture();
        let mut rx = app.event_bus.subscribe();
        let issue = new_issue(&app, "card");
        with_task(&app, "t1");
        app.issues.set_linked_task(&issue.id, Some("t1")).unwrap();

        // Shelf: state moves, engine does nothing.
        let ready = column(&app, "Ready");
        let outcome = enter_column_from_command(&app, &issue.id, &ready.id, None).unwrap();
        assert_eq!(outcome.step, "inert");
        assert!(outcome.launch.is_none());
        assert!(step_events(&mut rx).is_empty());

        // Run-mode agent step (Quick, pinned claude · haiku): launches a
        // session in the LINKED task; lane untouched (Quick is
        // non-seeded); prompt is the packet byte-identical (no framing).
        let quick = column(&app, "Quick");
        let outcome = enter_column_from_command(&app, &issue.id, &quick.id, None).unwrap();
        assert_eq!(outcome.step, "launched");
        let launch = outcome.launch.unwrap();
        assert_eq!(launch.task.id, "t1");
        assert_eq!(launch.provider, "claude");
        assert_eq!(launch.model.as_deref(), Some("haiku"));
        assert!(launch.effort.is_none());
        assert!(!launch.reattached);
        assert!(launch
            .prompt
            .starts_with("You are implementing an issue from the project board."));
        assert_eq!(outcome.issue.lane, Lane::Ready); // unchanged
        assert_eq!(outcome.issue.column_id.as_deref(), Some(quick.id.as_str()));
        let events = step_events(&mut rx);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            InternalEvent::StepLaunch { task_id, provider, reattached: false, .. }
                if task_id == "t1" && provider == "claude"
        ));
        assert_eq!(task_count(&app), 1, "no provisioning when a task is linked");
    }

    /// ADR-0032 item 3 preserved: re-entry reattaches, never respawns.
    #[test]
    fn reentry_reattaches_never_respawns() {
        let app = fixture();
        let mut rx = app.event_bus.subscribe();
        let issue = new_issue(&app, "card");
        with_task(&app, "t1");
        app.issues.set_linked_task(&issue.id, Some("t1")).unwrap();
        let quick = column(&app, "Quick");

        enter_column_from_command(&app, &issue.id, &quick.id, None).unwrap();
        let outcome = enter_column_from_command(&app, &issue.id, &quick.id, None).unwrap();
        assert_eq!(outcome.step, "reattached");
        let launch = outcome.launch.unwrap();
        assert!(launch.reattached);
        assert_eq!(launch.prompt, ""); // DispatchOutcome parity
        assert_eq!(launch.task.id, "t1");
        assert_eq!(task_count(&app), 1);
        let events = step_events(&mut rx);
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[1],
            InternalEvent::StepLaunch {
                reattached: true,
                ..
            }
        ));
    }

    /// E18-07: an out-of-contract row with NO column (pre-0008 shape,
    /// unreachable post-migration) fails INERT on settle — no flip, no
    /// launch — instead of resolving through the deleted lane fallback.
    #[test]
    fn columnless_row_settles_nothing() {
        let app = fixture();
        let issue = new_issue_in(&app, "orphan", Some(Lane::InProgress));
        app.db
            .conn()
            .lock()
            .unwrap()
            .execute(
                "UPDATE issues SET column_id = NULL WHERE id = ?1",
                [&issue.id],
            )
            .unwrap();
        with_task(&app, "t1");
        app.issues.set_linked_task(&issue.id, Some("t1")).unwrap();
        assert_eq!(settle_issues_for_task(&app, "t1", Some("pty:x")), 0);
        assert_eq!(
            app.issues.get(&issue.id).unwrap().unwrap().lane,
            Lane::InProgress,
            "no lane-fallback flip"
        );
    }

    #[test]
    fn queue_parks_until_confirm_and_confirm_is_single_shot() {
        let app = fixture();
        let mut rx = app.event_bus.subscribe();
        let issue = new_issue(&app, "card");
        with_task(&app, "t1");
        app.issues.set_linked_task(&issue.id, Some("t1")).unwrap();
        let col_store = ColumnStore::new(app.db.clone());
        let plan = col_store
            .create(NewColumn {
                project_id: "p1".into(),
                name: "Plan".into(),
                kind: ColumnKind::AgentStep,
                counts_as_done: false,
                is_landing: false,
                on_enter: Some(OnEnter::Queue),
                on_settle: Some(OnSettle::Hold),
                advance_to: None,
                step_prompt: Some("Grill me on the plan.".into()),
                step_provider: None,
                step_model: None,
                step_effort: Some("high".into()),
                step_tools: None,
            })
            .unwrap();

        // Queue: parks, emits StepQueued with the RESOLVED agent config
        // (column NULL provider → project default), launches nothing.
        let outcome = enter_column_from_command(&app, &issue.id, &plan.id, None).unwrap();
        assert_eq!(outcome.step, "queued");
        assert!(outcome.launch.is_none());
        let events = step_events(&mut rx);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            InternalEvent::StepQueued { column_id, provider, effort: Some(e), .. }
                if column_id == &plan.id && provider == "claude" && e == "high"
        ));

        // Confirm fires it: new session in the linked task, column prompt
        // framing the packet.
        let outcome = confirm_step(&app, &issue.id).unwrap();
        assert_eq!(outcome.step, "launched");
        let launch = outcome.launch.unwrap();
        assert_eq!(launch.task.id, "t1");
        assert_eq!(launch.effort.as_deref(), Some("high"));
        assert!(launch.prompt.starts_with("Grill me on the plan.\n\n---\n"));
        assert!(launch
            .prompt
            .contains("You are implementing an issue from the project board."));
        assert!(matches!(
            &step_events(&mut rx)[..],
            [InternalEvent::StepLaunch {
                reattached: false,
                ..
            }]
        ));

        // Single-shot: a second confirm is the typed error.
        let err = confirm_step(&app, &issue.id).unwrap_err();
        assert!(err.contains("no parked step"), "got: {err}");
    }

    /// Fix round, finding 7: the park take is atomic — concurrent
    /// confirms launch exactly once — and a card that left the parked
    /// column gets the typed error plus the cleared event.
    #[test]
    fn concurrent_confirms_launch_exactly_once() {
        let app = fixture();
        let issue = new_issue(&app, "card");
        with_task(&app, "t1");
        app.issues.set_linked_task(&issue.id, Some("t1")).unwrap();
        let plan = make_step(&app, "Plan", OnEnter::Queue, OnSettle::Hold, None);
        enter_column_from_command(&app, &issue.id, &plan.id, None).unwrap();
        let mut rx = app.event_bus.subscribe();

        let results: Vec<Result<EnterOutcome, String>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..2)
                .map(|_| {
                    let app = app.clone();
                    let id = issue.id.clone();
                    scope.spawn(move || confirm_step(&app, &id))
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let ok_count = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(ok_count, 1, "exactly one confirm wins");
        assert!(results
            .iter()
            .any(|r| matches!(r, Err(e) if e.contains("no parked step"))));
        let launches = step_events(&mut rx)
            .into_iter()
            .filter(|e| matches!(e, InternalEvent::StepLaunch { .. }))
            .count();
        assert_eq!(launches, 1, "exactly one launch");

        // Stale-column backstop: park again, move the card away at the
        // CORE level (bypassing the command layer), confirm → typed error
        // + cleared event, no launch.
        enter_column_from_command(&app, &issue.id, &plan.id, None).unwrap();
        let ready = column(&app, "Ready");
        app.issues.enter_column(&issue.id, &ready.id, None).unwrap();
        let _ = step_events(&mut rx);
        let err = confirm_step(&app, &issue.id).unwrap_err();
        assert!(err.contains("no parked step"));
        assert!(matches!(
            &step_events(&mut rx)[..],
            [InternalEvent::StepQueueCleared { column_id, .. }] if column_id == &plan.id
        ));
    }

    /// Item 5: manual movement supersedes a pending confirm — but a drag
    /// that stays in the parked column keeps it.
    #[test]
    fn manual_moves_clear_the_parked_step() {
        let app = fixture();
        let mut rx = app.event_bus.subscribe();
        let issue = new_issue(&app, "card");
        let col_store = ColumnStore::new(app.db.clone());
        // Reconfigure the seeded In Progress column to queue-mode so the
        // park sits on a SEEDED column (lane-addressable by issue_move).
        let in_progress = column(&app, "In Progress");
        col_store
            .update(
                &in_progress.id,
                ColumnPatch {
                    on_enter: Some(OnEnter::Queue),
                    ..Default::default()
                },
            )
            .unwrap();

        enter_column_from_command(&app, &issue.id, &in_progress.id, None).unwrap();
        assert!(app.steps.peek_park(&issue.id).is_some());
        let _ = step_events(&mut rx);

        // Lane drag to the SAME column: the park survives.
        on_lane_move(&app, &issue.id, Lane::InProgress);
        assert!(app.steps.peek_park(&issue.id).is_some());
        assert!(step_events(&mut rx).is_empty());

        // Lane drag elsewhere: park dropped + StepQueueCleared.
        on_lane_move(&app, &issue.id, Lane::Backlog);
        assert!(app.steps.peek_park(&issue.id).is_none());
        assert!(matches!(
            &step_events(&mut rx)[..],
            [InternalEvent::StepQueueCleared { column_id, .. }] if column_id == &in_progress.id
        ));
        let err = confirm_step(&app, &issue.id).unwrap_err();
        assert!(err.contains("no parked step"));

        // Engine entry into a different column also supersedes the park.
        enter_column_from_command(&app, &issue.id, &in_progress.id, None).unwrap();
        let ready = column(&app, "Ready");
        enter_column_from_command(&app, &issue.id, &ready.id, None).unwrap();
        assert!(app.steps.peek_park(&issue.id).is_none());
        let events = step_events(&mut rx);
        assert!(events
            .iter()
            .any(|e| matches!(e, InternalEvent::StepQueueCleared { .. })));
    }

    /// Fix round, finding 13 (park half): a same-lane reorder never
    /// drops a park held on a NON-seeded column — the card isn't moving.
    #[test]
    fn same_lane_reorder_keeps_the_park() {
        let app = fixture();
        let issue = new_issue(&app, "card"); // lane backlog
        let col_store = ColumnStore::new(app.db.clone());
        let quick = column(&app, "Quick");
        col_store
            .update(
                &quick.id,
                ColumnPatch {
                    on_enter: Some(OnEnter::Queue),
                    ..Default::default()
                },
            )
            .unwrap();
        enter_column_from_command(&app, &issue.id, &quick.id, None).unwrap();
        assert!(app.steps.peek_park(&issue.id).is_some());

        // Card renders in its stale backlog lane; reordering there is a
        // same-lane move → the park (and the residence) survive.
        on_lane_move(&app, &issue.id, Lane::Backlog);
        assert!(app.steps.peek_park(&issue.id).is_some());

        // A real cross-lane drag still supersedes it.
        on_lane_move(&app, &issue.id, Lane::Ready);
        assert!(app.steps.peek_park(&issue.id).is_none());
    }

    /// Fail-closed ordering (#66 fix round, defect 1): `issue_move` to a
    /// lane whose seeded column was deleted (legal since the flip) is a
    /// typed refusal — and a refused move must be side-effect-free: the
    /// park survives (step_confirm still fires), no `StepQueueCleared`,
    /// and the entry epoch is untouched (the consumed-session set and
    /// the bound launch entry are still there — a settle bookkeeping
    /// wipe for a move that never happened was the corruption).
    #[test]
    fn refused_issue_move_leaves_park_epoch_and_registry_untouched() {
        use tauri::Manager as _;

        let app = fixture();
        let col_store = ColumnStore::new(app.db.clone());
        // Delete seeded Ready (empty shelf, not landing, no advance
        // target) so lane "ready" no longer resolves.
        col_store.delete(&column(&app, "Ready").id).unwrap();

        let issue = new_issue(&app, "card");
        let quick = column(&app, "Quick");
        col_store
            .update(
                &quick.id,
                ColumnPatch {
                    on_enter: Some(OnEnter::Queue),
                    ..Default::default()
                },
            )
            .unwrap();
        enter_column_from_command(&app, &issue.id, &quick.id, None).unwrap();
        assert!(app.steps.peek_park(&issue.id).is_some());
        // Seed epoch-scoped state an epoch bump would wipe: a consumed
        // session and a bound, settled launch entry.
        {
            let mut st = app.steps.lock();
            st.consumed
                .entry(issue.id.clone())
                .or_default()
                .insert("acp:sess".into());
            st.launches.insert(
                issue.id.clone(),
                LaunchEntry {
                    column_id: quick.id.clone(),
                    project_id: "p1".into(),
                    session: Some("acp:sess".into()),
                    settled: true,
                    delivered: true,
                },
            );
        }

        let mut rx = app.event_bus.subscribe();
        let tapp = tauri::test::mock_app();
        tapp.handle().manage(app.clone());
        let err = crate::commands::issues::issue_move(
            tapp.state::<Arc<App>>(),
            issue.id.clone(),
            "ready".into(),
            None,
        )
        .unwrap_err();
        assert!(
            err.contains("mirrors lane 'ready'"),
            "typed refusal expected, got: {err}"
        );

        // Side-effect-free: park intact, nothing announced, card unmoved.
        assert!(app.steps.peek_park(&issue.id).is_some());
        assert!(step_events(&mut rx).is_empty());
        assert_eq!(
            issue_column_id(&app, &issue.id).as_deref(),
            Some(quick.id.as_str())
        );
        // Epoch untouched: consumed set and bound launch entry survive.
        {
            let st = app.steps.lock();
            assert!(st.consumed[&issue.id].contains("acp:sess"));
            let entry = st.launches.get(&issue.id).expect("launch entry wiped");
            assert_eq!(entry.session.as_deref(), Some("acp:sess"));
            assert!(entry.settled);
        }
        // The confirm gate still works: confirm takes the park and
        // proceeds to launch — the repo-less fixture then fails at git
        // provisioning, but never with the "no parked step" refusal the
        // defect produced.
        let confirm_err = confirm_step(&app, &issue.id).unwrap_err();
        assert!(
            !confirm_err.contains("no parked step"),
            "the confirm gate was destroyed: {confirm_err}"
        );
    }

    /// Fix round, findings 1/5/9 — the verifier's scenario, permanent:
    /// Implement(run, advance) → Review(queue, advance→Done). Two settles
    /// from the SAME first session: the card must end PARKED in Review,
    /// never in Done — and no session ever bypasses a pending park.
    #[test]
    fn stale_settle_does_not_bypass_queue_confirm_gate() {
        let app = fixture();
        let issue = new_issue(&app, "card");
        with_task(&app, "t1");
        app.issues.set_linked_task(&issue.id, Some("t1")).unwrap();
        let done = column(&app, "Done");
        let review = make_step(
            &app,
            "Review",
            OnEnter::Queue,
            OnSettle::Advance,
            Some(done.id.clone()),
        );
        let implement = make_step(
            &app,
            "Implement",
            OnEnter::Run,
            OnSettle::Advance,
            Some(review.id.clone()),
        );
        enter_column_from_command(&app, &issue.id, &implement.id, None).unwrap();
        let mut rx = app.event_bus.subscribe();

        // Settle #1 (session S1): advances into Review, which PARKS.
        assert_eq!(settle_issues_for_task(&app, "t1", Some("acp:c1")), 1);
        assert_eq!(
            issue_column_id(&app, &issue.id).as_deref(),
            Some(review.id.as_str())
        );
        assert!(app.steps.peek_park(&issue.id).is_some());
        assert!(matches!(
            &step_events(&mut rx)[..],
            [InternalEvent::StepQueued { .. }]
        ));

        // Settle #2 from the SAME session (follow-up turn / PTY exit):
        // consumed → no-op. Card stays parked in Review.
        assert_eq!(settle_issues_for_task(&app, "t1", Some("acp:c1")), 0);
        // And even a DIFFERENT session cannot act on a parked issue.
        assert_eq!(settle_issues_for_task(&app, "t1", Some("pty:t9")), 0);
        assert_eq!(
            issue_column_id(&app, &issue.id).as_deref(),
            Some(review.id.as_str())
        );
        assert!(app.steps.peek_park(&issue.id).is_some(), "park intact");
        assert!(
            step_events(&mut rx).is_empty(),
            "no settle/launch/clear events"
        );

        // The gate still works: confirm fires Review's step.
        let outcome = confirm_step(&app, &issue.id).unwrap();
        assert_eq!(outcome.step, "launched");
        assert_eq!(
            issue_column_id(&app, &issue.id).as_deref(),
            Some(review.id.as_str())
        );
    }

    /// Fix round, findings 6/8/12: one session's duplicate/late triggers
    /// cannot march the card down an agent_step chain — only the NEXT
    /// step's own session advances it.
    #[test]
    fn stale_session_settle_does_not_march_the_chain() {
        let app = fixture();
        let issue = new_issue(&app, "card");
        with_task(&app, "t1");
        app.issues.set_linked_task(&issue.id, Some("t1")).unwrap();
        let c = make_step(&app, "C", OnEnter::Run, OnSettle::Hold, None);
        let b = make_step(
            &app,
            "B",
            OnEnter::Run,
            OnSettle::Advance,
            Some(c.id.clone()),
        );
        let a = make_step(
            &app,
            "A",
            OnEnter::Run,
            OnSettle::Advance,
            Some(b.id.clone()),
        );
        enter_column_from_command(&app, &issue.id, &a.id, None).unwrap();
        let mut rx = app.event_bus.subscribe();

        // A's session settles → advance into B (chained launch).
        assert_eq!(settle_issues_for_task(&app, "t1", Some("acp:cA")), 1);
        assert_eq!(
            issue_column_id(&app, &issue.id).as_deref(),
            Some(b.id.as_str())
        );
        assert!(matches!(
            &step_events(&mut rx)[..],
            [InternalEvent::StepLaunch { column_id, .. }] if column_id == &b.id
        ));

        // A's session again (late exit / follow-up turn): consumed →
        // no-op. The card does NOT march into C.
        assert_eq!(settle_issues_for_task(&app, "t1", Some("acp:cA")), 0);
        assert_eq!(
            issue_column_id(&app, &issue.id).as_deref(),
            Some(b.id.as_str())
        );
        assert!(step_events(&mut rx).is_empty());

        // B's OWN session settles → advance into C.
        assert_eq!(settle_issues_for_task(&app, "t1", Some("pty:termB")), 1);
        assert_eq!(
            issue_column_id(&app, &issue.id).as_deref(),
            Some(c.id.as_str())
        );
        assert_eq!(task_count(&app), 1, "whole chain lived in one task");
    }

    /// GOLDEN (E18-05 parity): the seeded In Progress column reproduces
    /// the E17-03 auto-flip exactly — In Progress → In Review on settle,
    /// cards dragged elsewhere stay put — including (registry-empty, as
    /// after a restart) dispatch-launched agents.
    #[test]
    fn settle_reproduces_the_in_progress_auto_flip() {
        let app = fixture();
        let a = new_issue(&app, "a");
        let b = new_issue(&app, "b");
        with_task(&app, "t1");
        app.issues.set_linked_task(&a.id, Some("t1")).unwrap();
        app.issues.set_linked_task(&b.id, Some("t1")).unwrap();
        // Dispatch-style entry: lane move, NO engine launch — the
        // registry has no entry, like after an app restart.
        app.issues.move_to(&a.id, Lane::InProgress, None).unwrap();
        app.issues.move_to(&b.id, Lane::Ready, None).unwrap();
        let mut rx = app.event_bus.subscribe();

        assert_eq!(settle_issues_for_task(&app, "t1", Some("pty:term1")), 1);
        let a_read = app.issues.get(&a.id).unwrap().unwrap();
        assert_eq!(a_read.lane, Lane::InReview);
        let in_review = column(&app, "In Review");
        assert_eq!(a_read.column_id.as_deref(), Some(in_review.id.as_str()));
        // The card dragged elsewhere stays put.
        assert_eq!(app.issues.get(&b.id).unwrap().unwrap().lane, Lane::Ready);
        // Human gate is inert: no launch/queue events, just the move.
        assert!(step_events(&mut rx).is_empty());
        // Duplicate trigger (other hook, same or other identity): the
        // card left In Progress, so nothing settles — E17 idempotence.
        assert_eq!(settle_issues_for_task(&app, "t1", Some("acp:conv1")), 0);
    }

    /// Quick → Done end-to-end: run on entry, advance_to Done on settle.
    #[test]
    fn settle_advances_quick_to_done_end_to_end() {
        let app = fixture();
        let issue = new_issue(&app, "small fix");
        with_task(&app, "t1");
        app.issues.set_linked_task(&issue.id, Some("t1")).unwrap();
        let mut rx = app.event_bus.subscribe();

        let quick = column(&app, "Quick");
        let outcome = enter_column_from_command(&app, &issue.id, &quick.id, None).unwrap();
        assert_eq!(outcome.step, "launched");

        assert_eq!(settle_issues_for_task(&app, "t1", Some("pty:quick1")), 1);
        let issue = app.issues.get(&issue.id).unwrap().unwrap();
        let done = column(&app, "Done");
        assert_eq!(issue.column_id.as_deref(), Some(done.id.as_str()));
        assert_eq!(issue.lane, Lane::Done); // Done is seeded → lane syncs
                                            // One launch (the Quick entry); Done is a shelf → inert on enter.
        let events = step_events(&mut rx);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], InternalEvent::StepLaunch { .. }));
    }

    /// Hold emits the derived step-settled event; advance chains into a
    /// run-mode step and fires it (new session, same task). Each settle
    /// comes from ITS OWN step's session.
    #[test]
    fn settle_hold_emits_and_advance_chains_fire_the_next_step() {
        let app = fixture();
        let issue = new_issue(&app, "card");
        with_task(&app, "t1");
        app.issues.set_linked_task(&issue.id, Some("t1")).unwrap();
        let hold_col = make_step(&app, "Verify", OnEnter::Run, OnSettle::Hold, None);
        let run_col = make_step(
            &app,
            "Implement",
            OnEnter::Run,
            OnSettle::Advance,
            Some(hold_col.id.clone()),
        );
        let mut rx = app.event_bus.subscribe();

        // Enter the advancing step: launch #1.
        enter_column_from_command(&app, &issue.id, &run_col.id, None).unwrap();
        // Implement's session settles: advances into the run-mode hold
        // column → launch #2 (chained, new session in the same task).
        assert_eq!(settle_issues_for_task(&app, "t1", Some("acp:impl")), 1);
        let issue_now = app.issues.get(&issue.id).unwrap().unwrap();
        assert_eq!(issue_now.column_id.as_deref(), Some(hold_col.id.as_str()));
        let events = step_events(&mut rx);
        assert_eq!(events.len(), 2);
        assert!(
            matches!(&events[0], InternalEvent::StepLaunch { column_id, .. } if column_id == &run_col.id)
        );
        assert!(
            matches!(&events[1], InternalEvent::StepLaunch { column_id, .. } if column_id == &hold_col.id)
        );

        // Verify's OWN session settles: hold → StepSettled, card stays.
        // (The stale acp:impl session would no-op — see the chain test.)
        assert_eq!(settle_issues_for_task(&app, "t1", Some("acp:verify")), 1);
        let events = step_events(&mut rx);
        assert!(matches!(
            &events[..],
            [InternalEvent::StepSettled { column_id, task_id, .. }]
                if column_id == &hold_col.id && task_id == "t1"
        ));
        assert_eq!(
            issue_column_id(&app, &issue.id).as_deref(),
            Some(hold_col.id.as_str())
        );
        assert_eq!(task_count(&app), 1, "whole chain lived in one task");
    }

    /// Fix round, finding 10: a settle-chained launch nobody opened is a
    /// DEFERRED directive — re-entering the column re-emits it (full
    /// prompt, "launched"), and only after command delivery does re-entry
    /// reattach.
    #[test]
    fn reentry_reemits_an_undelivered_chained_launch() {
        let app = fixture();
        let issue = new_issue(&app, "card");
        with_task(&app, "t1");
        app.issues.set_linked_task(&issue.id, Some("t1")).unwrap();
        let b = make_step(&app, "B", OnEnter::Run, OnSettle::Hold, None);
        let a = make_step(
            &app,
            "A",
            OnEnter::Run,
            OnSettle::Advance,
            Some(b.id.clone()),
        );
        enter_column_from_command(&app, &issue.id, &a.id, None).unwrap();
        // Chain into B: the launch directive has no listener yet.
        assert_eq!(settle_issues_for_task(&app, "t1", Some("acp:cA")), 1);
        assert_eq!(
            issue_column_id(&app, &issue.id).as_deref(),
            Some(b.id.as_str())
        );
        let mut rx = app.event_bus.subscribe();

        // Re-entry into B re-emits the launch instead of "reattaching"
        // with an empty prompt.
        let outcome = enter_column_from_command(&app, &issue.id, &b.id, None).unwrap();
        assert_eq!(outcome.step, "launched");
        let launch = outcome.launch.unwrap();
        assert!(!launch.reattached);
        assert!(!launch.prompt.is_empty());
        assert!(matches!(
            &step_events(&mut rx)[..],
            [InternalEvent::StepLaunch { column_id, reattached: false, .. }] if column_id == &b.id
        ));

        // Now delivered: the next re-entry is a true reattach.
        let outcome = enter_column_from_command(&app, &issue.id, &b.id, None).unwrap();
        assert_eq!(outcome.step, "reattached");
    }

    /// Fix round, finding 14: a queue→run column edit drops the stale
    /// park (announced) and a same-column re-entry launches fresh — it
    /// must never silently destroy the park and "reattach" to a session
    /// that never started. Covers both the command-hook path and the
    /// in-engine had-park path.
    #[test]
    fn queue_to_run_edit_then_reentry_launches_fresh() {
        let app = fixture();
        let issue = new_issue(&app, "card");
        with_task(&app, "t1");
        app.issues.set_linked_task(&issue.id, Some("t1")).unwrap();
        let col_store = ColumnStore::new(app.db.clone());
        let plan = make_step(&app, "Plan", OnEnter::Queue, OnSettle::Hold, None);
        enter_column_from_command(&app, &issue.id, &plan.id, None).unwrap();
        let mut rx = app.event_bus.subscribe();

        // Command-hook path: the edit drops the park with the cleared
        // event and leaves an undelivered entry…
        col_store
            .update(
                &plan.id,
                ColumnPatch {
                    on_enter: Some(OnEnter::Run),
                    ..Default::default()
                },
            )
            .unwrap();
        on_column_lost_queue(&app, &plan.id, true);
        assert!(app.steps.peek_park(&issue.id).is_none());
        assert!(matches!(
            &step_events(&mut rx)[..],
            [InternalEvent::StepQueueCleared { column_id, .. }] if column_id == &plan.id
        ));
        // …so the same-column re-entry LAUNCHES with the full prompt.
        let outcome = enter_column_from_command(&app, &issue.id, &plan.id, None).unwrap();
        assert_eq!(outcome.step, "launched");
        assert!(!outcome.launch.unwrap().prompt.is_empty());

        // In-engine path (no hook ran): park exists when the re-entry
        // arrives — the taken park forces a launch, never a reattach.
        col_store
            .update(
                &plan.id,
                ColumnPatch {
                    on_enter: Some(OnEnter::Queue),
                    ..Default::default()
                },
            )
            .unwrap();
        enter_column_from_command(&app, &issue.id, &plan.id, None).unwrap(); // re-park
        col_store
            .update(
                &plan.id,
                ColumnPatch {
                    on_enter: Some(OnEnter::Run),
                    ..Default::default()
                },
            )
            .unwrap();
        let outcome = enter_column_from_command(&app, &issue.id, &plan.id, None).unwrap();
        assert_eq!(outcome.step, "launched");
        assert!(!outcome.launch.unwrap().prompt.is_empty());
        assert!(app.steps.peek_park(&issue.id).is_none());
    }

    /// Final round item 2 — permanent restart-contract test: a card in a
    /// queue+advance column with NO park and NO registry entry (exactly
    /// the post-restart state) must not advance on settle; the park is
    /// RESTORED (StepQueued) so the confirm gate visibly re-arms.
    #[test]
    fn restart_settle_reparks_queue_columns_instead_of_advancing() {
        let app = fixture();
        let issue = new_issue(&app, "card");
        with_task(&app, "t1");
        app.issues.set_linked_task(&issue.id, Some("t1")).unwrap();
        let done = column(&app, "Done");
        let review = make_step(
            &app,
            "Review",
            OnEnter::Queue,
            OnSettle::Advance,
            Some(done.id.clone()),
        );
        // Restart-equivalent state: the card sits IN the queue column but
        // the engine has no park and no registry entry (core-level enter
        // bypasses the engine, like a pre-restart entry would).
        app.issues
            .enter_column(&issue.id, &review.id, None)
            .unwrap();
        assert!(!app.steps.has_state(&issue.id));
        let mut rx = app.event_bus.subscribe();

        // Settle must NOT advance — it re-parks and announces the gate.
        assert_eq!(settle_issues_for_task(&app, "t1", Some("pty:old")), 0);
        assert_eq!(
            issue_column_id(&app, &issue.id).as_deref(),
            Some(review.id.as_str()),
            "card must not leave the queue column"
        );
        assert!(app.steps.peek_park(&issue.id).is_some(), "park restored");
        assert!(matches!(
            &step_events(&mut rx)[..],
            [InternalEvent::StepQueued { column_id, .. }] if column_id == &review.id
        ));
        // Further settles: the park guard now covers the card.
        assert_eq!(settle_issues_for_task(&app, "t1", Some("pty:old")), 0);
        assert_eq!(settle_issues_for_task(&app, "t1", Some("acp:other")), 0);
        // The restored gate works: confirm launches Review's step.
        let outcome = confirm_step(&app, &issue.id).unwrap();
        assert_eq!(outcome.step, "launched");
    }

    /// Final round item 3 — the E17 rework loop, permanent: dispatch,
    /// settle-flip, re-drag back to In Progress (legacy lane path, same
    /// conversation reattached per ADR-0032), and the SAME session's next
    /// settle flips again — a new entry is a new settle epoch.
    #[test]
    fn rework_loop_settles_again_after_reentry() {
        let app = fixture();
        let issue = new_issue(&app, "card");
        with_task(&app, "t1");
        app.issues.set_linked_task(&issue.id, Some("t1")).unwrap();
        // Dispatch-style entry (lane move, no engine launch).
        app.issues
            .move_to(&issue.id, Lane::InProgress, None)
            .unwrap();

        // First settle from the task's conversation: auto-flip.
        assert_eq!(settle_issues_for_task(&app, "t1", Some("acp:conv")), 1);
        assert_eq!(
            app.issues.get(&issue.id).unwrap().unwrap().lane,
            Lane::InReview
        );
        // Duplicate trigger inside the same epoch: consumed → no-op.
        assert_eq!(settle_issues_for_task(&app, "t1", Some("acp:conv")), 0);

        // Rework: re-drag back to In Progress exactly as the issue_move
        // command does (epoch reset + lane move). The dispatch reattaches
        // the SAME conversation — no new session identity exists.
        on_lane_move(&app, &issue.id, Lane::InProgress);
        app.issues
            .move_to(&issue.id, Lane::InProgress, None)
            .unwrap();

        // The same session settles again → flips again.
        assert_eq!(settle_issues_for_task(&app, "t1", Some("acp:conv")), 1);
        assert_eq!(
            app.issues.get(&issue.id).unwrap().unwrap().lane,
            Lane::InReview
        );
    }

    /// Final round item 4b: an errored enter (unknown column) must leave
    /// a pending park untouched — no silent destruction.
    #[test]
    fn errored_enter_leaves_the_park_untouched() {
        let app = fixture();
        let issue = new_issue(&app, "card");
        let plan = make_step(&app, "Plan", OnEnter::Queue, OnSettle::Hold, None);
        enter_column_from_command(&app, &issue.id, &plan.id, None).unwrap();
        assert!(app.steps.peek_park(&issue.id).is_some());
        let mut rx = app.event_bus.subscribe();

        let err = enter_column_from_command(&app, &issue.id, "col_nope", None).unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
        assert!(
            app.steps.peek_park(&issue.id).is_some(),
            "park survives the failed move"
        );
        assert!(step_events(&mut rx).is_empty(), "no spurious cleared event");
        // The park is still confirmable.
        with_task(&app, "t1");
        app.issues.set_linked_task(&issue.id, Some("t1")).unwrap();
        assert!(confirm_step(&app, &issue.id).is_ok());
    }

    /// Fix round, finding 4 (+ final round 4a): issue and project
    /// deletion sweep parks (with the cleared event) and every registry
    /// trace — including launched-but-unparked issues.
    #[test]
    fn deletion_sweeps_parks_and_registry() {
        let app = fixture();
        let issue = new_issue(&app, "card");
        let plan = make_step(&app, "Plan", OnEnter::Queue, OnSettle::Hold, None);
        enter_column_from_command(&app, &issue.id, &plan.id, None).unwrap();
        let mut rx = app.event_bus.subscribe();

        // Issue deletion (command order: store delete, then sweep).
        app.issues.delete(&issue.id).unwrap();
        on_issue_deleted(&app, &issue.id);
        assert!(!app.steps.has_state(&issue.id), "every trace swept");
        assert!(matches!(
            &step_events(&mut rx)[..],
            [InternalEvent::StepQueueCleared { issue_id, .. }] if issue_id == &issue.id
        ));
        let err = confirm_step(&app, &issue.id).unwrap_err();
        assert!(err.contains("no parked step"));

        // Project deletion: the FK cascade emits no per-issue events, so
        // the sweep is by project — covering BOTH a parked card and a
        // launched-but-unparked one (final round 4a: its registry entry
        // and consumed set must go too).
        let parked_card = new_issue(&app, "card2");
        enter_column_from_command(&app, &parked_card.id, &plan.id, None).unwrap();
        let launched_card = new_issue(&app, "card3");
        with_task(&app, "t9");
        app.issues
            .set_linked_task(&launched_card.id, Some("t9"))
            .unwrap();
        let quick = column(&app, "Quick");
        enter_column_from_command(&app, &launched_card.id, &quick.id, None).unwrap();
        settle_issues_for_task(&app, "t9", Some("pty:x")); // populate consumed
        assert!(app.steps.has_state(&launched_card.id));
        let _ = step_events(&mut rx);
        app.db
            .conn()
            .lock()
            .unwrap()
            .execute("DELETE FROM projects WHERE id = 'p1'", [])
            .unwrap();
        on_project_deleted(&app, "p1");
        assert!(!app.steps.has_state(&parked_card.id));
        assert!(
            !app.steps.has_state(&launched_card.id),
            "launched-unparked swept"
        );
        assert!(matches!(
            &step_events(&mut rx)[..],
            [InternalEvent::StepQueueCleared { issue_id, .. }] if issue_id == &parked_card.id
        ));
    }
}
