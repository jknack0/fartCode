//! Configurable pipeline columns (E18-01/E18-02, ADR-0037).
//!
//! Per-project board columns as data: each column carries a kind
//! (`shelf` | `agent_step` | `human_gate`), the `counts_as_done` and
//! `is_landing` flags, and — for agent steps — a step config (prompt,
//! provider, model, effort, tool allowlist) plus trigger (`on_enter`) and
//! settle (`on_settle`) behavior with an optional `advance_to` target
//! column (NULL = next column).
//!
//! **Authoritative since the E18-07 flip (#66):** `issues.column_id` owns
//! board placement — every write path maintains it and migration 0008
//! backfilled every row. `issues.lane` is a derived display mirror of a
//! seeded column's `seed_lane`; nothing here (or anywhere) keys behavior
//! off it.
//!
//! Invariants enforced here:
//! - **Exactly one `is_landing` column per project.** Creating/updating a
//!   column with the flag *moves* it off the previous holder in the same
//!   transaction; clearing it directly (or deleting the landing column) is
//!   rejected — point the flag somewhere else first.
//! - **Positions are compact** (0..n-1 in board order) after every delete
//!   and reorder; create appends at the end.
//! - **Deleting an occupied column fails** with
//!   [`Error::BoardColumnHasIssues`]. Occupancy is strictly by
//!   `column_id` — the authoritative pointer (E18-07).
//! - **`advance_to` must target a column in the same project** and never
//!   the column itself. Deleting a column that is another column's
//!   `advance_to` target is REFUSED with
//!   [`Error::BoardColumnIsAdvanceTarget`] — repoint the referrer first
//!   (E18-07; letting the FK null the pointer would silently re-route the
//!   advance to next-by-position, the ADR-0037 item 4 spend hazard).
//!
//! Schema: migration 0006 (which also seeds existing projects and
//! backfills `issues.column_id`). New projects are seeded by
//! [`seed_default_columns`] inside the project-create transaction.

use std::sync::Arc;

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::db::{parse_versioned, serialize_versioned, Db, Versioned};
use crate::Error;

/// Column kinds (ADR-0037 item 1). Text values are the stored
/// representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnKind {
    Shelf,
    AgentStep,
    HumanGate,
    /// The merge verb (pipeline overhaul): entering runs squash-merge +
    /// push + worktree cleanup FRONTEND-side; backend-side a ship column
    /// behaves exactly like a shelf (no step, no gate) so enter_column
    /// stays kind-agnostic and old boards are unaffected.
    Ship,
}

impl ColumnKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ColumnKind::Shelf => "shelf",
            ColumnKind::AgentStep => "agent_step",
            ColumnKind::HumanGate => "human_gate",
            ColumnKind::Ship => "ship",
        }
    }

    pub fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "shelf" => Ok(ColumnKind::Shelf),
            "agent_step" => Ok(ColumnKind::AgentStep),
            "human_gate" => Ok(ColumnKind::HumanGate),
            "ship" => Ok(ColumnKind::Ship),
            other => Err(Error::InvalidBoardColumnInput(format!(
                "invalid column kind: {other:?} (expected shelf|agent_step|human_gate|ship)"
            ))),
        }
    }
}

/// Step trigger (ADR-0037 item 3): `run` fires on drop, `queue` shows the
/// dispatch-style confirm overlay first. Meaningful for `agent_step` only;
/// stored (with the `queue` default) on every row for simplicity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnEnter {
    Run,
    Queue,
}

impl OnEnter {
    pub fn as_str(&self) -> &'static str {
        match self {
            OnEnter::Run => "run",
            OnEnter::Queue => "queue",
        }
    }

    pub fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "run" => Ok(OnEnter::Run),
            "queue" => Ok(OnEnter::Queue),
            other => Err(Error::InvalidBoardColumnInput(format!(
                "invalid on_enter: {other:?} (expected run|queue)"
            ))),
        }
    }
}

/// Settle behavior (ADR-0037 item 4, default hold): `hold` leaves the card
/// for a human drag, `advance` moves it to the next column — or to the
/// column's `advance_to` target when one is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnSettle {
    Hold,
    Advance,
}

impl OnSettle {
    pub fn as_str(&self) -> &'static str {
        match self {
            OnSettle::Hold => "hold",
            OnSettle::Advance => "advance",
        }
    }

    pub fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "hold" => Ok(OnSettle::Hold),
            "advance" => Ok(OnSettle::Advance),
            other => Err(Error::InvalidBoardColumnInput(format!(
                "invalid on_settle: {other:?} (expected hold|advance)"
            ))),
        }
    }
}

/// Versioned payload for the `board_columns.step_tools` column
/// (`{"version":1,"data":{"tools":[...]}}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepTools {
    #[serde(default)]
    pub tools: Vec<String>,
}

impl Versioned for StepTools {
    const VERSION: u32 = 1;
}

/// One board column row.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardColumn {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub position: i64,
    pub kind: ColumnKind,
    pub counts_as_done: bool,
    pub is_landing: bool,
    pub on_enter: OnEnter,
    pub on_settle: OnSettle,
    /// Explicit `on_settle: advance` target (ADR-0037 item 4). `None` =
    /// the next column by position. Always a same-project column, never
    /// this column itself.
    pub advance_to: Option<String>,
    /// `None` on an agent step = the built-in dispatch packet
    /// (`build_dispatch_prompt`), matching today's In Progress behavior.
    pub step_prompt: Option<String>,
    pub step_provider: Option<String>,
    pub step_model: Option<String>,
    pub step_effort: Option<String>,
    /// Tool allowlist for the step's agent session. `None` = unrestricted;
    /// `Some(vec)` = only the listed tools (so `Some([])` allows none). A
    /// corrupt stored cell parses as `Some([])` — **fail closed**, never
    /// silently unrestricted (see `column_from_row`).
    pub step_tools: Option<Vec<String>>,
    /// The legacy lane this seeded column mirrors
    /// (`backlog|ready|in_progress|in_review|done`); `None` on Quick and
    /// user-created columns. Read-only: set by the seed paths, never by
    /// create/update. Since E18-07 it only drives the DERIVED display
    /// lane sync (enter_column's reverse mapping) and lane-addressed
    /// wire-compat routing.
    pub seed_lane: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Fields for [`ColumnStore::create`]. The new column is appended to the
/// end of the board.
pub struct NewColumn {
    pub project_id: String,
    pub name: String,
    pub kind: ColumnKind,
    pub counts_as_done: bool,
    /// `true` moves the landing flag onto the new column.
    pub is_landing: bool,
    /// Defaults to [`OnEnter::Queue`] (ADR-0037: expensive columns queue).
    pub on_enter: Option<OnEnter>,
    /// Defaults to [`OnSettle::Hold`] (ADR-0037: approval gates remain the
    /// doctrine).
    pub on_settle: Option<OnSettle>,
    /// Must reference an existing column in the same project.
    pub advance_to: Option<String>,
    pub step_prompt: Option<String>,
    pub step_provider: Option<String>,
    pub step_model: Option<String>,
    pub step_effort: Option<String>,
    /// `None` = unrestricted; `Some(vec)` = allowlist.
    pub step_tools: Option<Vec<String>>,
}

/// Patch for [`ColumnStore::update`]: `None` leaves the field alone;
/// `Some(None)` clears a nullable field; `Some(Some(v))` sets it.
/// `name` is non-nullable (`Some("")` rejected). `is_landing: Some(true)`
/// moves the flag; `Some(false)` on the landing column is rejected.
/// An untouched `step_tools` preserves the stored cell byte-for-byte
/// (it is never round-tripped through the lossy parse).
#[derive(Debug, Default)]
pub struct ColumnPatch {
    pub name: Option<String>,
    pub kind: Option<ColumnKind>,
    pub counts_as_done: Option<bool>,
    pub is_landing: Option<bool>,
    pub on_enter: Option<OnEnter>,
    pub on_settle: Option<OnSettle>,
    /// `Some(Some(id))` must reference a same-project column ≠ this one.
    pub advance_to: Option<Option<String>>,
    pub step_prompt: Option<Option<String>>,
    pub step_provider: Option<Option<String>>,
    pub step_model: Option<Option<String>>,
    pub step_effort: Option<Option<String>>,
    /// `Some(None)` clears the allowlist (back to unrestricted).
    pub step_tools: Option<Option<Vec<String>>>,
}

/// Seed row shape for [`seed_default_columns`] (also mirrored by the SQL
/// seed in migration 0006 for pre-existing projects).
struct SeedColumn {
    name: &'static str,
    kind: ColumnKind,
    counts_as_done: bool,
    is_landing: bool,
    on_enter: OnEnter,
    on_settle: OnSettle,
    /// Legacy lane this column mirrors (`issues.lane` value).
    seed_lane: Option<&'static str>,
    /// `advance_to` target, named by the target column's seed NAME
    /// (names are unique within SEED_COLUMNS; several targets carry no
    /// seed_lane, so lanes cannot address them).
    advance_to_name: Option<&'static str>,
    /// Step framing prepended to the dispatch packet by
    /// `compose_step_prompt` (`None` = the packet alone).
    step_prompt: Option<&'static str>,
    /// Step agent pin (`None` = project default). Encoded like the
    /// ADR-0032 per-issue overrides: provider registry id + bare Claude
    /// model alias.
    step_provider: Option<&'static str>,
    step_model: Option<&'static str>,
}

/// Grill: interactive interrogation of a raw idea — the terminal chat IS
/// the back-and-forth; the hardened result lands in the dossier for the
/// steps downstream.
const GRILL_PROMPT: &str = "You are running a grill session, not implementing anything. \
     Interrogate this idea until it is fully specified: ask ONE question at a time in the \
     conversation and wait for the answer before asking the next — hunt gaps, hidden \
     assumptions, edge cases, scope cuts, failure modes, and what 'done' means. Ask every \
     question as multiple choice via the ask_user_question tool: 2-4 options, each with a \
     short label and a description of what it means or costs; put the option YOU would \
     choose first and append '(Recommended)' to its label. Do not author an 'Other' \
     option — the tool appends one. If that tool is unavailable, ask the same question as \
     labelled options (A, B, C…) in plain text and name your pick. Push back on vague \
     answers. When the idea is hardened, write the result into the issue's dossier: \
     the sharpened problem statement, the decisions made, and a numbered \
     acceptance-criteria list precise enough to write failing tests from. Do not write \
     code and do not modify anything else in the repo.";

/// Plan: grilled issue → ordered TDD-executable steps. No code.
const PLAN_PROMPT: &str = "Turn the grilled issue into an implementation plan — write no \
     code. Read the dossier (the grill session's decisions and acceptance criteria) and \
     the files the work will touch. Produce, in the dossier: an ordered list of small \
     implementation steps, each naming the files it touches and the acceptance criteria \
     it satisfies; a test list with one named failing test per criterion; the risks, \
     riskiest step first. Steps must be small enough for a TDD implementer to execute one \
     at a time. If a criterion cannot be planned, say so loudly instead of guessing.";

/// Implement: strict TDD against the plan.
const IMPLEMENT_PROMPT: &str = "Work strictly test-driven. Follow the plan in the dossier \
     one step at a time: first write the failing test for the step's acceptance \
     criterion, run it and watch it fail, then write the minimal code that makes it pass, \
     then refactor. Never write implementation before its failing test exists. An \
     acceptance criterion without a covering test is not done. Finish with the full test \
     suite green, and record any deviations from the plan in the dossier.";

/// Adversarial: hostile review — find, never fix.
const ADVERSARIAL_PROMPT: &str = "You are a hostile reviewer; assume the diff is wrong \
     until proven otherwise. Do not fix anything — only find. Hunt: acceptance criteria \
     not actually met, tests that pass without testing their criterion, unhandled edge \
     cases, race conditions, security holes, silent failure paths, dead code, and lies in \
     comments or names. Verify every finding against the code before reporting it. Write \
     the findings into the dossier ranked by severity with file:line references; an empty \
     findings list must be earned by listing what you checked.";

/// The seeded default board (pipeline overhaul): Idea · Grill · Quick ·
/// Plan · Implement · Adversarial · Review · Ship.
///
/// Idea lands imports; Grill and Plan are confirm-gated think-steps on
/// the project-default agent whose artifacts accumulate in the dossier;
/// Quick is the gateless small-work escape hatch advancing straight to
/// Ship; Implement runs strict TDD and auto-advances into Adversarial
/// (deliberate confirm-free chain — the hostile pass is the step that
/// must never be skipped), which auto-advances its findings into the
/// Review human gate; Ship is the merge verb (squash-merge + push +
/// worktree cleanup, frontend-driven) and counts as done. Ship carries
/// seed_lane 'done' so lane sync, imports, and the cleanup-dialog
/// trigger all keep working unchanged.
const SEED_COLUMNS: &[SeedColumn] = &[
    SeedColumn {
        name: "Idea",
        kind: ColumnKind::Shelf,
        counts_as_done: false,
        is_landing: true,
        on_enter: OnEnter::Queue,
        on_settle: OnSettle::Hold,
        seed_lane: Some("backlog"),
        advance_to_name: None,
        step_prompt: None,
        step_provider: None,
        step_model: None,
    },
    SeedColumn {
        name: "Grill",
        kind: ColumnKind::AgentStep,
        counts_as_done: false,
        is_landing: false,
        on_enter: OnEnter::Queue,
        on_settle: OnSettle::Hold,
        seed_lane: None,
        advance_to_name: None,
        step_prompt: Some(GRILL_PROMPT),
        step_provider: None,
        step_model: None,
    },
    SeedColumn {
        name: "Quick",
        kind: ColumnKind::AgentStep,
        counts_as_done: false,
        is_landing: false,
        on_enter: OnEnter::Run,
        on_settle: OnSettle::Advance,
        seed_lane: None,
        // Straight to Ship — without the pin a Quick card would walk
        // into Plan/Implement and fire a second unconfirmed dispatch.
        advance_to_name: Some("Ship"),
        step_prompt: None,
        step_provider: None,
        step_model: None,
    },
    SeedColumn {
        name: "Plan",
        kind: ColumnKind::AgentStep,
        counts_as_done: false,
        is_landing: false,
        on_enter: OnEnter::Queue,
        on_settle: OnSettle::Hold,
        seed_lane: Some("ready"),
        advance_to_name: None,
        step_prompt: Some(PLAN_PROMPT),
        step_provider: None,
        step_model: None,
    },
    SeedColumn {
        name: "Implement",
        kind: ColumnKind::AgentStep,
        counts_as_done: false,
        is_landing: false,
        on_enter: OnEnter::Run,
        on_settle: OnSettle::Advance,
        seed_lane: Some("in_progress"),
        // Pinned to Adversarial (never next-by-position): the hostile
        // pass survives reorders and neighbor deletion.
        advance_to_name: Some("Adversarial"),
        step_prompt: Some(IMPLEMENT_PROMPT),
        step_provider: None,
        step_model: None,
    },
    SeedColumn {
        name: "Adversarial",
        kind: ColumnKind::AgentStep,
        counts_as_done: false,
        is_landing: false,
        on_enter: OnEnter::Run,
        on_settle: OnSettle::Advance,
        seed_lane: None,
        // Findings land in front of the human gate.
        advance_to_name: Some("Review"),
        step_prompt: Some(ADVERSARIAL_PROMPT),
        step_provider: None,
        step_model: None,
    },
    SeedColumn {
        name: "Review",
        kind: ColumnKind::HumanGate,
        counts_as_done: false,
        is_landing: false,
        on_enter: OnEnter::Queue,
        on_settle: OnSettle::Hold,
        seed_lane: Some("in_review"),
        advance_to_name: None,
        step_prompt: None,
        step_provider: None,
        step_model: None,
    },
    SeedColumn {
        name: "Ship",
        kind: ColumnKind::Ship,
        counts_as_done: true,
        is_landing: false,
        on_enter: OnEnter::Queue,
        on_settle: OnSettle::Hold,
        seed_lane: Some("done"),
        advance_to_name: None,
        step_prompt: None,
        step_provider: None,
        step_model: None,
    },
];

/// Seeds the default column set for a project (called inside the
/// project-create transaction — migration 0006 covers projects that
/// existed before it ran). Idempotent per project: a project that already
/// has columns is left untouched.
pub fn seed_default_columns(conn: &rusqlite::Connection, project_id: &str) -> Result<(), Error> {
    let has_columns: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM board_columns WHERE project_id = ?1)",
        [project_id],
        |row| row.get(0),
    )?;
    if has_columns {
        return Ok(());
    }
    let mut id_by_name: Vec<(&'static str, String)> = Vec::new();
    let mut pending_targets: Vec<(String, &'static str)> = Vec::new();
    for (position, seed) in SEED_COLUMNS.iter().enumerate() {
        let id = format!("col_{}", uuid::Uuid::new_v4());
        conn.execute(
            "INSERT INTO board_columns
                 (id, project_id, name, position, kind, counts_as_done,
                  is_landing, on_enter, on_settle, seed_lane,
                  step_provider, step_model, step_prompt)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                id,
                project_id,
                seed.name,
                position as i64,
                seed.kind.as_str(),
                seed.counts_as_done,
                seed.is_landing,
                seed.on_enter.as_str(),
                seed.on_settle.as_str(),
                seed.seed_lane,
                seed.step_provider,
                seed.step_model,
                seed.step_prompt,
            ],
        )?;
        id_by_name.push((seed.name, id.clone()));
        if let Some(target) = seed.advance_to_name {
            pending_targets.push((id, target));
        }
    }
    // Second pass: wire advance_to targets (Quick → Ship, Implement →
    // Adversarial, Adversarial → Review) once every target row exists.
    for (id, target_name) in pending_targets {
        let target_id = id_by_name
            .iter()
            .find(|(name, _)| *name == target_name)
            .map(|(_, id)| id.clone())
            .ok_or_else(|| {
                Error::Internal(format!(
                    "seed advance_to target '{target_name}' missing from SEED_COLUMNS"
                ))
            })?;
        conn.execute(
            "UPDATE board_columns SET advance_to = ?2 WHERE id = ?1",
            rusqlite::params![id, target_id],
        )?;
    }
    Ok(())
}

/// Composes the prompt for an agent step (E18-04, ADR-0037 item 2).
///
/// `step_prompt` NULL/blank → the reference packet alone — byte-identical
/// to today's dispatch. A set `step_prompt` becomes the framing, with the
/// reference packet (issue title/body/acceptance/PRD/conventions — built
/// by `build_dispatch_prompt`, composed here rather than duplicated)
/// appended under a labeled divider.
pub fn compose_step_prompt(step_prompt: Option<&str>, packet: &str) -> String {
    match step_prompt.map(str::trim).filter(|s| !s.is_empty()) {
        None => packet.to_string(),
        Some(framing) => {
            format!("{framing}\n\n---\n\n# Reference: issue packet\n\n{packet}")
        }
    }
}

pub struct ColumnStore {
    db: Arc<dyn Db>,
}

/// Columns in `column_from_row` order.
const COLUMNS: &str = "id, project_id, name, position, kind, counts_as_done, is_landing, \
     on_enter, on_settle, advance_to, step_prompt, step_provider, step_model, \
     step_effort, step_tools, seed_lane, created_at, updated_at";

fn column_from_row(row: &rusqlite::Row) -> rusqlite::Result<BoardColumn> {
    let kind: String = row.get(4)?;
    let counts_as_done: i64 = row.get(5)?;
    let is_landing: i64 = row.get(6)?;
    let on_enter: String = row.get(7)?;
    let on_settle: String = row.get(8)?;
    let step_tools_cell: Option<String> = row.get(14)?;
    // Allowlist parse FAILS CLOSED: NULL cell = unrestricted (None), but a
    // present-yet-corrupt cell becomes the EMPTY allowlist (no tools) —
    // never silently unrestricted. parse_versioned logs the corruption;
    // the module's "reads never throw" philosophy rules out a read error.
    let step_tools = step_tools_cell.as_deref().map(|cell| {
        parse_versioned::<StepTools>("board_columns.step_tools", Some(cell))
            .map(|t| t.tools)
            .unwrap_or_default()
    });
    Ok(BoardColumn {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        position: row.get(3)?,
        kind: ColumnKind::parse(&kind).unwrap_or(ColumnKind::Shelf),
        counts_as_done: counts_as_done != 0,
        is_landing: is_landing != 0,
        on_enter: OnEnter::parse(&on_enter).unwrap_or(OnEnter::Queue),
        on_settle: OnSettle::parse(&on_settle).unwrap_or(OnSettle::Hold),
        advance_to: row.get(9)?,
        step_prompt: row.get(10)?,
        step_provider: row.get(11)?,
        step_model: row.get(12)?,
        step_effort: row.get(13)?,
        step_tools,
        seed_lane: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

/// Serializes a tool allowlist for storage: `None` = NULL cell
/// (unrestricted), `Some(vec)` = versioned JSON (even when empty).
fn step_tools_cell(tools: Option<&[String]>) -> Result<Option<String>, Error> {
    match tools {
        None => Ok(None),
        Some(tools) => serialize_versioned(&StepTools {
            tools: tools.to_vec(),
        })
        .map(Some),
    }
}

/// The landing column is never an `agent_step` (ADR-0037 item 7, amended).
///
/// Entry paths (GitHub import, PM proposal apply, manual add) write issue
/// rows directly and never run `on_enter`, so a run-mode landing column
/// would silently deposit inert cards — no launch, no park, no gate, and
/// no settle path to rescue them. Routing creation through the step
/// engine is explicitly rejected instead: a 50-issue import would launch
/// 50 agents. Work is dispatched by MOVING a card onto a step, never by
/// its arrival. Enforced on both create and update, in both directions
/// (flagging an agent step as landing, and turning the landing column
/// into an agent step).
fn reject_landing_agent_step(is_landing: bool, kind: ColumnKind) -> Result<(), Error> {
    if is_landing && kind == ColumnKind::AgentStep {
        return Err(Error::InvalidBoardColumnInput(
            "the landing column cannot be an agent step: entry paths create \
             cards directly and never fire on_enter, so arriving cards would \
             sit inert (ADR-0037 item 7) — move is_landing to a shelf or \
             human gate, or change this column's kind first"
                .into(),
        ));
    }
    if is_landing && kind == ColumnKind::Ship {
        return Err(Error::InvalidBoardColumnInput(
            "the landing column cannot be a ship column: arriving cards \
             would sit on the merge verb with nothing to merge — move \
             is_landing to a shelf or human gate first"
                .into(),
        ));
    }
    Ok(())
}

/// `advance_to` must point at an existing column of the same project.
fn validate_advance_target(
    conn: &rusqlite::Connection,
    project_id: &str,
    target: &str,
) -> Result<(), Error> {
    let ok: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM board_columns WHERE id = ?1 AND project_id = ?2)",
        rusqlite::params![target, project_id],
        |row| row.get(0),
    )?;
    if !ok {
        return Err(Error::InvalidBoardColumnInput(format!(
            "advance_to target {target} is not a column of project {project_id}"
        )));
    }
    Ok(())
}

impl ColumnStore {
    pub fn new(db: Arc<dyn Db>) -> Self {
        Self { db }
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>, Error> {
        self.db
            .conn()
            .lock()
            .map_err(|e| Error::Internal(format!("db mutex poisoned: {e}")))
    }

    pub fn get(&self, id: &str) -> Result<Option<BoardColumn>, Error> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM board_columns WHERE id = ?1"),
                [id],
                column_from_row,
            )
            .optional()?)
    }

    /// All columns for a project in board order (`position`, then
    /// `created_at` as the tiebreak — positions are kept compact, so ties
    /// only exist transiently).
    pub fn list_for_project(&self, project_id: &str) -> Result<Vec<BoardColumn>, Error> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM board_columns
              WHERE project_id = ?1
              ORDER BY position, created_at"
        ))?;
        let columns = stmt
            .query_map([project_id], column_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(columns)
    }

    /// Creates a column appended to the end of the board. `is_landing: true`
    /// moves the landing flag onto the new column (never duplicates it).
    /// `advance_to` must reference a same-project column.
    pub fn create(&self, new: NewColumn) -> Result<BoardColumn, Error> {
        let name = new.name.trim().to_string();
        if name.is_empty() {
            return Err(Error::InvalidBoardColumnInput("name is empty".into()));
        }
        reject_landing_agent_step(new.is_landing, new.kind)?;
        let id = format!("col_{}", uuid::Uuid::new_v4());
        let step_tools = step_tools_cell(new.step_tools.as_deref())?;
        {
            let conn = self.conn()?;
            let project_exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)",
                [&new.project_id],
                |row| row.get(0),
            )?;
            if !project_exists {
                return Err(Error::ProjectNotFound(new.project_id));
            }
            if let Some(target) = &new.advance_to {
                validate_advance_target(&conn, &new.project_id, target)?;
            }
            let tx = conn.unchecked_transaction()?;
            if new.is_landing {
                tx.execute(
                    "UPDATE board_columns SET is_landing = 0, updated_at = datetime('now')
                      WHERE project_id = ?1 AND is_landing = 1",
                    [&new.project_id],
                )?;
            }
            tx.execute(
                "INSERT INTO board_columns
                     (id, project_id, name, position, kind, counts_as_done,
                      is_landing, on_enter, on_settle, advance_to, step_prompt,
                      step_provider, step_model, step_effort, step_tools)
                 VALUES (
                     ?1, ?2, ?3,
                     (SELECT COALESCE(MAX(position) + 1, 0) FROM board_columns
                       WHERE project_id = ?2),
                     ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14
                 )",
                rusqlite::params![
                    id,
                    new.project_id,
                    name,
                    new.kind.as_str(),
                    new.counts_as_done,
                    new.is_landing,
                    new.on_enter.unwrap_or(OnEnter::Queue).as_str(),
                    new.on_settle.unwrap_or(OnSettle::Hold).as_str(),
                    new.advance_to,
                    new.step_prompt,
                    new.step_provider,
                    new.step_model,
                    new.step_effort,
                    step_tools,
                ],
            )?;
            tx.commit()?;
        }
        self.get(&id)?
            .ok_or_else(|| Error::Internal(format!("column vanished after insert: {id}")))
    }

    /// Applies a field patch. `is_landing: Some(true)` moves the landing
    /// flag from its current holder; `Some(false)` on the landing column is
    /// rejected (set the flag on another column instead — the board always
    /// has exactly one landing column). `advance_to` targets are validated
    /// (same project, never self). An untouched `step_tools` leaves the
    /// stored cell exactly as it was.
    pub fn update(&self, id: &str, patch: ColumnPatch) -> Result<BoardColumn, Error> {
        let mut column = self
            .get(id)?
            .ok_or_else(|| Error::BoardColumnNotFound(id.into()))?;
        if let Some(name) = patch.name {
            let name = name.trim().to_string();
            if name.is_empty() {
                return Err(Error::InvalidBoardColumnInput("name is empty".into()));
            }
            column.name = name;
        }
        if let Some(kind) = patch.kind {
            column.kind = kind;
        }
        if let Some(counts_as_done) = patch.counts_as_done {
            column.counts_as_done = counts_as_done;
        }
        let move_landing_here = match patch.is_landing {
            Some(true) => !column.is_landing,
            Some(false) if column.is_landing => {
                return Err(Error::InvalidBoardColumnInput(format!(
                    "column {id} is the landing column; set is_landing on another \
                     column to move the flag instead of clearing it"
                )));
            }
            _ => false,
        };
        if patch.is_landing == Some(true) {
            column.is_landing = true;
        }
        // Both directions of the landing-kind rule: the merged column is
        // what gets stored, so flagging an agent step as landing AND
        // turning the landing column into an agent step both land here.
        reject_landing_agent_step(column.is_landing, column.kind)?;
        if let Some(on_enter) = patch.on_enter {
            column.on_enter = on_enter;
        }
        if let Some(on_settle) = patch.on_settle {
            column.on_settle = on_settle;
        }
        if let Some(advance_to) = patch.advance_to {
            if let Some(target) = &advance_to {
                if target == id {
                    return Err(Error::InvalidBoardColumnInput(format!(
                        "column {id} cannot advance_to itself"
                    )));
                }
            }
            column.advance_to = advance_to;
        }
        if let Some(step_prompt) = patch.step_prompt {
            column.step_prompt = step_prompt;
        }
        if let Some(step_provider) = patch.step_provider {
            column.step_provider = step_provider;
        }
        if let Some(step_model) = patch.step_model {
            column.step_model = step_model;
        }
        if let Some(step_effort) = patch.step_effort {
            column.step_effort = step_effort;
        }
        // step_tools is handled apart from the merged model: only an
        // explicit patch writes the cell, so an unrelated update can never
        // round-trip (and thereby erase) a corrupt stored value through the
        // lossy fail-closed parse.
        let step_tools_update = match &patch.step_tools {
            Some(next) => Some(step_tools_cell(next.as_deref())?),
            None => None,
        };
        if let Some(step_tools) = patch.step_tools {
            column.step_tools = step_tools;
        }
        {
            let conn = self.conn()?;
            if let Some(target) = &column.advance_to {
                validate_advance_target(&conn, &column.project_id, target)?;
            }
            let tx = conn.unchecked_transaction()?;
            if move_landing_here {
                tx.execute(
                    "UPDATE board_columns SET is_landing = 0, updated_at = datetime('now')
                      WHERE project_id = ?1 AND is_landing = 1",
                    [&column.project_id],
                )?;
            }
            tx.execute(
                "UPDATE board_columns SET name = ?2, kind = ?3, counts_as_done = ?4,
                     is_landing = ?5, on_enter = ?6, on_settle = ?7,
                     advance_to = ?8, step_prompt = ?9, step_provider = ?10,
                     step_model = ?11, step_effort = ?12,
                     updated_at = datetime('now')
                  WHERE id = ?1",
                rusqlite::params![
                    id,
                    column.name,
                    column.kind.as_str(),
                    column.counts_as_done,
                    column.is_landing,
                    column.on_enter.as_str(),
                    column.on_settle.as_str(),
                    column.advance_to,
                    column.step_prompt,
                    column.step_provider,
                    column.step_model,
                    column.step_effort,
                ],
            )?;
            if let Some(cell) = step_tools_update {
                tx.execute(
                    "UPDATE board_columns SET step_tools = ?2 WHERE id = ?1",
                    rusqlite::params![id, cell],
                )?;
            }
            tx.commit()?;
        }
        self.get(id)?
            .ok_or_else(|| Error::Internal(format!("column vanished after update: {id}")))
    }

    /// Deletes a column and compacts the remaining positions to 0..n-1.
    /// Rejected when the column is occupied ([`Error::BoardColumnHasIssues`]),
    /// when it is the landing column (move the flag first — the board
    /// always has exactly one), or when it is another column's
    /// `advance_to` target ([`Error::BoardColumnIsAdvanceTarget`] —
    /// repoint the referrer first).
    ///
    /// E18-07 (#66): occupancy is strictly `column_id = ?` — the pointer
    /// is authoritative and non-NULL on every row, so the pre-flip
    /// seed_lane fallback arm is gone, and with it the temporary
    /// seeded-agent-step delete lock: an EMPTY seeded step deletes like
    /// any other column (lane-addressed paths now error on a deleted
    /// seeded column instead of dispatching into the void).
    pub fn delete(&self, id: &str) -> Result<(), Error> {
        let column = self
            .get(id)?
            .ok_or_else(|| Error::BoardColumnNotFound(id.into()))?;
        if column.is_landing {
            return Err(Error::InvalidBoardColumnInput(format!(
                "column {id} is the landing column; move is_landing to another \
                 column before deleting it"
            )));
        }
        let conn = self.conn()?;
        // EXACTLY ONE column owns each card (fix round): the mirror wins
        // when set — `column_id`'s FK is `ON DELETE SET NULL`, so a
        // non-NULL value always names a live column — and the lane
        // mapping covers only mirrorless (pre-E18) rows. Counting both
        // unconditionally double-booked cards resident in a NON-SEEDED
        // column (e.g. a landing Triage), whose interim lane fallback is
        // Backlog: the seeded Backlog column then looked occupied by
        // cards the board draws elsewhere and could never be deleted.
        let issue_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM issues WHERE column_id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        )?;
        if issue_count > 0 {
            return Err(Error::BoardColumnHasIssues {
                id: id.into(),
                count: issue_count,
            });
        }
        // DECIDED (E18-07, #66): deleting an `advance_to` target is
        // refused, never degraded. The FK's `ON DELETE SET NULL` would
        // silently re-route `on_settle: advance` to next-by-position,
        // which can walk cards into an adjacent agent step and fire an
        // unconfirmed dispatch — the ADR-0037 item 4 spend hazard.
        let referrer: Option<String> = conn
            .query_row(
                "SELECT name FROM board_columns
                  WHERE advance_to = ?1
                  ORDER BY position LIMIT 1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(referrer) = referrer {
            return Err(Error::BoardColumnIsAdvanceTarget {
                id: id.into(),
                referrer,
            });
        }
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM board_columns WHERE id = ?1", [id])?;
        compact_positions(&tx, &column.project_id)?;
        tx.commit()?;
        Ok(())
    }

    /// Reorders a project's columns to exactly `ordered_ids` (a full
    /// permutation — anything missing, extra, or duplicated is rejected).
    /// Positions come out compact (0..n-1). Returns the new board order.
    pub fn reorder(
        &self,
        project_id: &str,
        ordered_ids: &[String],
    ) -> Result<Vec<BoardColumn>, Error> {
        {
            let conn = self.conn()?;
            let mut stmt = conn.prepare("SELECT id FROM board_columns WHERE project_id = ?1")?;
            let mut existing = stmt
                .query_map([project_id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            existing.sort_unstable();
            let mut requested = ordered_ids.to_vec();
            requested.sort_unstable();
            requested.dedup();
            if requested.len() != ordered_ids.len() || requested != existing {
                return Err(Error::InvalidBoardColumnInput(format!(
                    "reorder must list each of the project's {} column id(s) exactly once",
                    existing.len()
                )));
            }
            let tx = conn.unchecked_transaction()?;
            for (position, id) in ordered_ids.iter().enumerate() {
                tx.execute(
                    "UPDATE board_columns SET position = ?2, updated_at = datetime('now')
                      WHERE id = ?1",
                    rusqlite::params![id, position as i64],
                )?;
            }
            tx.commit()?;
        }
        self.list_for_project(project_id)
    }
}

/// Renumbers a project's columns to 0..n-1 in current board order.
fn compact_positions(conn: &rusqlite::Connection, project_id: &str) -> Result<(), Error> {
    let mut stmt = conn.prepare(
        "SELECT id FROM board_columns WHERE project_id = ?1
          ORDER BY position, created_at",
    )?;
    let ids = stmt
        .query_map([project_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<String>, _>>()?;
    for (position, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE board_columns SET position = ?2 WHERE id = ?1",
            rusqlite::params![id, position as i64],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDb;
    use crate::events::BroadcastEventBus;
    use crate::issues::{IssueStore, Lane, NewIssue};

    fn fixture() -> ColumnStore {
        let db: Arc<dyn Db> = SqliteDb::init(Some(":memory:")).unwrap();
        db.conn()
            .lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO projects (id, name, path) VALUES
                    ('p1', 'proj', '/tmp/proj'),
                    ('p2', 'other', '/tmp/other');",
            )
            .unwrap();
        ColumnStore::new(db)
    }

    fn seeded_fixture() -> ColumnStore {
        let store = fixture();
        seed_default_columns(&store.conn().unwrap(), "p1").unwrap();
        store
    }

    /// IssueStore over the same DB (scenario tests drive the authoritative
    /// lane through the real store, not raw SQL).
    fn issue_store(store: &ColumnStore) -> IssueStore {
        IssueStore::new(store.db.clone(), Arc::new(BroadcastEventBus::new(16)))
    }

    fn new_issue(title: &str) -> NewIssue {
        NewIssue {
            project_id: "p1".into(),
            title: title.into(),
            body: None,
            acceptance: Vec::new(),
            lane: None,
            provider: None,
            model: None,
            prd_path: None,
            prd_section: None,
            external_ref: None,
            dossier_path: None,
        }
    }

    fn new_column(name: &str) -> NewColumn {
        NewColumn {
            project_id: "p1".into(),
            name: name.into(),
            kind: ColumnKind::Shelf,
            counts_as_done: false,
            is_landing: false,
            on_enter: None,
            on_settle: None,
            advance_to: None,
            step_prompt: None,
            step_provider: None,
            step_model: None,
            step_effort: None,
            step_tools: None,
        }
    }

    fn raw_column_cell(store: &ColumnStore, id: &str, column: &str) -> Option<String> {
        store
            .conn()
            .unwrap()
            .query_row(
                &format!("SELECT {column} FROM board_columns WHERE id = ?1"),
                [id],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn seed_creates_the_adr_0037_default_board() {
        let store = seeded_fixture();
        let columns = store.list_for_project("p1").unwrap();
        let summary: Vec<(&str, ColumnKind, bool, bool, OnEnter, OnSettle)> = columns
            .iter()
            .map(|c| {
                (
                    c.name.as_str(),
                    c.kind,
                    c.counts_as_done,
                    c.is_landing,
                    c.on_enter,
                    c.on_settle,
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    "Idea",
                    ColumnKind::Shelf,
                    false,
                    true,
                    OnEnter::Queue,
                    OnSettle::Hold
                ),
                (
                    "Grill",
                    ColumnKind::AgentStep,
                    false,
                    false,
                    OnEnter::Queue,
                    OnSettle::Hold
                ),
                (
                    "Quick",
                    ColumnKind::AgentStep,
                    false,
                    false,
                    OnEnter::Run,
                    OnSettle::Advance
                ),
                (
                    "Plan",
                    ColumnKind::AgentStep,
                    false,
                    false,
                    OnEnter::Queue,
                    OnSettle::Hold
                ),
                (
                    "Implement",
                    ColumnKind::AgentStep,
                    false,
                    false,
                    OnEnter::Run,
                    OnSettle::Advance
                ),
                (
                    "Adversarial",
                    ColumnKind::AgentStep,
                    false,
                    false,
                    OnEnter::Run,
                    OnSettle::Advance
                ),
                (
                    "Review",
                    ColumnKind::HumanGate,
                    false,
                    false,
                    OnEnter::Queue,
                    OnSettle::Hold
                ),
                (
                    "Ship",
                    ColumnKind::Ship,
                    true,
                    false,
                    OnEnter::Queue,
                    OnSettle::Hold
                ),
            ]
        );
        // Quick advances straight to Ship; Implement pins its advance to
        // Adversarial and Adversarial to the Review human gate — pins
        // must survive reorders and neighbor deletion.
        let ship = &columns[7];
        let review = &columns[6];
        let adversarial = &columns[5];
        assert_eq!(columns[2].advance_to.as_deref(), Some(ship.id.as_str()));
        assert_eq!(
            columns[4].advance_to.as_deref(),
            Some(adversarial.id.as_str())
        );
        assert_eq!(
            columns[5].advance_to.as_deref(),
            Some(review.id.as_str())
        );
        // Every agent step rides the project-default agent (the old
        // claude·haiku Quick pin is gone — pipeline overhaul).
        assert!(columns.iter().all(|c| c.step_provider.is_none()));
        assert!(columns.iter().all(|c| c.step_model.is_none()));
        // The think/attack steps carry their pipeline prompts; the rest
        // ride the bare dispatch packet.
        let prompts: Vec<bool> = columns.iter().map(|c| c.step_prompt.is_some()).collect();
        assert_eq!(
            prompts,
            vec![false, true, false, true, true, true, false, false]
        );
        // seed_lane mirrors the legacy lanes where one fits; Grill,
        // Quick, and Adversarial have none.
        let seed_lanes: Vec<Option<&str>> =
            columns.iter().map(|c| c.seed_lane.as_deref()).collect();
        assert_eq!(
            seed_lanes,
            vec![
                Some("backlog"),
                None,
                None,
                Some("ready"),
                Some("in_progress"),
                None,
                Some("in_review"),
                Some("done"),
            ]
        );
        // Positions are compact board order; ids carry the col_ prefix.
        let positions: Vec<i64> = columns.iter().map(|c| c.position).collect();
        assert_eq!(positions, vec![0, 1, 2, 3, 4, 5, 6, 7]);
        assert!(columns.iter().all(|c| c.id.starts_with("col_")));
        // Idempotent: a second seed call leaves the board alone.
        seed_default_columns(&store.conn().unwrap(), "p1").unwrap();
        assert_eq!(store.list_for_project("p1").unwrap().len(), 8);
        // Project scoping: p2 was never seeded.
        assert!(store.list_for_project("p2").unwrap().is_empty());
    }

    /// Integration test of migrations 0006 + 0007 against a database that
    /// already has projects and laned issues: replays migrations 0000–0005
    /// on a raw connection, inserts pre-columns data, applies 0006 (seed +
    /// backfill) and 0007 (the In Progress → In Review pin, shipped
    /// separately because 0006 landed and is sha256-frozen), and checks
    /// the seed (incl. both advance_to pins and the seed_lane markers) +
    /// the `issues.column_id` backfill.
    #[test]
    fn migration_0006_seeds_existing_projects_and_backfills_column_id() {
        const PRIOR: &[&str] = &[
            include_str!("../../migrations/0000_initial.sql"),
            include_str!("../../migrations/0001_line_comments.sql"),
            include_str!("../../migrations/0002_issues.sql"),
            include_str!("../../migrations/0003_issue_external_ref.sql"),
            include_str!("../../migrations/0004_provider_auth_method.sql"),
            include_str!("../../migrations/0005_pull_requests.sql"),
        ];
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        let apply = |sql: &str| {
            for statement in sql.split("--> statement-breakpoint") {
                let statement = statement.trim();
                if !statement.is_empty() {
                    conn.execute_batch(statement).unwrap();
                }
            }
        };
        for sql in PRIOR {
            apply(sql);
        }
        conn.execute_batch(
            "INSERT INTO projects (id, name, path) VALUES
                ('p1', 'proj', '/tmp/proj'),
                ('p2', 'other', '/tmp/other');
             INSERT INTO issues (id, project_id, title, lane) VALUES
                ('i1', 'p1', 'a', 'backlog'),
                ('i2', 'p1', 'b', 'in_progress'),
                ('i3', 'p1', 'c', 'done'),
                ('i4', 'p2', 'd', 'in_progress');",
        )
        .unwrap();

        apply(include_str!("../../migrations/0006_board_columns.sql"));
        apply(include_str!(
            "../../migrations/0007_pin_in_progress_advance.sql"
        ));

        // Both pre-existing projects got the six-column default board, with
        // seed_lane on the five legacy columns and Quick advancing to the
        // project's OWN Done column.
        for project in ["p1", "p2"] {
            let rows: Vec<(String, Option<String>, Option<String>)> = conn
                .prepare(
                    "SELECT name, seed_lane, advance_to FROM board_columns
                      WHERE project_id = ?1 ORDER BY position",
                )
                .unwrap()
                .query_map([project], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let names: Vec<&str> = rows.iter().map(|(n, _, _)| n.as_str()).collect();
            assert_eq!(
                names,
                vec![
                    "Backlog",
                    "Ready",
                    "Quick",
                    "In Progress",
                    "In Review",
                    "Done"
                ]
            );
            let seed_lanes: Vec<Option<&str>> = rows.iter().map(|(_, l, _)| l.as_deref()).collect();
            assert_eq!(
                seed_lanes,
                vec![
                    Some("backlog"),
                    Some("ready"),
                    None,
                    Some("in_progress"),
                    Some("in_review"),
                    Some("done"),
                ]
            );
            let done_id: String = conn
                .query_row(
                    "SELECT id FROM board_columns
                      WHERE project_id = ?1 AND seed_lane = 'done'",
                    [project],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(rows[2].2.as_deref(), Some(done_id.as_str()), "Quick → Done");
            let in_review_id: String = conn
                .query_row(
                    "SELECT id FROM board_columns
                      WHERE project_id = ?1 AND seed_lane = 'in_review'",
                    [project],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                rows[3].2.as_deref(),
                Some(in_review_id.as_str()),
                "In Progress → In Review (pinned human gate)"
            );
            // Quick is pinned to claude · haiku; In Progress keeps the
            // project default (NULL provider/model).
            let step_pins: Vec<(String, Option<String>, Option<String>)> = conn
                .prepare(
                    "SELECT name, step_provider, step_model FROM board_columns
                      WHERE project_id = ?1 AND name IN ('Quick', 'In Progress')
                      ORDER BY position",
                )
                .unwrap()
                .query_map([project], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(
                step_pins,
                vec![
                    ("Quick".into(), Some("claude".into()), Some("haiku".into())),
                    ("In Progress".into(), None, None),
                ]
            );
        }
        // Backfill: each issue's mirror pointer targets its OWN project's
        // column mirroring its lane; the lane string itself is untouched.
        let issue_column = |issue: &str| -> (String, String, String) {
            conn.query_row(
                "SELECT i.lane, c.name, c.project_id
                   FROM issues i JOIN board_columns c ON c.id = i.column_id
                  WHERE i.id = ?1",
                [issue],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap()
        };
        assert_eq!(
            issue_column("i1"),
            ("backlog".into(), "Backlog".into(), "p1".into())
        );
        assert_eq!(
            issue_column("i2"),
            ("in_progress".into(), "In Progress".into(), "p1".into())
        );
        assert_eq!(
            issue_column("i3"),
            ("done".into(), "Done".into(), "p1".into())
        );
        assert_eq!(
            issue_column("i4"),
            ("in_progress".into(), "In Progress".into(), "p2".into())
        );
    }

    /// Migration 0013 against a database whose boards predate the Closed
    /// column: replays 0000–0007 with projects in place before 0006 (so
    /// both get the classic six-column seed), gives one project a
    /// hand-added trailing column and the other a pre-existing 'Closed',
    /// then applies 0013. The board lacking Closed gets exactly one,
    /// appended after everything (position = MAX+1) with the shelf /
    /// counts_as_done / no-seed_lane shape; the board already carrying a
    /// Closed is left untouched (no duplicate).
    #[test]
    fn migration_0013_appends_closed_to_existing_boards() {
        const PRIOR: &[&str] = &[
            include_str!("../../migrations/0000_initial.sql"),
            include_str!("../../migrations/0001_line_comments.sql"),
            include_str!("../../migrations/0002_issues.sql"),
            include_str!("../../migrations/0003_issue_external_ref.sql"),
            include_str!("../../migrations/0004_provider_auth_method.sql"),
            include_str!("../../migrations/0005_pull_requests.sql"),
        ];
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        let apply = |sql: &str| {
            for statement in sql.split("--> statement-breakpoint") {
                let statement = statement.trim();
                if !statement.is_empty() {
                    conn.execute_batch(statement).unwrap();
                }
            }
        };
        for sql in PRIOR {
            apply(sql);
        }
        // Projects exist before 0006 so both boards get the seeded six.
        conn.execute_batch(
            "INSERT INTO projects (id, name, path) VALUES
                ('p1', 'proj', '/tmp/proj'),
                ('p2', 'other', '/tmp/other');",
        )
        .unwrap();
        apply(include_str!("../../migrations/0006_board_columns.sql"));
        apply(include_str!(
            "../../migrations/0007_pin_in_progress_advance.sql"
        ));
        // p1: a user column already sits past the seeded six — Closed must
        // land AFTER it. p2: a Closed already exists (a user made their
        // own) — 0013 must not add a second.
        conn.execute_batch(
            "INSERT INTO board_columns (id, project_id, name, position)
                VALUES ('col_user', 'p1', 'Later', 6);
             INSERT INTO board_columns (id, project_id, name, position, counts_as_done)
                VALUES ('col_mine', 'p2', 'Closed', 6, 0);",
        )
        .unwrap();

        apply(include_str!("../../migrations/0013_closed_column.sql"));

        // p1: exactly one Closed, appended last, with the seeded shape.
        let (count, position, kind, counts_as_done, is_landing, on_enter, on_settle, seed_lane): (
            i64,
            i64,
            String,
            i64,
            i64,
            String,
            String,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT COUNT(*), position, kind, counts_as_done, is_landing,
                        on_enter, on_settle, seed_lane
                   FROM board_columns
                  WHERE project_id = 'p1' AND name = 'Closed'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(position, 7, "appended after the user column at 6");
        assert_eq!(kind, "shelf");
        assert_eq!(counts_as_done, 1);
        assert_eq!(is_landing, 0);
        assert_eq!(on_enter, "queue");
        assert_eq!(on_settle, "hold");
        assert_eq!(seed_lane, None);

        // p2: the hand-made Closed survives untouched and alone.
        let (count, id, counts_as_done): (i64, String, i64) = conn
            .query_row(
                "SELECT COUNT(*), id, counts_as_done FROM board_columns
                  WHERE project_id = 'p2' AND name = 'Closed'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(count, 1, "0013 must not duplicate an existing Closed");
        assert_eq!(id, "col_mine");
        assert_eq!(counts_as_done, 0, "the user's own column is untouched");

        // Re-application is a no-op (defense for a replayed journal).
        apply(include_str!("../../migrations/0013_closed_column.sql"));
        let total: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM board_columns WHERE name = 'Closed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(total, 2);
    }

    /// E18-07 migration 0008 (#66): a pre-flip database with mirrorless
    /// rows backfills TOTALLY — the lane's seeded column when it exists,
    /// the project's landing column when the seeded column was deleted
    /// (mirroring the frontend's columnIdForIssue display resolution).
    /// No NULL column_id survives; rows already carrying a mirror are
    /// untouched.
    #[test]
    fn migration_0008_backfills_every_mirrorless_row() {
        const PRIOR: &[&str] = &[
            include_str!("../../migrations/0000_initial.sql"),
            include_str!("../../migrations/0001_line_comments.sql"),
            include_str!("../../migrations/0002_issues.sql"),
            include_str!("../../migrations/0003_issue_external_ref.sql"),
            include_str!("../../migrations/0004_provider_auth_method.sql"),
            include_str!("../../migrations/0005_pull_requests.sql"),
            include_str!("../../migrations/0006_board_columns.sql"),
            include_str!("../../migrations/0007_pin_in_progress_advance.sql"),
        ];
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        let apply = |sql: &str| {
            for statement in sql.split("--> statement-breakpoint") {
                let statement = statement.trim();
                if !statement.is_empty() {
                    conn.execute_batch(statement).unwrap();
                }
            }
        };
        // Project exists before 0006 so the seed covers it.
        apply(PRIOR[0]);
        apply(PRIOR[1]);
        apply(PRIOR[2]);
        apply(PRIOR[3]);
        apply(PRIOR[4]);
        apply(PRIOR[5]);
        conn.execute_batch(
            "INSERT INTO projects (id, name, path) VALUES ('p1', 'proj', '/tmp/p');",
        )
        .unwrap();
        apply(PRIOR[6]);
        apply(PRIOR[7]);
        // Pre-flip states, constructed post-0006 (its backfill already
        // ran): mirrorless rows in various lanes — including one whose
        // seeded column gets DELETED — plus one row already mirrored.
        let done_id: String = conn
            .query_row(
                "SELECT id FROM board_columns WHERE project_id = 'p1' AND seed_lane = 'done'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        conn.execute_batch(&format!(
            "INSERT INTO issues (id, project_id, title, lane) VALUES
                ('i1', 'p1', 'a', 'backlog'),
                ('i2', 'p1', 'b', 'in_progress'),
                ('i3', 'p1', 'c', 'ready');
             INSERT INTO issues (id, project_id, title, lane, column_id) VALUES
                ('i4', 'p1', 'd', 'backlog', '{done_id}');
             DELETE FROM board_columns
              WHERE project_id = 'p1' AND seed_lane = 'ready';"
        ))
        .unwrap();

        apply(include_str!(
            "../../migrations/0008_column_id_authoritative.sql"
        ));

        let nulls: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM issues WHERE column_id IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(nulls, 0, "no NULL column_id survives the backfill");
        let column_name = |issue: &str| -> String {
            conn.query_row(
                "SELECT c.name FROM issues i JOIN board_columns c ON c.id = i.column_id
                  WHERE i.id = ?1",
                [issue],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(column_name("i1"), "Backlog"); // seeded-lane match
        assert_eq!(column_name("i2"), "In Progress"); // seeded-lane match
                                                      // The ready-laned row's seeded column is gone → landing column.
        assert_eq!(column_name("i3"), "Backlog");
        // An already-mirrored row is untouched, whatever its lane says.
        assert_eq!(column_name("i4"), "Done");
    }

    #[test]
    fn create_appends_validates_and_round_trips_step_config() {
        let store = seeded_fixture();
        let done_id = store.list_for_project("p1").unwrap()[7].id.clone();
        let created = store
            .create(NewColumn {
                kind: ColumnKind::AgentStep,
                on_enter: Some(OnEnter::Run),
                on_settle: Some(OnSettle::Advance),
                advance_to: Some(done_id.clone()),
                step_prompt: Some("Grill me on the plan.".into()),
                step_provider: Some("claude".into()),
                step_model: Some("fable".into()),
                step_effort: Some("high".into()),
                step_tools: Some(vec!["read".into(), "grep".into()]),
                ..new_column("Plan")
            })
            .unwrap();
        assert_eq!(created.position, 8); // appended after the seeded eight
        assert_eq!(created.kind, ColumnKind::AgentStep);
        assert_eq!(created.on_enter, OnEnter::Run);
        assert_eq!(created.on_settle, OnSettle::Advance);
        assert_eq!(created.advance_to.as_deref(), Some(done_id.as_str()));
        assert_eq!(
            created.step_prompt.as_deref(),
            Some("Grill me on the plan.")
        );
        assert_eq!(created.step_provider.as_deref(), Some("claude"));
        assert_eq!(created.step_model.as_deref(), Some("fable"));
        assert_eq!(created.step_effort.as_deref(), Some("high"));
        assert_eq!(
            created.step_tools,
            Some(vec!["read".to_string(), "grep".to_string()])
        );
        assert!(!created.is_landing);
        assert!(created.seed_lane.is_none()); // user columns never get one
        let fetched = store.get(&created.id).unwrap().unwrap();
        assert_eq!(fetched, created);

        // Defaults: on_enter queue, on_settle hold, no advance target, and
        // an ABSENT allowlist (None = unrestricted).
        let plain = store.create(new_column("Later")).unwrap();
        assert_eq!(plain.on_enter, OnEnter::Queue);
        assert_eq!(plain.on_settle, OnSettle::Hold);
        assert!(plain.advance_to.is_none());
        assert!(plain.step_tools.is_none());

        assert!(matches!(
            store.create(new_column("   ")),
            Err(Error::InvalidBoardColumnInput(_))
        ));
        let mut orphan = new_column("x");
        orphan.project_id = "nope".into();
        assert!(matches!(
            store.create(orphan),
            Err(Error::ProjectNotFound(_))
        ));
    }

    #[test]
    fn advance_to_is_validated_on_create_and_update() {
        let store = seeded_fixture();
        let columns = store.list_for_project("p1").unwrap();
        let (quick, in_review) = (&columns[2], &columns[4]);

        // Create: target must exist in the SAME project.
        assert!(matches!(
            store.create(NewColumn {
                advance_to: Some("col_nope".into()),
                ..new_column("Broken")
            }),
            Err(Error::InvalidBoardColumnInput(_))
        ));
        let mut p2_col = new_column("P2 shelf");
        p2_col.project_id = "p2".into();
        let p2_col = store.create(p2_col).unwrap();
        assert!(matches!(
            store.create(NewColumn {
                advance_to: Some(p2_col.id.clone()),
                ..new_column("Cross-project")
            }),
            Err(Error::InvalidBoardColumnInput(_))
        ));

        // Update: same-project retarget works; self and foreign are
        // rejected; explicit clear falls back to next-column.
        let retargeted = store
            .update(
                &quick.id,
                ColumnPatch {
                    advance_to: Some(Some(in_review.id.clone())),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            retargeted.advance_to.as_deref(),
            Some(in_review.id.as_str())
        );
        assert!(matches!(
            store.update(
                &quick.id,
                ColumnPatch {
                    advance_to: Some(Some(quick.id.clone())),
                    ..Default::default()
                }
            ),
            Err(Error::InvalidBoardColumnInput(_))
        ));
        assert!(matches!(
            store.update(
                &quick.id,
                ColumnPatch {
                    advance_to: Some(Some(p2_col.id.clone())),
                    ..Default::default()
                }
            ),
            Err(Error::InvalidBoardColumnInput(_))
        ));
        let cleared = store
            .update(
                &quick.id,
                ColumnPatch {
                    advance_to: Some(None),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(cleared.advance_to.is_none());

        // E18-07 (#66): deleting a referrer's target is REFUSED with the
        // typed error naming the referrer — never FK-degraded to
        // next-column (the ADR-0037 item 4 spend hazard). Repointing the
        // referrer unlocks the delete.
        let target = store.create(new_column("Target")).unwrap();
        let pointer = store
            .create(NewColumn {
                advance_to: Some(target.id.clone()),
                ..new_column("Pointer")
            })
            .unwrap();
        let err = store.delete(&target.id).unwrap_err();
        assert!(matches!(err, Error::BoardColumnIsAdvanceTarget { .. }));
        assert_eq!(
            err.to_string(),
            format!(
                "column {} is the advance target of Pointer — repoint it first",
                target.id
            )
        );
        store
            .update(
                &pointer.id,
                ColumnPatch {
                    advance_to: Some(None),
                    ..Default::default()
                },
            )
            .unwrap();
        store.delete(&target.id).unwrap();
        assert!(store.get(&target.id).unwrap().is_none());
    }

    /// E18-07 (#66): the seeded board's own advance pins are protected —
    /// Ship is Quick's target, Adversarial is Implement's, Review is
    /// Adversarial's.
    #[test]
    fn seeded_advance_targets_cannot_be_deleted_until_repointed() {
        let store = seeded_fixture();
        let columns = store.list_for_project("p1").unwrap();
        let (quick, adversarial, review, ship) = (
            columns[2].clone(),
            columns[5].clone(),
            columns[6].clone(),
            columns[7].clone(),
        );

        let err = store.delete(&ship.id).unwrap_err();
        assert!(matches!(err, Error::BoardColumnIsAdvanceTarget { .. }));
        assert!(err.to_string().contains("advance target of Quick"));
        let err = store.delete(&adversarial.id).unwrap_err();
        assert!(err.to_string().contains("advance target of Implement"));
        let err = store.delete(&review.id).unwrap_err();
        assert!(err.to_string().contains("advance target of Adversarial"));

        // Clearing Quick's pin frees Ship.
        store
            .update(
                &quick.id,
                ColumnPatch {
                    advance_to: Some(None),
                    ..Default::default()
                },
            )
            .unwrap();
        store.delete(&ship.id).unwrap();
    }

    #[test]
    fn landing_flag_moves_and_never_duplicates() {
        let store = seeded_fixture();
        let landing_ids = |store: &ColumnStore| -> Vec<String> {
            store
                .list_for_project("p1")
                .unwrap()
                .into_iter()
                .filter(|c| c.is_landing)
                .map(|c| c.id)
                .collect()
        };
        let backlog = store.list_for_project("p1").unwrap()[0].clone();
        assert!(backlog.is_landing);

        // Creating a landing column MOVES the flag off Backlog.
        let inbox = store
            .create(NewColumn {
                is_landing: true,
                ..new_column("Inbox")
            })
            .unwrap();
        assert_eq!(landing_ids(&store), vec![inbox.id.clone()]);

        // Updating another column with the flag moves it again.
        let moved = store
            .update(
                &backlog.id,
                ColumnPatch {
                    is_landing: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(moved.is_landing);
        assert_eq!(landing_ids(&store), vec![backlog.id.clone()]);

        // Clearing the flag on the landing column is rejected: the board
        // always has exactly one landing column.
        assert!(matches!(
            store.update(
                &backlog.id,
                ColumnPatch {
                    is_landing: Some(false),
                    ..Default::default()
                }
            ),
            Err(Error::InvalidBoardColumnInput(_))
        ));
        // No-op clear on a non-landing column is fine.
        store
            .update(
                &inbox.id,
                ColumnPatch {
                    is_landing: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(landing_ids(&store), vec![backlog.id]);
    }

    #[test]
    fn update_patches_and_clears_fields() {
        let store = seeded_fixture();
        let created = store
            .create(NewColumn {
                step_prompt: Some("old prompt".into()),
                ..new_column("Plan")
            })
            .unwrap();
        let updated = store
            .update(
                &created.id,
                ColumnPatch {
                    name: Some("Plan v2".into()),
                    kind: Some(ColumnKind::AgentStep),
                    counts_as_done: Some(true),
                    on_enter: Some(OnEnter::Run),
                    step_model: Some(Some("fable".into())),
                    step_tools: Some(Some(vec!["read".into()])),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(updated.name, "Plan v2");
        assert_eq!(updated.kind, ColumnKind::AgentStep);
        assert!(updated.counts_as_done);
        assert_eq!(updated.on_enter, OnEnter::Run);
        assert_eq!(updated.step_prompt.as_deref(), Some("old prompt")); // untouched
        assert_eq!(updated.step_model.as_deref(), Some("fable"));
        assert_eq!(updated.step_tools, Some(vec!["read".to_string()]));

        // Some(None) clears (the wire-level `null`); omitted fields stay.
        let cleared = store
            .update(
                &created.id,
                ColumnPatch {
                    step_prompt: Some(None),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(cleared.step_prompt.is_none(), "explicit null must clear");
        assert_eq!(cleared.name, "Plan v2");
        // Clearing the allowlist returns the column to unrestricted.
        let unrestricted = store
            .update(
                &created.id,
                ColumnPatch {
                    step_tools: Some(None),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(unrestricted.step_tools.is_none());
        assert!(raw_column_cell(&store, &created.id, "step_tools").is_none());

        assert!(matches!(
            store.update(
                &created.id,
                ColumnPatch {
                    name: Some("  ".into()),
                    ..Default::default()
                }
            ),
            Err(Error::InvalidBoardColumnInput(_))
        ));
        assert!(matches!(
            store.update("nope", ColumnPatch::default()),
            Err(Error::BoardColumnNotFound(_))
        ));
    }

    #[test]
    fn corrupt_step_tools_fails_closed_and_survives_unrelated_updates() {
        let store = seeded_fixture();
        let created = store
            .create(NewColumn {
                step_tools: Some(vec!["read".into()]),
                ..new_column("Guarded")
            })
            .unwrap();
        // Corrupt the stored cell behind the store's back.
        store
            .conn()
            .unwrap()
            .execute(
                "UPDATE board_columns SET step_tools = '{nope' WHERE id = ?1",
                [&created.id],
            )
            .unwrap();

        // Fail CLOSED: the corrupt allowlist reads as EMPTY (no tools) —
        // never as None/unrestricted.
        let read = store.get(&created.id).unwrap().unwrap();
        assert_eq!(read.step_tools, Some(vec![]));

        // An update that does not touch step_tools preserves the corrupt
        // raw cell byte-for-byte instead of round-tripping it through the
        // lossy parse.
        store
            .update(
                &created.id,
                ColumnPatch {
                    name: Some("Guarded v2".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            raw_column_cell(&store, &created.id, "step_tools").as_deref(),
            Some("{nope")
        );

        // An explicit step_tools patch is the deliberate way out.
        let repaired = store
            .update(
                &created.id,
                ColumnPatch {
                    step_tools: Some(Some(vec!["grep".into()])),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(repaired.step_tools, Some(vec!["grep".to_string()]));
        assert!(raw_column_cell(&store, &created.id, "step_tools")
            .unwrap()
            .contains("\"version\":1"));
    }

    /// E18-07 (#66): occupancy is strictly by the authoritative
    /// `column_id` — a stale display lane never blocks a delete, and an
    /// occupied pointer always does.
    #[test]
    fn delete_guard_occupancy_is_strictly_by_column_id() {
        let store = seeded_fixture();
        let issues = issue_store(&store);
        let ready = store.list_for_project("p1").unwrap()[3].clone();
        assert_eq!(ready.seed_lane.as_deref(), Some("ready"));

        // A card resident in Ready (column_id set) blocks the delete.
        let card = issues
            .create(NewIssue {
                lane: Some(Lane::Ready),
                ..new_issue("card")
            })
            .unwrap();
        assert_eq!(card.column_id.as_deref(), Some(ready.id.as_str()));
        assert!(matches!(
            store.delete(&ready.id),
            Err(Error::BoardColumnHasIssues { ref id, count: 1 }) if *id == ready.id
        ));

        // Move the card into Quick (non-seeded): its DISPLAY lane stays
        // 'ready' but the pointer moved — Ready is empty by the only
        // signal that counts and deletes cleanly.
        let quick_id = store.list_for_project("p1").unwrap()[2].id.clone();
        issues.enter_column(&card.id, &quick_id, None).unwrap();
        let parked = issues.get(&card.id).unwrap().unwrap();
        assert_eq!(parked.column_id.as_deref(), Some(quick_id.as_str()));
        assert_eq!(parked.lane, Lane::Ready); // stale display lane
        store.delete(&ready.id).unwrap();
        let after = store.list_for_project("p1").unwrap();
        let names: Vec<&str> = after.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Idea", "Grill", "Quick", "Implement", "Adversarial", "Review", "Ship"]
        );
        let positions: Vec<i64> = after.iter().map(|c| c.position).collect();
        assert_eq!(positions, vec![0, 1, 2, 3, 4, 5, 6]);

        // The landing column can never be deleted; unknown ids are typed.
        assert!(matches!(
            store.delete(&after[0].id),
            Err(Error::InvalidBoardColumnInput(_))
        ));
        assert!(matches!(
            store.delete("nope"),
            Err(Error::BoardColumnNotFound(_))
        ));
    }

    /// Fix round (finding 2): with the landing flag on a NON-seeded
    /// column, a created card's interim lane fallback is Backlog — but
    /// only the landing column may own it. The seeded Backlog column must
    /// stay deletable (the board draws it empty); the owner must not.
    #[test]
    fn non_seeded_landing_column_owns_its_cards_alone() {
        let store = seeded_fixture();
        let issues = issue_store(&store);
        let triage = store
            .create(NewColumn {
                is_landing: true, // moves the flag off Backlog
                ..new_column("Triage")
            })
            .unwrap();
        assert!(triage.seed_lane.is_none());

        let card = issues.create(new_issue("landed")).unwrap();
        assert_eq!(card.column_id.as_deref(), Some(triage.id.as_str()));
        assert_eq!(card.lane, Lane::Backlog); // interim fallback

        // Backlog is empty as far as the board is concerned → deletable
        // (it is no longer the landing column, so no landing guard).
        let backlog = store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.seed_lane.as_deref() == Some("backlog"))
            .unwrap();
        store.delete(&backlog.id).unwrap();

        // Triage owns the card: it cannot be deleted while occupied.
        // (Landing guard first — move the flag, then the occupancy guard
        // is what refuses.)
        let review = store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.name == "Review")
            .unwrap();
        store
            .update(
                &review.id,
                ColumnPatch {
                    is_landing: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(matches!(
            store.delete(&triage.id),
            Err(Error::BoardColumnHasIssues { ref id, count: 1 }) if *id == triage.id
        ));
    }

    /// Fix round (finding 1, ADR-0037 item 7 amended): the landing column
    /// is never an agent_step — entry paths write rows directly and never
    /// fire `on_enter`, so arriving cards would sit inert.
    #[test]
    fn landing_column_cannot_be_an_agent_step() {
        let store = seeded_fixture();

        // Direction 1 — create an agent step WITH the landing flag.
        let err = store
            .create(NewColumn {
                kind: ColumnKind::AgentStep,
                is_landing: true,
                ..new_column("Auto Intake")
            })
            .unwrap_err();
        assert!(matches!(err, Error::InvalidBoardColumnInput(_)));
        assert!(err
            .to_string()
            .contains("landing column cannot be an agent step"));
        // The rejected create is atomic: Backlog still holds the flag.
        let landing: Vec<String> = store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .filter(|c| c.is_landing)
            .map(|c| c.name)
            .collect();
        assert_eq!(landing, vec!["Idea"]);

        // Direction 2 — flag an EXISTING agent step as landing.
        let quick = store.list_for_project("p1").unwrap()[2].clone();
        assert_eq!(quick.kind, ColumnKind::AgentStep);
        assert!(matches!(
            store.update(
                &quick.id,
                ColumnPatch {
                    is_landing: Some(true),
                    ..Default::default()
                }
            ),
            Err(Error::InvalidBoardColumnInput(_))
        ));

        // Direction 3 — turn the EXISTING landing column into an agent
        // step.
        let backlog = store.list_for_project("p1").unwrap()[0].clone();
        assert!(backlog.is_landing);
        assert!(matches!(
            store.update(
                &backlog.id,
                ColumnPatch {
                    kind: Some(ColumnKind::AgentStep),
                    ..Default::default()
                }
            ),
            Err(Error::InvalidBoardColumnInput(_))
        ));

        // Legal transitions still work: a shelf/human gate may hold the
        // flag, a non-landing column may become an agent step, and an
        // agent step may become a shelf and THEN take the flag.
        let triage = store
            .create(NewColumn {
                is_landing: true,
                ..new_column("Triage")
            })
            .unwrap();
        assert!(triage.is_landing);
        store
            .update(
                &backlog.id,
                ColumnPatch {
                    kind: Some(ColumnKind::AgentStep), // no longer landing
                    ..Default::default()
                },
            )
            .unwrap();
        let gate = store
            .create(NewColumn {
                kind: ColumnKind::HumanGate,
                is_landing: true,
                ..new_column("Intake gate")
            })
            .unwrap();
        assert!(gate.is_landing);
        let demoted = store
            .update(
                &quick.id,
                ColumnPatch {
                    kind: Some(ColumnKind::Shelf),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(demoted.kind, ColumnKind::Shelf);
        let promoted = store
            .update(
                &quick.id,
                ColumnPatch {
                    is_landing: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(promoted.is_landing);
    }

    /// The seeded board satisfies the landing-kind invariant, so no
    /// existing data violates it (no migration needed).
    #[test]
    fn seeded_board_satisfies_the_landing_kind_invariant() {
        let store = seeded_fixture();
        let landing: Vec<BoardColumn> = store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .filter(|c| c.is_landing)
            .collect();
        assert_eq!(landing.len(), 1);
        assert_eq!(landing[0].name, "Idea");
        assert_ne!(landing[0].kind, ColumnKind::AgentStep);
        assert_ne!(landing[0].kind, ColumnKind::Ship);
    }

    #[test]
    fn delete_guard_uses_the_mirror_for_non_lane_columns() {
        let store = seeded_fixture();
        let quick = store.list_for_project("p1").unwrap()[2].clone();
        assert!(quick.seed_lane.is_none());

        // No lane can express membership of Quick, so the column_id
        // pointer is the occupancy signal there.
        store
            .conn()
            .unwrap()
            .execute(
                "INSERT INTO issues (id, project_id, title, lane, column_id)
                 VALUES ('iq', 'p1', 'in quick', 'backlog', ?1)",
                [&quick.id],
            )
            .unwrap();
        assert!(matches!(
            store.delete(&quick.id),
            Err(Error::BoardColumnHasIssues { count: 1, .. })
        ));
        store
            .conn()
            .unwrap()
            .execute("UPDATE issues SET column_id = NULL WHERE id = 'iq'", [])
            .unwrap();
        store.delete(&quick.id).unwrap();
    }

    #[test]
    fn reorder_requires_a_full_permutation_and_compacts() {
        let store = seeded_fixture();
        let ids: Vec<String> = store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();

        // Reverse the board.
        let reversed: Vec<String> = ids.iter().rev().cloned().collect();
        let after = store.reorder("p1", &reversed).unwrap();
        let names: Vec<&str> = after.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Ship",
                "Review",
                "Adversarial",
                "Implement",
                "Plan",
                "Quick",
                "Grill",
                "Idea"
            ]
        );
        let positions: Vec<i64> = after.iter().map(|c| c.position).collect();
        assert_eq!(positions, vec![0, 1, 2, 3, 4, 5, 6, 7]);

        // Missing, duplicated, and foreign ids are all rejected.
        assert!(matches!(
            store.reorder("p1", &reversed[1..]),
            Err(Error::InvalidBoardColumnInput(_))
        ));
        let mut duplicated = reversed.clone();
        duplicated[0] = duplicated[1].clone();
        assert!(matches!(
            store.reorder("p1", &duplicated),
            Err(Error::InvalidBoardColumnInput(_))
        ));
        let mut foreign = reversed.clone();
        foreign[0] = "col_nope".into();
        assert!(matches!(
            store.reorder("p1", &foreign),
            Err(Error::InvalidBoardColumnInput(_))
        ));
        // p2 has no columns: only the empty permutation is valid.
        assert!(store.reorder("p2", &[]).unwrap().is_empty());
        assert!(matches!(
            store.reorder("p2", &reversed),
            Err(Error::InvalidBoardColumnInput(_))
        ));
    }

    /// E18-07 (#66): the TEMPORARY seeded-agent-step delete lock is
    /// LIFTED — an empty seeded step deletes like any other column
    /// (lane-addressed paths now fail typed on a deleted seeded column
    /// instead of dispatching into the void).
    #[test]
    fn seeded_agent_step_columns_are_deletable_when_empty() {
        let store = seeded_fixture();
        let in_progress = store.list_for_project("p1").unwrap()[4].clone();
        assert_eq!(in_progress.seed_lane.as_deref(), Some("in_progress"));
        assert_eq!(in_progress.kind, ColumnKind::AgentStep);
        store.delete(&in_progress.id).unwrap();
        // Quick (non-seeded step) stays deletable too.
        let quick = store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.name == "Quick")
            .unwrap();
        store.delete(&quick.id).unwrap();
    }

    /// E18-04: NULL/blank step_prompt = the packet byte-identical to
    /// today's dispatch; a set prompt frames it with the packet appended.
    #[test]
    fn compose_step_prompt_frames_or_passes_through() {
        let packet = "You are implementing an issue from the project board.\n\n# Issue\nX\n";
        assert_eq!(compose_step_prompt(None, packet), packet);
        assert_eq!(compose_step_prompt(Some("   "), packet), packet);
        let framed = compose_step_prompt(Some("Grill me on the plan."), packet);
        assert!(framed.starts_with("Grill me on the plan.\n\n---\n\n# Reference: issue packet\n\n"));
        assert!(framed.ends_with(packet));
    }

    #[test]
    fn project_delete_cascades_columns() {
        let store = seeded_fixture();
        assert_eq!(store.list_for_project("p1").unwrap().len(), 8);
        store
            .conn()
            .unwrap()
            .execute("DELETE FROM projects WHERE id = 'p1'", [])
            .unwrap();
        assert!(store.list_for_project("p1").unwrap().is_empty());
    }
}
