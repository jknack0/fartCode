//! Configurable pipeline columns (E18-01/E18-02, ADR-0037).
//!
//! Per-project board columns as data: each column carries a kind
//! (`shelf` | `agent_step` | `human_gate`), the `counts_as_done` and
//! `is_landing` flags, and — for agent steps — a step config (prompt,
//! provider, model, effort, tool allowlist) plus trigger (`on_enter`) and
//! settle (`on_settle`) behavior with an optional `advance_to` target
//! column (NULL = next column).
//!
//! **Spike, behind the seeded default:** `issues.lane` stays authoritative
//! for dispatch/flip/blocked/board order; `issues.column_id` is a mirrored
//! pointer set by the 0006 backfill and not yet maintained by any
//! production write path. Nothing here changes existing lane behavior.
//! Each of the five legacy seeded columns records the lane it mirrors in
//! `seed_lane`, so occupancy questions are answered by the authoritative
//! lane rather than the unmaintained mirror.
//!
//! Invariants enforced here:
//! - **Exactly one `is_landing` column per project.** Creating/updating a
//!   column with the flag *moves* it off the previous holder in the same
//!   transaction; clearing it directly (or deleting the landing column) is
//!   rejected — point the flag somewhere else first.
//! - **Positions are compact** (0..n-1 in board order) after every delete
//!   and reorder; create appends at the end.
//! - **Deleting an occupied column fails** with
//!   [`Error::BoardColumnHasIssues`]. Occupancy of a seeded column
//!   (`seed_lane` set) is derived from the authoritative lane; occupancy
//!   of any other column falls back to the `column_id` mirror, the only
//!   signal a non-lane column has.
//! - **`advance_to` must target a column in the same project** and never
//!   the column itself. The FK's `ON DELETE SET NULL` degrades a deleted
//!   target back to "next column".
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
}

impl ColumnKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ColumnKind::Shelf => "shelf",
            ColumnKind::AgentStep => "agent_step",
            ColumnKind::HumanGate => "human_gate",
        }
    }

    pub fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "shelf" => Ok(ColumnKind::Shelf),
            "agent_step" => Ok(ColumnKind::AgentStep),
            "human_gate" => Ok(ColumnKind::HumanGate),
            other => Err(Error::InvalidBoardColumnInput(format!(
                "invalid column kind: {other:?} (expected shelf|agent_step|human_gate)"
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
    /// create/update. Drives the occupancy guard while lane stays
    /// authoritative.
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
    /// `advance_to` target, named by the target's `seed_lane`.
    advance_to_seed_lane: Option<&'static str>,
    /// Step agent pin (`None` = project default). Encoded like the
    /// ADR-0032 per-issue overrides: provider registry id + bare Claude
    /// model alias.
    step_provider: Option<&'static str>,
    step_model: Option<&'static str>,
}

/// The seeded default board (ADR-0037 item 8): Backlog · Ready · Quick ·
/// In Progress · In Review · Done. In Progress carries today's dispatch
/// semantics (`on_enter: run`) and the E17-03 auto-flip
/// (`on_settle: advance` into the next column) on the project-default
/// agent; Quick is the gateless small-work lane, pinned to the cheap
/// agent (design handoff v3: claude · haiku) and advancing straight to
/// Done (ADR item 4 — without the target a Quick card would walk into In
/// Progress and fire a second unconfirmed dispatch); In Review is the
/// human gate; Done counts as done; Backlog lands imports.
const SEED_COLUMNS: &[SeedColumn] = &[
    SeedColumn {
        name: "Backlog",
        kind: ColumnKind::Shelf,
        counts_as_done: false,
        is_landing: true,
        on_enter: OnEnter::Queue,
        on_settle: OnSettle::Hold,
        seed_lane: Some("backlog"),
        advance_to_seed_lane: None,
        step_provider: None,
        step_model: None,
    },
    SeedColumn {
        name: "Ready",
        kind: ColumnKind::Shelf,
        counts_as_done: false,
        is_landing: false,
        on_enter: OnEnter::Queue,
        on_settle: OnSettle::Hold,
        seed_lane: Some("ready"),
        advance_to_seed_lane: None,
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
        advance_to_seed_lane: Some("done"),
        step_provider: Some("claude"),
        step_model: Some("haiku"),
    },
    SeedColumn {
        name: "In Progress",
        kind: ColumnKind::AgentStep,
        counts_as_done: false,
        is_landing: false,
        on_enter: OnEnter::Run,
        on_settle: OnSettle::Advance,
        seed_lane: Some("in_progress"),
        advance_to_seed_lane: None,
        step_provider: None,
        step_model: None,
    },
    SeedColumn {
        name: "In Review",
        kind: ColumnKind::HumanGate,
        counts_as_done: false,
        is_landing: false,
        on_enter: OnEnter::Queue,
        on_settle: OnSettle::Hold,
        seed_lane: Some("in_review"),
        advance_to_seed_lane: None,
        step_provider: None,
        step_model: None,
    },
    SeedColumn {
        name: "Done",
        kind: ColumnKind::Shelf,
        counts_as_done: true,
        is_landing: false,
        on_enter: OnEnter::Queue,
        on_settle: OnSettle::Hold,
        seed_lane: Some("done"),
        advance_to_seed_lane: None,
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
    let mut id_by_seed_lane: Vec<(&'static str, String)> = Vec::new();
    let mut pending_targets: Vec<(String, &'static str)> = Vec::new();
    for (position, seed) in SEED_COLUMNS.iter().enumerate() {
        let id = format!("col_{}", uuid::Uuid::new_v4());
        conn.execute(
            "INSERT INTO board_columns
                 (id, project_id, name, position, kind, counts_as_done,
                  is_landing, on_enter, on_settle, seed_lane,
                  step_provider, step_model)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
            ],
        )?;
        if let Some(seed_lane) = seed.seed_lane {
            id_by_seed_lane.push((seed_lane, id.clone()));
        }
        if let Some(target) = seed.advance_to_seed_lane {
            pending_targets.push((id, target));
        }
    }
    // Second pass: wire advance_to targets (Quick → Done) once every
    // target row exists.
    for (id, target_seed_lane) in pending_targets {
        let target_id = id_by_seed_lane
            .iter()
            .find(|(lane, _)| *lane == target_seed_lane)
            .map(|(_, id)| id.clone())
            .ok_or_else(|| {
                Error::Internal(format!(
                    "seed advance_to target '{target_seed_lane}' missing from SEED_COLUMNS"
                ))
            })?;
        conn.execute(
            "UPDATE board_columns SET advance_to = ?2 WHERE id = ?1",
            rusqlite::params![id, target_id],
        )?;
    }
    Ok(())
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
    /// Rejected when the column is occupied ([`Error::BoardColumnHasIssues`])
    /// or when it is the landing column (move the flag first — the board
    /// always has exactly one).
    ///
    /// Occupancy derives from the AUTHORITATIVE signal per column:
    /// - seeded column (`seed_lane` set): any project issue whose `lane`
    ///   equals `seed_lane`. The `column_id` mirror is deliberately ignored
    ///   here — nothing maintains it after the 0006 backfill, so a stale
    ///   pointer must not block a truly empty column (the FK's
    ///   `ON DELETE SET NULL` clears it harmlessly), and a fresh
    ///   NULL-pointer issue cannot escape the lane check;
    /// - non-seeded column (`seed_lane` NULL — Quick, user-created): no
    ///   lane can express membership, so any `column_id` pointer counts.
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
        let issue_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM issues
              WHERE (?2 IS NOT NULL AND project_id = ?3 AND lane = ?2)
                 OR (?2 IS NULL AND column_id = ?1)",
            rusqlite::params![id, column.seed_lane, column.project_id],
            |row| row.get(0),
        )?;
        if issue_count > 0 {
            return Err(Error::BoardColumnHasIssues {
                id: id.into(),
                count: issue_count,
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
            let mut stmt =
                conn.prepare("SELECT id FROM board_columns WHERE project_id = ?1")?;
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
                ("Backlog", ColumnKind::Shelf, false, true, OnEnter::Queue, OnSettle::Hold),
                ("Ready", ColumnKind::Shelf, false, false, OnEnter::Queue, OnSettle::Hold),
                ("Quick", ColumnKind::AgentStep, false, false, OnEnter::Run, OnSettle::Advance),
                ("In Progress", ColumnKind::AgentStep, false, false, OnEnter::Run, OnSettle::Advance),
                ("In Review", ColumnKind::HumanGate, false, false, OnEnter::Queue, OnSettle::Hold),
                ("Done", ColumnKind::Shelf, true, false, OnEnter::Queue, OnSettle::Hold),
            ]
        );
        // Quick advances straight to Done (ADR-0037 item 4); In Progress
        // advances to the NEXT column (advance_to NULL).
        let done = &columns[5];
        assert_eq!(columns[2].advance_to.as_deref(), Some(done.id.as_str()));
        assert_eq!(columns[3].advance_to, None);
        // Quick is pinned to the cheap agent (design handoff v3:
        // claude · haiku); In Progress stays NULL = project default.
        assert_eq!(columns[2].step_provider.as_deref(), Some("claude"));
        assert_eq!(columns[2].step_model.as_deref(), Some("haiku"));
        assert_eq!(columns[3].step_provider, None);
        assert_eq!(columns[3].step_model, None);
        // seed_lane mirrors the five legacy lanes; Quick has none.
        let seed_lanes: Vec<Option<&str>> =
            columns.iter().map(|c| c.seed_lane.as_deref()).collect();
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
        // Positions are compact board order; ids carry the col_ prefix.
        let positions: Vec<i64> = columns.iter().map(|c| c.position).collect();
        assert_eq!(positions, vec![0, 1, 2, 3, 4, 5]);
        assert!(columns.iter().all(|c| c.id.starts_with("col_")));
        // Idempotent: a second seed call leaves the board alone.
        seed_default_columns(&store.conn().unwrap(), "p1").unwrap();
        assert_eq!(store.list_for_project("p1").unwrap().len(), 6);
        // Project scoping: p2 was never seeded.
        assert!(store.list_for_project("p2").unwrap().is_empty());
    }

    /// Integration test of migration 0006 against a database that already
    /// has projects and laned issues: replays migrations 0000–0005 on a raw
    /// connection, inserts pre-columns data, applies 0006, and checks the
    /// seed (incl. Quick's advance_to and the seed_lane markers) + the
    /// `issues.column_id` backfill.
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
                .query_map([project], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let names: Vec<&str> = rows.iter().map(|(n, _, _)| n.as_str()).collect();
            assert_eq!(
                names,
                vec!["Backlog", "Ready", "Quick", "In Progress", "In Review", "Done"]
            );
            let seed_lanes: Vec<Option<&str>> =
                rows.iter().map(|(_, l, _)| l.as_deref()).collect();
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
            assert_eq!(rows[3].2, None, "In Progress advances to next column");
            // Quick is pinned to claude · haiku; In Progress keeps the
            // project default (NULL provider/model).
            let step_pins: Vec<(String, Option<String>, Option<String>)> = conn
                .prepare(
                    "SELECT name, step_provider, step_model FROM board_columns
                      WHERE project_id = ?1 AND name IN ('Quick', 'In Progress')
                      ORDER BY position",
                )
                .unwrap()
                .query_map([project], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
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
        assert_eq!(issue_column("i1"), ("backlog".into(), "Backlog".into(), "p1".into()));
        assert_eq!(issue_column("i2"), ("in_progress".into(), "In Progress".into(), "p1".into()));
        assert_eq!(issue_column("i3"), ("done".into(), "Done".into(), "p1".into()));
        assert_eq!(issue_column("i4"), ("in_progress".into(), "In Progress".into(), "p2".into()));
    }

    #[test]
    fn create_appends_validates_and_round_trips_step_config() {
        let store = seeded_fixture();
        let done_id = store.list_for_project("p1").unwrap()[5].id.clone();
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
        assert_eq!(created.position, 6); // appended after the seeded six
        assert_eq!(created.kind, ColumnKind::AgentStep);
        assert_eq!(created.on_enter, OnEnter::Run);
        assert_eq!(created.on_settle, OnSettle::Advance);
        assert_eq!(created.advance_to.as_deref(), Some(done_id.as_str()));
        assert_eq!(created.step_prompt.as_deref(), Some("Grill me on the plan."));
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
        assert_eq!(retargeted.advance_to.as_deref(), Some(in_review.id.as_str()));
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

        // Deleting a target column degrades referrers to next-column (FK
        // ON DELETE SET NULL), never dangles.
        let target = store.create(new_column("Target")).unwrap();
        let pointer = store
            .create(NewColumn {
                advance_to: Some(target.id.clone()),
                ..new_column("Pointer")
            })
            .unwrap();
        store.delete(&target.id).unwrap();
        assert!(store.get(&pointer.id).unwrap().unwrap().advance_to.is_none());
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

    #[test]
    fn delete_guard_derives_from_the_authoritative_lane() {
        let store = seeded_fixture();
        let issues = issue_store(&store);
        let ready = store.list_for_project("p1").unwrap()[1].clone();
        assert_eq!(ready.seed_lane.as_deref(), Some("ready"));

        // Reproduced scenario (a): a FRESH issue (column_id NULL — nothing
        // maintains the mirror) moved to Ready via move_to must BLOCK the
        // delete. The old column_id-count guard false-allowed this.
        let fresh = issues.create(new_issue("fresh")).unwrap();
        issues.move_to(&fresh.id, Lane::Ready, None).unwrap();
        let mirror: Option<String> = store
            .conn()
            .unwrap()
            .query_row(
                "SELECT column_id FROM issues WHERE id = ?1",
                [&fresh.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(mirror.is_none(), "precondition: the mirror is unmaintained");
        assert!(matches!(
            store.delete(&ready.id),
            Err(Error::BoardColumnHasIssues { ref id, count: 1 }) if *id == ready.id
        ));

        // Reproduced scenario (b): a BACKFILLED issue (column_id = Ready)
        // later moved lane-wise to Done leaves a stale pointer; deleting
        // the now-empty Ready column must SUCCEED. The old guard
        // false-blocked on the stale mirror.
        issues.move_to(&fresh.id, Lane::Done, None).unwrap(); // drain (a)
        let backfilled = issues.create(new_issue("backfilled")).unwrap();
        issues.move_to(&backfilled.id, Lane::Ready, None).unwrap();
        store
            .conn()
            .unwrap()
            .execute(
                "UPDATE issues SET column_id = ?1 WHERE id = ?2",
                rusqlite::params![ready.id, backfilled.id],
            )
            .unwrap(); // simulate the 0006 backfill
        issues.move_to(&backfilled.id, Lane::Done, None).unwrap();
        store.delete(&ready.id).unwrap();
        // The stale pointer was cleared by the FK (SET NULL), not orphaned,
        // and the remaining positions compacted to 0..n-1.
        let mirror: Option<String> = store
            .conn()
            .unwrap()
            .query_row(
                "SELECT column_id FROM issues WHERE id = ?1",
                [&backfilled.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(mirror.is_none());
        let after = store.list_for_project("p1").unwrap();
        let names: Vec<&str> = after.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["Backlog", "Quick", "In Progress", "In Review", "Done"]);
        let positions: Vec<i64> = after.iter().map(|c| c.position).collect();
        assert_eq!(positions, vec![0, 1, 2, 3, 4]);

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
            vec!["Done", "In Review", "In Progress", "Quick", "Ready", "Backlog"]
        );
        let positions: Vec<i64> = after.iter().map(|c| c.position).collect();
        assert_eq!(positions, vec![0, 1, 2, 3, 4, 5]);

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

    #[test]
    fn project_delete_cascades_columns() {
        let store = seeded_fixture();
        assert_eq!(store.list_for_project("p1").unwrap().len(), 6);
        store
            .conn()
            .unwrap()
            .execute("DELETE FROM projects WHERE id = 'p1'", [])
            .unwrap();
        assert!(store.list_for_project("p1").unwrap().is_empty());
    }
}
