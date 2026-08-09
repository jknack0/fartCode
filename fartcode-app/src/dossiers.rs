//! Feature-dossier lifecycle (E19-01, #70; ADR-0038 items 1–2).
//!
//! The content and file primitives live in `fartcode_core::dossiers`. This
//! module is the half that needs the wired App: the consent gate, worktree
//! resolution, `issues.dossier_path`, and the event subscriber that appends
//! machine breadcrumbs to `## Timeline`.
//!
//! **Three invariants, all load-bearing.**
//!
//! 1. *No consent, no write — on EVERY path.* [`consented`] gates both
//!    creation and every append. Consent is not a one-time admission
//!    ticket: turning the switch off must stop the breadcrumbs too, not
//!    just future dossiers.
//! 2. *Creation never fails a dispatch.* [`create_for_task`] returns an
//!    `Option` and swallows every error into a `tracing::warn!`. A
//!    read-only repo, a declined project, a vanished worktree — all leave
//!    `dossier_path` NULL and the agent running. The feature is memory,
//!    not a gate (ADR-0038 item 3: declining still dispatches).
//! 3. *Appending never blocks the emitting path.* The subscriber owns its
//!    own task and does its filesystem work inside `spawn_blocking`, per
//!    AGENTS.md "Tauri commands and the main thread" — the bus sender never
//!    waits on a disk write, and a failed append logs instead of
//!    propagating.

use std::path::PathBuf;
use std::sync::Arc;

use rusqlite::OptionalExtension;

use fartcode_core::dossiers;
use fartcode_core::events::{EventBus, InternalEvent};
use fartcode_core::issues::columns::ColumnKind;
use fartcode_core::issues::{Issue, IssuePatch};
use fartcode_core::projects::ProjectStore;
use fartcode_core::tasks::TaskStore;

use crate::app::App;

/// Whether this project consented to dossier writes (ADR-0038 item 3).
///
/// **Fail closed.** `None` — never asked, and the state of every project
/// until #74's consent card ships — resolves to **false**, as does an
/// unreadable settings row. Writing files into someone's repository is a
/// side effect on their property; the dispatch prompt tells the agent to
/// commit as it goes, so an unrequested dossier does not sit quietly in a
/// worktree, it lands in their pull request. The only defensible default
/// for "we have not asked" is the same as for "we could not tell": don't.
///
/// The cost, accepted deliberately: the feature is inert until #74 lands.
/// The reasoning lives with the setting itself —
/// [`fartcode_core::settings::BaseProjectSettings::feature_dossiers`].
///
/// Crate-visible since E19-02 (#71): the seeded feature-log skill and the
/// step-prompt append instruction (`crate::skills`) write into — and talk
/// about — the same repository, so they must ask the same question. One
/// gate, not three that can drift apart.
pub(crate) fn consented(app: &App, project_id: &str) -> bool {
    let Ok(Some(project)) = app.projects.get(project_id) else {
        return false;
    };
    match app
        .settings
        .get_project_settings(project_id, std::path::Path::new(&project.path))
    {
        Ok(settings) => settings.feature_dossiers.unwrap_or(false),
        Err(e) => {
            tracing::warn!(project_id, error = %e, "dossier consent unreadable — not writing");
            false
        }
    }
}

/// The materialized worktree root of a task, or `None` when it has no
/// workspace row / no path / the directory is gone.
///
/// Crate-visible for the two consumers that must agree with the appender
/// about where a feature's files live: the skill scaffold (E19-02, #71),
/// seeded into the same worktree, and `crate::dossier_index` (E19-03),
/// which reindexes the same worktree copy of the same dossier. One
/// resolver, so writer and indexer can never disagree.
pub(crate) fn task_worktree(app: &App, task_id: &str) -> Option<PathBuf> {
    let conn = app.db.conn().lock().ok()?;
    let path: Option<String> = conn
        .query_row(
            "SELECT w.path FROM tasks t
               JOIN workspaces w ON w.id = t.workspace_id
              WHERE t.id = ?1",
            [task_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .ok()?
        .flatten();
    drop(conn);
    let path = PathBuf::from(path?);
    if path.as_os_str().is_empty() || !path.is_dir() {
        return None;
    }
    Some(path)
}

/// Births the feature dossier alongside the freshly provisioned worktree
/// (ADR-0038 item 1) and records its repo-relative path on the card.
///
/// Called from `dispatch::provision_issue_task` — the ONE provisioning
/// helper both the legacy board dispatch and the step engine's first
/// `agent_step` entry share — AFTER the worktree exists, so the file is
/// born on the feature branch and travels with it. Because provisioning
/// only happens when the card has no live linked task, a second step
/// column cannot mint a second dossier: later entries reuse the same
/// task/worktree and the `dossier_path` already on the row.
///
/// Returns the UPDATED issue when the dossier is in place (freshly written
/// or adopted), `None` in every refusal or failure case.
///
/// Returning the issue rather than the path is deliberate: it leaves the
/// caller with nothing fallible to do afterwards. An earlier version made
/// `provision_issue_task` re-read the row and propagate the read's error,
/// which turned a dossier bookkeeping failure into a failed dispatch —
/// exactly the thing this feature must never do.
pub fn create_for_task(app: &App, issue: &Issue, task_id: &str) -> Option<Issue> {
    if !consented(app, &issue.project_id) {
        tracing::debug!(issue = %issue.id, "feature dossiers off for this project — not writing");
        return None;
    }
    let worktree = task_worktree(app, task_id)?;

    // Only an `agent_step` column is named in the birth line. The legacy
    // `issue_dispatch` path provisions BEFORE it moves the card, so the
    // card is still sitting on a shelf here — naming that shelf would
    // claim a step entry that has not happened. The engine path (the
    // board's real one) has already entered the step column, so it names
    // it correctly.
    let column_name = issue
        .column_id
        .as_deref()
        .and_then(|id| app.columns.get(id).ok().flatten())
        .filter(|c| c.kind == ColumnKind::AgentStep)
        .map(|c| c.name);
    let header = dossiers::backfilled_header(issue, column_name.as_deref());

    // The core picks the final path: it adopts only this card's own
    // dossier and steps around anything else living at the slug — a
    // hand-written `docs/features/<slug>.md` is someone's document, not a
    // vacancy.
    let placed =
        match dossiers::place_dossier(&worktree, issue, &header, issue.dossier_path.as_deref()) {
            Ok(placed) => placed,
            Err(e) => {
                // ADR-0038: memory, not a gate. Log and leave the path NULL.
                tracing::warn!(issue = %issue.id, error = %e, "feature dossier write failed");
                return None;
            }
        };
    if placed.created {
        tracing::info!(issue = %issue.id, path = %placed.rel_path, "feature dossier created");
    } else {
        tracing::info!(issue = %issue.id, path = %placed.rel_path, "feature dossier adopted");
    }

    if issue.dossier_path.as_deref() == Some(placed.rel_path.as_str()) {
        return Some(issue.clone());
    }
    match app.issues.update(
        &issue.id,
        IssuePatch {
            dossier_path: Some(Some(placed.rel_path.clone())),
            ..Default::default()
        },
    ) {
        Ok(updated) => Some(updated),
        Err(e) => {
            tracing::warn!(issue = %issue.id, error = %e, "recording dossier_path failed");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Timeline appender
// ---------------------------------------------------------------------------

/// Subscriber that appends machine breadcrumbs under `## Timeline`
/// (ADR-0038 item 2) from events the app ALREADY emits.
///
/// Wired facts: step launched (column · provider · model), step settled,
/// column moves, PR opened/merged. Creation and the pre-worktree history
/// are backfilled into the header at birth ([`create_for_task`]) rather
/// than appended, because they predate the file.
///
/// Writes only while a worktree exists AND consent stands: every append
/// re-checks [`consented`] and resolves the card's linked task → workspace
/// path → dossier file. A withdrawn consent, a missing link, a pruned
/// worktree, or a deleted file all mean "record nothing" (ADR-0038 item 2:
/// post-teardown events go unrecorded).
pub struct TimelineAppender {
    app: Arc<App>,
}

impl TimelineAppender {
    pub fn new(app: Arc<App>) -> Self {
        Self { app }
    }

    /// Resolves a card to `(worktree root, repo-relative dossier path)`, or
    /// `None` when there is nothing we may write — consent withdrawn, no
    /// dossier, no linked task, no worktree on disk, or the file itself is
    /// gone with the branch.
    ///
    /// The consent check lives HERE rather than at the four event arms so
    /// there is exactly one place to forget it. An existing dossier is not
    /// standing permission: flipping the project switch off must stop the
    /// breadcrumbs too, or "off" would only mean "no NEW files".
    fn target(&self, issue_id: &str) -> Option<(PathBuf, String)> {
        let issue = self.app.issues.get(issue_id).ok().flatten()?;
        self.target_for(&issue)
    }

    /// [`Self::target`] for a card already in hand — lets the PR fan-out
    /// resolve many cards without re-reading each row.
    fn target_for(&self, issue: &Issue) -> Option<(PathBuf, String)> {
        if !consented(&self.app, &issue.project_id) {
            return None;
        }
        let rel = issue.dossier_path.clone()?;
        let task_id = issue.linked_task_id.as_deref()?;
        // A deleted task clears the link (ON DELETE SET NULL), but check
        // anyway: the row may be mid-teardown.
        self.app.tasks.get(task_id).ok().flatten()?;
        let worktree = task_worktree(&self.app, task_id)?;
        if !worktree.join(&rel).is_file() {
            return None;
        }
        Some((worktree, rel))
    }

    fn append(&self, issue_id: &str, fact: &str, once_key: Option<&str>) {
        let Some((worktree, rel)) = self.target(issue_id) else {
            return;
        };
        let line = dossiers::timeline_line(fact);
        if let Err(e) = dossiers::append_timeline(&worktree, &rel, &line, once_key) {
            tracing::warn!(issue = issue_id, path = %rel, error = %e, "dossier timeline append failed");
        }
    }

    fn column_name(&self, column_id: &str) -> String {
        self.app
            .columns
            .get(column_id)
            .ok()
            .flatten()
            .map(|c| c.name)
            .unwrap_or_else(|| column_id.to_string())
    }

    /// Handles one event. Synchronous and self-contained so tests can drive
    /// it without the bus; the spawned loop calls it inside `spawn_blocking`.
    pub fn handle(&self, event: &InternalEvent) {
        match event {
            // A reattach is a focus, not a launch — nothing new happened.
            InternalEvent::StepLaunch {
                issue_id,
                column_id,
                provider,
                model,
                reattached: false,
                ..
            } => {
                let mut fact = format!("{} · launched · {provider}", self.column_name(column_id));
                if let Some(model) = model.as_deref().filter(|m| !m.is_empty()) {
                    fact.push_str(" · ");
                    fact.push_str(model);
                }
                self.append(issue_id, &fact, None);
            }
            InternalEvent::StepSettled {
                issue_id,
                column_id,
                ..
            } => {
                let fact = format!("{} · settled", self.column_name(column_id));
                self.append(issue_id, &fact, None);
            }
            // The move arrives with BOTH endpoints from the emitter, which
            // is the only place that knows them. (This used to diff the
            // card's current column against an in-memory map — read at
            // handler time, so rapid moves recorded the wrong "from", and
            // the map grew without bound.)
            InternalEvent::IssueColumnChanged { id, from, to, .. } => {
                let fact = match from {
                    Some(from) => format!(
                        "column · {} → {}",
                        self.column_name(from),
                        self.column_name(to)
                    ),
                    None => format!("column → {}", self.column_name(to)),
                };
                self.append(id, &fact, None);
            }
            InternalEvent::PrUpdated {
                workspace_id,
                pr_url,
            } => self.on_pr_updated(workspace_id, pr_url),
            _ => {}
        }
    }

    fn on_pr_updated(&self, workspace_id: &str, pr_url: &str) {
        // `PrUpdated` fires on every payload change (checks, comments), so
        // the interesting facts — opened, merged — are deduped by
        // `once_key` against the file itself. That keeps it stateless and
        // restart-safe: no in-memory "last status" to lose.
        let Ok(conn) = self.app.db.conn().lock() else {
            return;
        };
        let status: Option<String> = conn
            .query_row(
                "SELECT status FROM pull_requests WHERE url = ?1",
                [pr_url],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten();
        let issue_ids: Vec<String> = conn
            .prepare(
                "SELECT i.id FROM issues i
                   JOIN tasks t ON t.id = i.linked_task_id
                  WHERE t.workspace_id = ?1 AND i.dossier_path IS NOT NULL",
            )
            .and_then(|mut stmt| {
                stmt.query_map([workspace_id], |row| row.get::<_, String>(0))?
                    .collect()
            })
            .unwrap_or_default();
        drop(conn);

        let verb = match status.as_deref() {
            Some("open") => "pr opened",
            Some("merged") => "pr merged",
            // `closed` without a merge is not a lifecycle fact worth a line.
            _ => return,
        };
        let fact = format!("{verb} · {pr_url}");
        // One workspace is one project, so consent is read once for the
        // whole fan-out rather than once per card.
        let issues: Vec<Issue> = issue_ids
            .iter()
            .filter_map(|id| self.app.issues.get(id).ok().flatten())
            .collect();
        let Some(project_id) = issues.first().map(|i| i.project_id.clone()) else {
            return;
        };
        if !consented(&self.app, &project_id) {
            return;
        }
        let line = dossiers::timeline_line(&fact);
        for issue in issues {
            let Some((worktree, rel)) = self.target_for(&issue) else {
                continue;
            };
            if let Err(e) = dossiers::append_timeline(&worktree, &rel, &line, Some(&fact)) {
                tracing::warn!(issue = %issue.id, path = %rel, error = %e, "dossier PR breadcrumb failed");
            }
        }
    }
}

/// Boot wiring: subscribe for the app's lifetime.
///
/// The filesystem work runs on the blocking pool, never on the async
/// runtime's worker (AGENTS.md) — the event bus is a broadcast channel, so
/// a slow subscriber only lags itself, but a blocked runtime worker is
/// everyone's problem.
///
/// Stateless by construction since the fix round: the move event carries
/// its own endpoints, so there is no boot backfill to get wrong and no
/// restart window where a move goes unrecorded.
pub fn spawn_dossier_timeline(app: Arc<App>) {
    let appender = Arc::new(TimelineAppender::new(app.clone()));
    tauri::async_runtime::spawn(async move {
        let mut rx = app.event_bus.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let appender = appender.clone();
                    let _ =
                        tauri::async_runtime::spawn_blocking(move || appender.handle(&event)).await;
                }
                // A lagging subscriber drops events but must survive; only
                // a closed bus ends the loop.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
