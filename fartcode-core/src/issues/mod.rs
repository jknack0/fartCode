//! Project board issues (E17-01, #55; ARCHITECTURE.md §13, ADR-0032).
//!
//! Local-first issue store backing the project board: five lanes
//! (backlog → ready → in_progress → in_review → done), blocked-by edges
//! between issues, and a linked-task pointer the dispatch engine (E17-03)
//! fills when a card spawns an implementation agent.
//!
//! Invariants enforced here:
//! - **Blocked status is derived, never stored** (ADR-0032; flag-keyed by
//!   E18-03/ADR-0037 item 6): an issue is blocked iff any direct blocker
//!   sits outside a `counts_as_done` column — the blocker's column resolved
//!   through the `seed_lane` mapping while lane stays authoritative. A
//!   blocker landing in a counts-as-done column (seeded Done, or any later
//!   terminal column) unblocks its dependents with no writes.
//! - **Cycles are rejected at edge creation** — a `blocked by` edge that
//!   would close a loop (including a self-edge) fails with
//!   [`Error::IssueDependencyCycle`]. Cross-project edges are rejected:
//!   board lanes and blocked semantics are project-local.
//! - Deleting an issue cascades its edges (FK `ON DELETE CASCADE` on both
//!   endpoints); deleting a task clears `linked_task_id` (`ON DELETE SET
//!   NULL`) so a card survives its task's teardown.
//!
//! Schema: migration 0002.

/// Configurable pipeline columns (E18, ADR-0037) — spike behind the seeded
/// default; `lane` above stays authoritative.
pub mod columns;

use std::collections::HashMap;
use std::sync::Arc;

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::db::{parse_versioned, serialize_versioned, Db, Versioned};
use crate::events::{EventBus, InternalEvent};
use crate::Error;

/// Board lanes (§13). Text values are the stored representation; board
/// column order is the `CASE` in `list_for_project` (text sort is NOT lane
/// order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    Backlog,
    Ready,
    InProgress,
    InReview,
    Done,
}

impl Lane {
    pub fn as_str(&self) -> &'static str {
        match self {
            Lane::Backlog => "backlog",
            Lane::Ready => "ready",
            Lane::InProgress => "in_progress",
            Lane::InReview => "in_review",
            Lane::Done => "done",
        }
    }

    pub fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "backlog" => Ok(Lane::Backlog),
            "ready" => Ok(Lane::Ready),
            "in_progress" => Ok(Lane::InProgress),
            "in_review" => Ok(Lane::InReview),
            "done" => Ok(Lane::Done),
            other => Err(Error::InvalidIssueInput(format!(
                "invalid lane: {other:?} (expected backlog|ready|in_progress|in_review|done)"
            ))),
        }
    }
}

/// Versioned payload for the `issues.acceptance` column
/// (`{"version":1,"data":{"items":[...]}}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcceptanceCriteria {
    #[serde(default)]
    pub items: Vec<String>,
}

impl Versioned for AcceptanceCriteria {
    const VERSION: u32 = 1;
}

/// Blocker summary for the board badge + hover list.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockerRef {
    pub id: String,
    pub title: String,
    pub lane: Lane,
    /// True when the blocker's column carries `counts_as_done` (E18-03,
    /// ADR-0037 item 6) — the flag every "is it finished?" consumer keys
    /// off instead of the `'done'` lane string.
    pub counts_as_done: bool,
    /// The blocker's effective column. `lane` cannot name a non-seeded
    /// column, so a blocker parked in Quick used to be reported as being
    /// in the column its stale lane pointed at — the same panel naming two
    /// different columns for one card. `None` only when the card's lane
    /// maps to no column at all.
    pub column_id: Option<String>,
}

/// An issue row with its derived board state.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub body: Option<String>,
    pub acceptance: Vec<String>,
    pub lane: Lane,
    pub position: i64,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub prd_path: Option<String>,
    pub prd_section: Option<String>,
    /// Task spawned by board dispatch (E17-03). Survives lane moves;
    /// cleared when the task is deleted.
    pub linked_task_id: Option<String>,
    /// Source URL when imported from an external tracker (GitHub issue
    /// import — dedupe key + provenance badge).
    pub external_ref: Option<String>,
    /// Mirror pointer to the card's board column (E18-04): written by
    /// [`IssueStore::enter_column`] (which lane-addressed moves route
    /// through) and the 0006 backfill. Lane stays authoritative until the
    /// E18-07 flip; `None` on cards that have never been moved/entered.
    pub column_id: Option<String>,
    /// Derived at read time (ADR-0032): any direct blocker not in a
    /// counts-as-done column.
    pub blocked: bool,
    pub blockers: Vec<BlockerRef>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Fields for [`IssueStore::create`].
pub struct NewIssue {
    pub project_id: String,
    pub title: String,
    pub body: Option<String>,
    pub acceptance: Vec<String>,
    /// Overrides the landing target. `None` (every user-facing entry
    /// path — import, proposal apply, manual add) lands the card on the
    /// project's `is_landing` column (E18-06).
    pub lane: Option<Lane>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub prd_path: Option<String>,
    pub prd_section: Option<String>,
    pub external_ref: Option<String>,
}

/// Patch for [`IssueStore::update`]: `None` leaves the field alone;
/// `Some(None)` clears a nullable field; `Some(Some(v))` sets it.
/// `title` and `acceptance` are non-nullable (`Some("")` titles rejected).
#[derive(Debug, Default)]
pub struct IssuePatch {
    pub title: Option<String>,
    pub body: Option<Option<String>>,
    pub acceptance: Option<Vec<String>>,
    pub provider: Option<Option<String>>,
    pub model: Option<Option<String>>,
    pub prd_path: Option<Option<String>>,
    pub prd_section: Option<Option<String>>,
}

/// Builds the dispatch prompt packet (E17-03, #57; ADR-0032) — issue
/// title, body, acceptance criteria, PRD by reference (the agent reads the
/// file), one-line finished-blocker notes (blockers whose column carries
/// `counts_as_done`, E18-03/ADR-0037 item 6), and the branch/worktree
/// footer. Empty sections are omitted entirely.
pub fn build_dispatch_prompt(issue: &Issue, finished_blocker_titles: &[String]) -> String {
    let mut out = String::from(
        "You are implementing an issue from the project board.\n\n\
         # Issue\n",
    );
    out.push_str(&issue.title);
    out.push('\n');
    if let Some(body) = issue
        .body
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
    {
        out.push('\n');
        out.push_str(body);
        out.push('\n');
    }
    if !issue.acceptance.is_empty() {
        out.push_str("\n# Acceptance criteria\n");
        for ac in &issue.acceptance {
            out.push_str("- ");
            out.push_str(ac);
            out.push('\n');
        }
    }
    let has_prd = issue.prd_path.is_some();
    if has_prd || !finished_blocker_titles.is_empty() {
        out.push_str("\n# Context\n");
        if let Some(path) = &issue.prd_path {
            out.push_str("- PRD: ");
            out.push_str(path);
            if let Some(section) = &issue.prd_section {
                out.push_str(" (section: ");
                out.push_str(section);
                out.push(')');
            }
            out.push_str(" — read it before starting.\n");
        }
        // Honest wording (ADR-0037 item 6): the board flag says the card is
        // finished; it does not by itself prove a merge landed.
        for title in finished_blocker_titles {
            out.push_str("- Dependency \"");
            out.push_str(title);
            out.push_str(
                "\" is marked finished on the board — expect its work on the \
                 default branch (verify if your change depends on it).\n",
            );
        }
    }
    out.push_str(
        "\n# Conventions\n\
         - You are on a dedicated branch in a git worktree of the project — stay on it.\n\
         - Commit your changes as you go; leave the worktree clean when finished.\n",
    );
    out
}

pub struct IssueStore {
    db: Arc<dyn Db>,
    event_bus: Arc<dyn EventBus>,
}

/// Columns in `issue_from_row` order; the final `blocked` slot is the
/// EXISTS subquery appended by the list/get SELECTs.
const COLUMNS: &str = "id, project_id, title, body, acceptance, lane, position, \
     provider, model, prd_path, prd_section, linked_task_id, external_ref, \
     column_id, created_at, updated_at";

/// THE definition of column membership on the read side: `board_columns c`
/// is the effective column of issue row `b`.
///
/// Mirror FIRST, seeded lane as the fallback for a card that has never
/// moved. Lane alone cannot express a non-seeded column — `enter_column`
/// leaves `lane` untouched for Quick and user-created columns — so a
/// lane-only join answered for the column the card used to be in. That was
/// invisible while the board rendered five fixed lanes and became wrong on
/// screen the moment it rendered every column: dragging a finished card
/// out of Done into Quick left it resolving to Done, so its dependents
/// stayed unblocked and their agents were told the work had landed.
///
/// `issues.column_id` is `ON DELETE SET NULL` (migration 0006), so a
/// mirror is either valid or absent, never dangling. A card whose lane
/// maps to no column at all matches nothing and therefore counts as
/// UNFINISHED — failing toward blocked, the safe direction.
const BLOCKER_COLUMN_MATCH: &str = "c.project_id = b.project_id \
     AND (c.id = b.column_id OR (b.column_id IS NULL AND c.seed_lane = b.lane))";

/// Derived-blocked subquery (E18-03, #77; ADR-0037 item 6): true when any
/// direct blocker is NOT in a `counts_as_done` column. Membership comes
/// from [`BLOCKER_COLUMN_MATCH`] — the same resolution `attach_blockers`
/// uses, so the flag and the badge can never disagree.
fn blocked_sql() -> String {
    format!(
        "EXISTS(SELECT 1 FROM issue_dependencies d \
         JOIN issues b ON b.id = d.blocked_by_id \
         WHERE d.issue_id = issues.id \
           AND NOT EXISTS(SELECT 1 FROM board_columns c \
                WHERE {BLOCKER_COLUMN_MATCH} \
                  AND c.counts_as_done = 1))"
    )
}

/// SQL expression for the effective column id of issue row `<alias>` —
/// the write-side twin of [`BLOCKER_COLUMN_MATCH`], used to group a
/// column's cards for renumbering.
fn effective_column_expr(alias: &str) -> String {
    format!(
        "COALESCE({alias}.column_id, \
          (SELECT c2.id FROM board_columns c2 \
            WHERE c2.project_id = {alias}.project_id \
              AND c2.seed_lane = {alias}.lane))"
    )
}

/// Renumbers one column's cards to a dense 0..n-1, optionally placing
/// `moved_id` at index `at` (`None` = append; pass `moved_id: None` to
/// simply compact a column a card just left).
///
/// Board positions are single-row absolute writes with no ordering
/// constraint, so "drop at index 0" used to leave the dropped card holding
/// the same position as the card already there — and a stable sort then
/// rendered it second, one slot below where it was let go. The namespace
/// was wrong as well as the arithmetic: positions were allocated per LANE
/// while the board renders per COLUMN, so two cards in a non-seeded column
/// could both hold position 0 in a lane neither of them appears in, with
/// `created_at` (second resolution) breaking the tie unpredictably.
/// Renumbering the destination group — resolved exactly the way the board
/// resolves membership — is what makes a drop land where it was dropped.
fn renumber_column(
    conn: &rusqlite::Connection,
    project_id: &str,
    column_id: &str,
    moved_id: Option<&str>,
    at: Option<i64>,
) -> Result<(), Error> {
    let mut ids: Vec<String> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT i.id FROM issues i \
              WHERE i.project_id = ?1 AND {expr} = ?2 \
              ORDER BY i.position, i.created_at, i.id",
            expr = effective_column_expr("i"),
        ))?;
        let rows = stmt.query_map(rusqlite::params![project_id, column_id], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    if let Some(moved) = moved_id {
        ids.retain(|id| id != moved);
        let at = at.unwrap_or(ids.len() as i64).clamp(0, ids.len() as i64) as usize;
        ids.insert(at, moved.to_string());
    }
    for (position, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE issues SET position = ?2 WHERE id = ?1",
            rusqlite::params![id, position as i64],
        )?;
    }
    Ok(())
}

/// The issue's effective column — mirror first, seeded lane otherwise.
fn effective_column_of(
    conn: &rusqlite::Connection,
    issue: &Issue,
) -> Result<Option<String>, Error> {
    match &issue.column_id {
        Some(column_id) => Ok(Some(column_id.clone())),
        None => seeded_column_for_lane(conn, &issue.project_id, issue.lane),
    }
}

fn issue_from_row(row: &rusqlite::Row) -> rusqlite::Result<Issue> {
    let lane: String = row.get(5)?;
    let acceptance_cell: Option<String> = row.get(4)?;
    let blocked: i64 = row.get(16)?;
    Ok(Issue {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        body: row.get(3)?,
        acceptance: parse_versioned::<AcceptanceCriteria>(
            "issues.acceptance",
            acceptance_cell.as_deref(),
        )
        .map(|a| a.items)
        .unwrap_or_default(),
        lane: Lane::parse(&lane).unwrap_or(Lane::Backlog),
        position: row.get(6)?,
        provider: row.get(7)?,
        model: row.get(8)?,
        prd_path: row.get(9)?,
        prd_section: row.get(10)?,
        linked_task_id: row.get(11)?,
        external_ref: row.get(12)?,
        column_id: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
        blocked: blocked != 0,
        blockers: Vec::new(), // attached by the caller (second query)
    })
}

/// The seeded column mirroring `lane` in this project, if one exists
/// (a user may have deleted it). Shared by the lane-addressed move
/// routing and new-card entry resolution.
fn seeded_column_for_lane(
    conn: &rusqlite::Connection,
    project_id: &str,
    lane: Lane,
) -> Result<Option<String>, Error> {
    Ok(conn
        .query_row(
            "SELECT id FROM board_columns WHERE project_id = ?1 AND seed_lane = ?2",
            rusqlite::params![project_id, lane.as_str()],
            |row| row.get(0),
        )
        .optional()?)
}

/// Board target for a NEW card (E18-06, #65): every entry path lands on
/// the project's `is_landing` column instead of hardcoding Backlog.
///
/// An explicit lane override maps to that lane's seeded column; with no
/// override the landing column decides — its `seed_lane` gives the lane,
/// and a NON-seeded landing column falls back to [`Lane::Backlog`]
/// because lane cannot represent it (interim until the E18-07 authority
/// flip; the mirror still points at the real landing column). A project
/// with no board at all (no columns) keeps the legacy Backlog default
/// with a NULL mirror.
fn resolve_entry_target(
    conn: &rusqlite::Connection,
    project_id: &str,
    lane: Option<Lane>,
) -> Result<(Lane, Option<String>), Error> {
    if let Some(lane) = lane {
        return Ok((lane, seeded_column_for_lane(conn, project_id, lane)?));
    }
    let landing: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT id, seed_lane FROM board_columns
              WHERE project_id = ?1 AND is_landing = 1",
            [project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match landing {
        Some((column_id, seed_lane)) => {
            let lane = seed_lane
                .as_deref()
                .map(Lane::parse)
                .transpose()?
                .unwrap_or(Lane::Backlog);
            Ok((lane, Some(column_id)))
        }
        None => Ok((Lane::Backlog, None)),
    }
}

/// Would adding `from` blocked-by `to` close a cycle? `edges` maps each
/// issue to its direct blockers. Cycle iff `from` is reachable from `to`
/// following existing edges (self-edge is the trivial case).
fn would_cycle(edges: &HashMap<String, Vec<String>>, from: &str, to: &str) -> bool {
    if from == to {
        return true;
    }
    let mut stack = vec![to.to_string()];
    let mut seen = std::collections::HashSet::new();
    while let Some(node) = stack.pop() {
        if node == from {
            return true;
        }
        if seen.insert(node.clone()) {
            if let Some(blockers) = edges.get(&node) {
                stack.extend(blockers.iter().cloned());
            }
        }
    }
    false
}

impl IssueStore {
    pub fn new(db: Arc<dyn Db>, event_bus: Arc<dyn EventBus>) -> Self {
        Self { db, event_bus }
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>, Error> {
        self.db
            .conn()
            .lock()
            .map_err(|e| Error::Internal(format!("db mutex poisoned: {e}")))
    }

    /// The shared handle, for siblings that need a companion store on the
    /// same database (e.g. resolving board columns).
    pub fn db(&self) -> Arc<dyn Db> {
        self.db.clone()
    }

    /// Issues whose dispatch link points at this task (E17-03 auto-flip).
    pub fn list_by_linked_task(&self, task_id: &str) -> Result<Vec<Issue>, Error> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS}, {blocked} FROM issues WHERE linked_task_id = ?1",
            blocked = blocked_sql()
        ))?;
        let mut issues = stmt
            .query_map([task_id], issue_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        attach_blockers(&conn, &mut issues)?;
        Ok(issues)
    }

    /// Creates an issue on the project's LANDING column (E18-06, #65) —
    /// GitHub import, PM proposal apply, and manual add all arrive here,
    /// so moving the `is_landing` flag moves every entry path with it.
    /// `new.lane` overrides the landing target (internal/legacy callers).
    /// The card carries its column mirror from birth. Appended to the end
    /// of the resolved lane. Emits `IssueCreated`.
    pub fn create(&self, new: NewIssue) -> Result<Issue, Error> {
        let title = new.title.trim().to_string();
        if title.is_empty() {
            return Err(Error::InvalidIssueInput("title is empty".into()));
        }
        let acceptance = serialize_versioned(&AcceptanceCriteria {
            items: new.acceptance,
        })?;
        let id = format!("iss_{}", uuid::Uuid::new_v4());
        let mut deduped_id: Option<String> = None;
        let lane;
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
            let column_id;
            (lane, column_id) = resolve_entry_target(&conn, &new.project_id, new.lane)?;
            // One board card per external issue: importing the same GitHub
            // issue twice returns the existing card instead of a duplicate.
            // (Resolved inside the guard block; returned after it drops so
            // `get` can take the lock itself.)
            if let Some(external_ref) = &new.external_ref {
                deduped_id = conn
                    .query_row(
                        "SELECT id FROM issues WHERE project_id = ?1 AND external_ref = ?2",
                        rusqlite::params![new.project_id, external_ref],
                        |row| row.get(0),
                    )
                    .optional()?;
            }
            if deduped_id.is_none() {
                conn.execute(
                    "INSERT INTO issues
                         (id, project_id, title, body, acceptance, lane, position,
                          provider, model, prd_path, prd_section, external_ref,
                          column_id)
                     VALUES (
                         ?1, ?2, ?3, ?4, ?5, ?6,
                         (SELECT COALESCE(MAX(position) + 1, 0) FROM issues
                           WHERE project_id = ?2 AND lane = ?6),
                         ?7, ?8, ?9, ?10, ?11, ?12
                     )",
                    rusqlite::params![
                        id,
                        new.project_id,
                        title,
                        new.body,
                        acceptance,
                        lane.as_str(),
                        new.provider,
                        new.model,
                        new.prd_path,
                        new.prd_section,
                        new.external_ref,
                        column_id,
                    ],
                )?;
            }
        }
        if let Some(existing_id) = deduped_id {
            return self
                .get(&existing_id)?
                .ok_or_else(|| Error::Internal("deduped issue vanished".into()));
        }
        self.event_bus.send(InternalEvent::IssueCreated {
            id: id.clone(),
            project_id: new.project_id.clone(),
            title,
        });
        self.get(&id)?
            .ok_or_else(|| Error::Internal(format!("issue vanished after insert: {id}")))
    }

    pub fn get(&self, id: &str) -> Result<Option<Issue>, Error> {
        let conn = self.conn()?;
        let mut issue: Issue = match conn
            .query_row(
                &format!(
                    "SELECT {COLUMNS}, {blocked} FROM issues WHERE id = ?1",
                    blocked = blocked_sql()
                ),
                [id],
                issue_from_row,
            )
            .optional()?
        {
            Some(issue) => issue,
            None => return Ok(None),
        };
        attach_blockers(&conn, std::slice::from_mut(&mut issue))?;
        Ok(Some(issue))
    }

    /// All issues for a project, ordered by lane rank then position
    /// (board render order).
    pub fn list_for_project(&self, project_id: &str) -> Result<Vec<Issue>, Error> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS}, {blocked} FROM issues
              WHERE project_id = ?1
              ORDER BY CASE lane
                           WHEN 'backlog' THEN 0 WHEN 'ready' THEN 1
                           WHEN 'in_progress' THEN 2 WHEN 'in_review' THEN 3
                           WHEN 'done' THEN 4 ELSE 5
                       END, position, created_at",
            blocked = blocked_sql()
        ))?;
        let mut issues = stmt
            .query_map([project_id], issue_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        attach_blockers(&conn, &mut issues)?;
        Ok(issues)
    }

    /// Applies a field patch. Emits `IssueUpdated`.
    pub fn update(&self, id: &str, patch: IssuePatch) -> Result<Issue, Error> {
        let mut issue = self
            .get(id)?
            .ok_or_else(|| Error::IssueNotFound(id.into()))?;
        if let Some(title) = patch.title {
            let title = title.trim().to_string();
            if title.is_empty() {
                return Err(Error::InvalidIssueInput("title is empty".into()));
            }
            issue.title = title;
        }
        if let Some(body) = patch.body {
            issue.body = body;
        }
        if let Some(items) = patch.acceptance {
            issue.acceptance = items;
        }
        if let Some(provider) = patch.provider {
            issue.provider = provider;
        }
        if let Some(model) = patch.model {
            issue.model = model;
        }
        if let Some(prd_path) = patch.prd_path {
            issue.prd_path = prd_path;
        }
        if let Some(prd_section) = patch.prd_section {
            issue.prd_section = prd_section;
        }
        let acceptance = serialize_versioned(&AcceptanceCriteria {
            items: issue.acceptance.clone(),
        })?;
        {
            let conn = self.conn()?;
            conn.execute(
                "UPDATE issues SET title = ?2, body = ?3, acceptance = ?4,
                     provider = ?5, model = ?6, prd_path = ?7, prd_section = ?8,
                     updated_at = datetime('now')
                  WHERE id = ?1",
                rusqlite::params![
                    id,
                    issue.title,
                    issue.body,
                    acceptance,
                    issue.provider,
                    issue.model,
                    issue.prd_path,
                    issue.prd_section,
                ],
            )?;
        }
        self.event_bus.send(InternalEvent::IssueUpdated {
            id: id.into(),
            project_id: issue.project_id.clone(),
        });
        self.get(id)?
            .ok_or_else(|| Error::Internal(format!("issue vanished after update: {id}")))
    }

    /// Moves an issue to a lane. `position: None` appends to the end of the
    /// target lane. Any lane transition is allowed — blocked-dispatch
    /// confirmation is a UI concern (ADR-0032) and derived blocked state
    /// needs no maintenance writes. Emits `IssueUpdated`.
    ///
    /// E18-04: when the target lane maps to a seeded column, the move
    /// routes through [`Self::enter_column`] — one code path, and the
    /// mirror pointer stays maintained. When the lane has no seeded column
    /// (the user deleted it), the legacy lane-only write runs and the
    /// mirror is cleared: no column can represent that lane.
    pub fn move_to(&self, id: &str, lane: Lane, position: Option<i64>) -> Result<Issue, Error> {
        let issue = self
            .get(id)?
            .ok_or_else(|| Error::IssueNotFound(id.into()))?;
        // Same-lane REORDER (fix round, finding 13): position only — the
        // mirror is NOT touched. A card resident in a non-seeded column
        // (Quick) still renders in its stale lane on the legacy board;
        // reordering it there must not clobber the column residence.
        if issue.lane == lane {
            {
                let conn = self.conn()?;
                // Renumber the card's effective COLUMN, not its lane: this
                // is the board's within-column reorder, and `position` is
                // the destination index AFTER the card is lifted out. A
                // card resident in a non-seeded column reorders among the
                // cards it is displayed with, not among its stale lane.
                match effective_column_of(&conn, &issue)? {
                    Some(column_id) => {
                        renumber_column(&conn, &issue.project_id, &column_id, Some(id), position)?;
                        conn.execute(
                            "UPDATE issues SET updated_at = datetime('now') WHERE id = ?1",
                            rusqlite::params![id],
                        )?;
                    }
                    // No column can represent this lane (the user deleted
                    // the seeded one) — legacy absolute write.
                    None => {
                        let position = match position {
                            Some(p) => p,
                            None => conn.query_row(
                                "SELECT COALESCE(MAX(position) + 1, 0) FROM issues
                                  WHERE project_id = ?1 AND lane = ?2",
                                rusqlite::params![issue.project_id, lane.as_str()],
                                |row| row.get(0),
                            )?,
                        };
                        conn.execute(
                            "UPDATE issues SET position = ?2, updated_at = datetime('now')
                              WHERE id = ?1",
                            rusqlite::params![id, position],
                        )?;
                    }
                }
            }
            self.event_bus.send(InternalEvent::IssueUpdated {
                id: id.into(),
                project_id: issue.project_id.clone(),
            });
            return self
                .get(id)?
                .ok_or_else(|| Error::Internal(format!("issue vanished after move: {id}")));
        }
        let seeded_column: Option<String> = {
            let conn = self.conn()?;
            seeded_column_for_lane(&conn, &issue.project_id, lane)?
        };
        if let Some(column_id) = seeded_column {
            return self.enter_column(id, &column_id, position);
        }
        {
            let conn = self.conn()?;
            let position = match position {
                Some(p) => p,
                None => conn.query_row(
                    "SELECT COALESCE(MAX(position) + 1, 0) FROM issues
                      WHERE project_id = ?1 AND lane = ?2",
                    rusqlite::params![issue.project_id, lane.as_str()],
                    |row| row.get(0),
                )?,
            };
            conn.execute(
                "UPDATE issues SET lane = ?2, position = ?3, column_id = NULL,
                     updated_at = datetime('now')
                  WHERE id = ?1",
                rusqlite::params![id, lane.as_str(), position],
            )?;
        }
        self.event_bus.send(InternalEvent::IssueUpdated {
            id: id.into(),
            project_id: issue.project_id.clone(),
        });
        self.get(id)?
            .ok_or_else(|| Error::Internal(format!("issue vanished after move: {id}")))
    }

    /// The step engine's move (E18-04, #63): points the card's mirror at
    /// `column_id` and, when the column mirrors a seeded lane, syncs the
    /// authoritative lane through the REVERSE seed_lane mapping. Entering
    /// a non-seeded column (Quick, user-created) leaves the lane
    /// UNCHANGED — interim until the E18-07 authority flip; the legacy
    /// lane board cannot render non-seeded columns, so the stale lane is
    /// invisible there.
    ///
    /// `position: None` appends to the end of the synced lane; when the
    /// lane does not move and no position is given, position is left
    /// alone. Cross-project entries are rejected. Emits `IssueUpdated`
    /// (exactly one — the same event a lane move fires).
    pub fn enter_column(
        &self,
        id: &str,
        column_id: &str,
        position: Option<i64>,
    ) -> Result<Issue, Error> {
        let issue = self
            .get(id)?
            .ok_or_else(|| Error::IssueNotFound(id.into()))?;
        {
            let conn = self.conn()?;
            let (column_project, seed_lane): (String, Option<String>) = conn
                .query_row(
                    "SELECT project_id, seed_lane FROM board_columns WHERE id = ?1",
                    [column_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?
                .ok_or_else(|| Error::BoardColumnNotFound(column_id.into()))?;
            if column_project != issue.project_id {
                return Err(Error::InvalidBoardColumnInput(format!(
                    "issue {id} is in project {}, column {column_id} is in project {column_project}",
                    issue.project_id
                )));
            }
            let previous_column = effective_column_of(&conn, &issue)?;
            match seed_lane.as_deref().map(Lane::parse).transpose()? {
                Some(lane) => {
                    conn.execute(
                        "UPDATE issues SET lane = ?2, column_id = ?3,
                             updated_at = datetime('now')
                          WHERE id = ?1",
                        rusqlite::params![id, lane.as_str(), column_id],
                    )?;
                }
                None => {
                    conn.execute(
                        "UPDATE issues SET column_id = ?2, updated_at = datetime('now')
                          WHERE id = ?1",
                        rusqlite::params![id, column_id],
                    )?;
                }
            }
            // Position is an INDEX IN THE DESTINATION COLUMN (after the
            // card is lifted out of it), never an absolute row value:
            // renumbering is what makes a drop land where it was dropped
            // instead of tying with whatever already held that slot.
            // `None` appends.
            renumber_column(&conn, &issue.project_id, column_id, Some(id), position)?;
            if let Some(previous) = previous_column {
                if previous != column_id {
                    renumber_column(&conn, &issue.project_id, &previous, None, None)?;
                }
            }
        }
        self.event_bus.send(InternalEvent::IssueUpdated {
            id: id.into(),
            project_id: issue.project_id.clone(),
        });
        self.get(id)?
            .ok_or_else(|| Error::Internal(format!("issue vanished after enter: {id}")))
    }

    /// Deletes an issue; its dependency edges cascade (both directions).
    /// Emits `IssueDeleted`.
    pub fn delete(&self, id: &str) -> Result<(), Error> {
        let issue = self
            .get(id)?
            .ok_or_else(|| Error::IssueNotFound(id.into()))?;
        let conn = self.conn()?;
        conn.execute("DELETE FROM issues WHERE id = ?1", [id])?;
        drop(conn);
        self.event_bus.send(InternalEvent::IssueDeleted {
            id: id.into(),
            project_id: issue.project_id,
        });
        Ok(())
    }

    /// Sets/clears the dispatch link (E17-03). `Some(task_id)` requires the
    /// task to exist. Emits `IssueUpdated`.
    pub fn set_linked_task(&self, id: &str, task_id: Option<&str>) -> Result<Issue, Error> {
        let issue = self
            .get(id)?
            .ok_or_else(|| Error::IssueNotFound(id.into()))?;
        {
            let conn = self.conn()?;
            if let Some(task_id) = task_id {
                let task_exists: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?1)",
                    [task_id],
                    |row| row.get(0),
                )?;
                if !task_exists {
                    return Err(Error::TaskNotFound(task_id.into()));
                }
            }
            conn.execute(
                "UPDATE issues SET linked_task_id = ?2, updated_at = datetime('now')
                  WHERE id = ?1",
                rusqlite::params![id, task_id],
            )?;
        }
        self.event_bus.send(InternalEvent::IssueUpdated {
            id: id.into(),
            project_id: issue.project_id.clone(),
        });
        self.get(id)?
            .ok_or_else(|| Error::Internal(format!("issue vanished after link: {id}")))
    }

    /// Adds a blocked-by edge (`issue_id` is blocked by `blocked_by_id`).
    /// Rejects self-edges, cross-project edges, and cycles. Duplicate edges
    /// are idempotent. Emits `IssueUpdated` for the dependent.
    pub fn add_dependency(&self, issue_id: &str, blocked_by_id: &str) -> Result<Issue, Error> {
        let issue = self
            .get(issue_id)?
            .ok_or_else(|| Error::IssueNotFound(issue_id.into()))?;
        let blocker = self
            .get(blocked_by_id)?
            .ok_or_else(|| Error::IssueNotFound(blocked_by_id.into()))?;
        if issue.project_id != blocker.project_id {
            return Err(Error::InvalidIssueInput(format!(
                "blocked-by edges must stay within a project: {issue_id} is in {}, {blocked_by_id} is in {}",
                issue.project_id, blocker.project_id
            )));
        }
        {
            let conn = self.conn()?;
            let mut stmt =
                conn.prepare("SELECT issue_id, blocked_by_id FROM issue_dependencies")?;
            let edges: HashMap<String, Vec<String>> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<Vec<(String, String)>, _>>()?
                .into_iter()
                .fold(HashMap::new(), |mut acc, (dependent, blocker)| {
                    acc.entry(dependent).or_default().push(blocker);
                    acc
                });
            if would_cycle(&edges, issue_id, blocked_by_id) {
                return Err(Error::IssueDependencyCycle {
                    from: issue_id.into(),
                    to: blocked_by_id.into(),
                });
            }
            conn.execute(
                "INSERT OR IGNORE INTO issue_dependencies (issue_id, blocked_by_id)
                 VALUES (?1, ?2)",
                rusqlite::params![issue_id, blocked_by_id],
            )?;
        }
        self.event_bus.send(InternalEvent::IssueUpdated {
            id: issue_id.into(),
            project_id: issue.project_id.clone(),
        });
        self.get(issue_id)?
            .ok_or_else(|| Error::Internal(format!("issue vanished after link: {issue_id}")))
    }

    /// Removes a blocked-by edge. Errors when the edge does not exist.
    /// Emits `IssueUpdated` for the dependent.
    pub fn remove_dependency(&self, issue_id: &str, blocked_by_id: &str) -> Result<Issue, Error> {
        let issue = self
            .get(issue_id)?
            .ok_or_else(|| Error::IssueNotFound(issue_id.into()))?;
        {
            let conn = self.conn()?;
            let n = conn.execute(
                "DELETE FROM issue_dependencies WHERE issue_id = ?1 AND blocked_by_id = ?2",
                rusqlite::params![issue_id, blocked_by_id],
            )?;
            if n == 0 {
                return Err(Error::InvalidIssueInput(format!(
                    "no blocked-by edge from {issue_id} to {blocked_by_id}"
                )));
            }
        }
        self.event_bus.send(InternalEvent::IssueUpdated {
            id: issue_id.into(),
            project_id: issue.project_id.clone(),
        });
        self.get(issue_id)?
            .ok_or_else(|| Error::Internal(format!("issue vanished after unlink: {issue_id}")))
    }
}

/// Fills `blockers` for each issue with one query over the project(s) in
/// the slice (badge hover list, title-ordered). Each ref carries its
/// blocker's effective COLUMN — the id so the UI can name the column the
/// card is actually in, and the derived `counts_as_done` so consumers key
/// off the flag, never the lane string. Membership is
/// [`BLOCKER_COLUMN_MATCH`], the same resolution `blocked_sql` uses.
fn attach_blockers(conn: &rusqlite::Connection, issues: &mut [Issue]) -> Result<(), Error> {
    if issues.is_empty() {
        return Ok(());
    }
    let project_ids: Vec<&str> = {
        let mut ids: Vec<&str> = issues.iter().map(|i| i.project_id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let placeholders = project_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let mut stmt = conn.prepare(&format!(
        "SELECT d.issue_id, b.id, b.title, b.lane,
                EXISTS(SELECT 1 FROM board_columns c
                        WHERE {BLOCKER_COLUMN_MATCH}
                          AND c.counts_as_done = 1),
                (SELECT c.id FROM board_columns c WHERE {BLOCKER_COLUMN_MATCH})
           FROM issue_dependencies d
           JOIN issues b ON b.id = d.blocked_by_id
           JOIN issues i ON i.id = d.issue_id
          WHERE i.project_id IN ({placeholders})
          ORDER BY b.title"
    ))?;
    let rows = stmt.query_map(rusqlite::params_from_iter(project_ids), |row| {
        let lane: String = row.get(3)?;
        let counts_as_done: i64 = row.get(4)?;
        Ok((
            row.get::<_, String>(0)?,
            BlockerRef {
                id: row.get(1)?,
                title: row.get(2)?,
                lane: Lane::parse(&lane).unwrap_or(Lane::Backlog),
                counts_as_done: counts_as_done != 0,
                column_id: row.get(5)?,
            },
        ))
    })?;
    let mut by_issue: HashMap<String, Vec<BlockerRef>> = HashMap::new();
    for row in rows {
        let (issue_id, blocker) = row?;
        by_issue.entry(issue_id).or_default().push(blocker);
    }
    for issue in issues.iter_mut() {
        issue.blockers = by_issue.remove(&issue.id).unwrap_or_default();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDb;
    use crate::events::BroadcastEventBus;

    fn fixture() -> Arc<IssueStore> {
        let db: Arc<dyn Db> = SqliteDb::init(Some(":memory:")).unwrap();
        let bus = Arc::new(BroadcastEventBus::new(16));
        {
            let conn = db.conn().lock().unwrap();
            conn.execute_batch(
                "INSERT INTO projects (id, name, path) VALUES
                    ('p1', 'proj', '/tmp/proj'),
                    ('p2', 'other', '/tmp/other');",
            )
            .unwrap();
            // Every real project carries the seeded default board
            // (migration 0006 backfill / project-create hook), and blocked
            // derivation reads counts_as_done off it (E18-03) — so the
            // fixture seeds it too.
            columns::seed_default_columns(&conn, "p1").unwrap();
            columns::seed_default_columns(&conn, "p2").unwrap();
        }
        Arc::new(IssueStore::new(db, bus))
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

    fn with_task(store: &IssueStore, task_id: &str) {
        store
            .conn()
            .unwrap()
            .execute(
                "INSERT INTO tasks (id, project_id, name, status)
                 VALUES (?1, 'p1', 't', 'in_progress')",
                [task_id],
            )
            .unwrap();
    }

    #[test]
    fn create_and_get_round_trip_all_fields() {
        let store = fixture();
        let created = store
            .create(NewIssue {
                body: Some("body text".into()),
                acceptance: vec!["AC one".into(), "AC two".into()],
                lane: Some(Lane::Ready),
                provider: Some("claude".into()),
                model: Some("opus".into()),
                prd_path: Some("docs/prds/oauth.md".into()),
                prd_section: Some("## Flow".into()),
                ..new_issue("Token storage")
            })
            .unwrap();

        let fetched = store.get(&created.id).unwrap().unwrap();
        assert_eq!(fetched, created);
        assert_eq!(fetched.title, "Token storage");
        assert_eq!(fetched.body.as_deref(), Some("body text"));
        assert_eq!(fetched.acceptance, vec!["AC one", "AC two"]);
        assert_eq!(fetched.lane, Lane::Ready);
        assert_eq!(fetched.provider.as_deref(), Some("claude"));
        assert_eq!(fetched.model.as_deref(), Some("opus"));
        assert_eq!(fetched.prd_path.as_deref(), Some("docs/prds/oauth.md"));
        assert_eq!(fetched.prd_section.as_deref(), Some("## Flow"));
        assert!(!fetched.blocked);
        assert!(fetched.blockers.is_empty());
        assert!(fetched.linked_task_id.is_none());
        // E18-06: the card carries its column mirror from birth — here
        // via the explicit lane override's seeded column.
        let ready = columns::ColumnStore::new(store.db())
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.seed_lane.as_deref() == Some("ready"))
            .unwrap();
        assert_eq!(fetched.column_id.as_deref(), Some(ready.id.as_str()));
        assert!(fetched.created_at.is_some());
    }

    #[test]
    fn create_validates_title_and_project() {
        let store = fixture();
        assert!(matches!(
            store.create(new_issue("   ")),
            Err(Error::InvalidIssueInput(_))
        ));
        let mut orphan = new_issue("x");
        orphan.project_id = "nope".into();
        assert!(matches!(
            store.create(orphan),
            Err(Error::ProjectNotFound(_))
        ));
    }

    #[test]
    fn create_appends_positions_within_the_lane() {
        let store = fixture();
        let a = store.create(new_issue("a")).unwrap();
        let mut ready = new_issue("b");
        ready.lane = Some(Lane::Ready);
        let b = store.create(ready).unwrap();
        let c = store.create(new_issue("c")).unwrap();
        assert_eq!(a.position, 0);
        assert_eq!(b.position, 0); // separate lane, own position sequence
        assert_eq!(c.position, 1);
    }

    #[test]
    fn list_orders_by_lane_rank_then_position() {
        let store = fixture();
        // Insert out of board order to prove the ORDER BY, not insertion order.
        let mut done = new_issue("done-first");
        done.lane = Some(Lane::Done);
        store.create(done).unwrap();
        let mut ip = new_issue("in-progress");
        ip.lane = Some(Lane::InProgress);
        store.create(ip).unwrap();
        store.create(new_issue("backlog")).unwrap();
        let mut ready = new_issue("ready");
        ready.lane = Some(Lane::Ready);
        store.create(ready).unwrap();

        let titles: Vec<String> = store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .map(|i| i.title)
            .collect();
        assert_eq!(
            titles,
            vec!["backlog", "ready", "in-progress", "done-first"]
        );
        // Project scoping: p2 sees nothing.
        assert!(store.list_for_project("p2").unwrap().is_empty());
    }

    #[test]
    fn update_patches_and_clears_fields() {
        let store = fixture();
        let created = store
            .create(NewIssue {
                body: Some("keep me".into()),
                acceptance: vec!["old".into()],
                ..new_issue("before")
            })
            .unwrap();
        let updated = store
            .update(
                &created.id,
                IssuePatch {
                    title: Some("after".into()),
                    acceptance: Some(vec!["new".into()]),
                    prd_path: Some(Some("docs/prds/x.md".into())),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(updated.title, "after");
        assert_eq!(updated.body.as_deref(), Some("keep me"));
        assert_eq!(updated.acceptance, vec!["new"]);
        assert_eq!(updated.prd_path.as_deref(), Some("docs/prds/x.md"));

        // Some(None) clears; omitted fields stay.
        let cleared = store
            .update(
                &created.id,
                IssuePatch {
                    body: Some(None),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(cleared.body.is_none());
        assert_eq!(cleared.title, "after");

        assert!(matches!(
            store.update(
                &created.id,
                IssuePatch {
                    title: Some("  ".into()),
                    ..Default::default()
                }
            ),
            Err(Error::InvalidIssueInput(_))
        ));
        assert!(matches!(
            store.update("nope", IssuePatch::default()),
            Err(Error::IssueNotFound(_))
        ));
    }

    #[test]
    fn move_to_appends_or_sets_position() {
        let store = fixture();
        let a = store.create(new_issue("a")).unwrap();
        let b = store.create(new_issue("b")).unwrap();

        let moved = store.move_to(&a.id, Lane::InProgress, None).unwrap();
        assert_eq!(moved.lane, Lane::InProgress);
        assert_eq!(moved.position, 0); // first card in the lane

        let moved_b = store.move_to(&b.id, Lane::InProgress, None).unwrap();
        assert_eq!(moved_b.position, 1); // appended after a

        let front = store.move_to(&b.id, Lane::InProgress, Some(0)).unwrap();
        assert_eq!(front.position, 0);

        assert!(matches!(
            store.move_to("nope", Lane::Done, None),
            Err(Error::IssueNotFound(_))
        ));
    }

    /// GOLDEN (E18-03 parity): on the seeded board Done is the only
    /// counts_as_done column, so the flag-keyed derivation must behave
    /// exactly like the old `lane != 'done'` string test this ticket
    /// removed. Flag-specific behavior lives in
    /// `counts_as_done_flag_drives_blocked_derivation`.
    /// E18-06: a new card lands on the project's is_landing column and
    /// carries the mirror from birth — including when the flag has been
    /// MOVED to a user column (whose lane fallback is Backlog, since a
    /// non-seeded column has no lane representation).
    #[test]
    fn create_lands_on_the_landing_column() {
        let store = fixture();
        let col_store = columns::ColumnStore::new(store.db());
        let backlog = col_store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.is_landing)
            .unwrap();
        assert_eq!(backlog.name, "Backlog"); // seeded default

        let card = store.create(new_issue("a")).unwrap();
        assert_eq!(card.column_id.as_deref(), Some(backlog.id.as_str()));
        assert_eq!(card.lane, Lane::Backlog);

        // Move the landing flag to a NON-seeded column: new cards follow
        // it, with the lane falling back to Backlog.
        let triage = col_store
            .create(columns::NewColumn {
                project_id: "p1".into(),
                name: "Triage".into(),
                kind: columns::ColumnKind::Shelf,
                counts_as_done: false,
                is_landing: true,
                on_enter: None,
                on_settle: None,
                advance_to: None,
                step_prompt: None,
                step_provider: None,
                step_model: None,
                step_effort: None,
                step_tools: None,
            })
            .unwrap();
        let card = store.create(new_issue("b")).unwrap();
        assert_eq!(card.column_id.as_deref(), Some(triage.id.as_str()));
        assert_eq!(card.lane, Lane::Backlog); // interim, until E18-07

        // Move it to a SEEDED column: the lane follows the seed_lane.
        let ready = col_store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.seed_lane.as_deref() == Some("ready"))
            .unwrap();
        col_store
            .update(
                &ready.id,
                columns::ColumnPatch {
                    is_landing: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
        let card = store.create(new_issue("c")).unwrap();
        assert_eq!(card.column_id.as_deref(), Some(ready.id.as_str()));
        assert_eq!(card.lane, Lane::Ready);

        // An explicit lane override still wins (internal callers), and
        // still carries that lane's seeded mirror.
        let card = store
            .create(NewIssue {
                lane: Some(Lane::Done),
                ..new_issue("d")
            })
            .unwrap();
        assert_eq!(card.lane, Lane::Done);
        let done = col_store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.seed_lane.as_deref() == Some("done"))
            .unwrap();
        assert_eq!(card.column_id.as_deref(), Some(done.id.as_str()));
    }

    /// E18-06, GitHub-import path: the import command builds exactly this
    /// `NewIssue` (external_ref set, no lane hardcode), so imports land
    /// wherever the flag sits — and dedupe still returns the first card.
    #[test]
    fn github_import_shape_lands_on_a_moved_landing_column() {
        let store = fixture();
        let col_store = columns::ColumnStore::new(store.db());
        let triage = col_store
            .create(columns::NewColumn {
                project_id: "p1".into(),
                name: "Triage".into(),
                kind: columns::ColumnKind::Shelf,
                counts_as_done: false,
                is_landing: true,
                on_enter: None,
                on_settle: None,
                advance_to: None,
                step_prompt: None,
                step_provider: None,
                step_model: None,
                step_effort: None,
                step_tools: None,
            })
            .unwrap();

        let imported = store
            .create(NewIssue {
                external_ref: Some("https://github.com/o/r/issues/42".into()),
                ..new_issue("#42 Fix the flaky test")
            })
            .unwrap();
        assert_eq!(imported.column_id.as_deref(), Some(triage.id.as_str()));

        // Re-import returns the existing card, still on Triage.
        let again = store
            .create(NewIssue {
                external_ref: Some("https://github.com/o/r/issues/42".into()),
                ..new_issue("#42 Fix the flaky test (again)")
            })
            .unwrap();
        assert_eq!(again.id, imported.id);
        assert_eq!(again.column_id.as_deref(), Some(triage.id.as_str()));
    }

    /// A project with no board at all (columns never seeded) keeps the
    /// legacy Backlog default and a NULL mirror — no entry path breaks.
    #[test]
    fn create_without_a_board_falls_back_to_backlog() {
        let store = fixture();
        {
            let conn = store.conn().unwrap();
            conn.execute("DELETE FROM board_columns WHERE project_id = 'p1'", [])
                .unwrap();
        }
        let card = store.create(new_issue("a")).unwrap();
        assert_eq!(card.lane, Lane::Backlog);
        assert!(card.column_id.is_none());
    }

    /// E18-04: the enter primitive writes the mirror always and syncs the
    /// lane only through the reverse seed_lane mapping.
    #[test]
    fn enter_column_syncs_lane_only_for_seeded_columns() {
        let store = fixture();
        let col_store = columns::ColumnStore::new(store.db.clone());
        let board = col_store.list_for_project("p1").unwrap();
        let (quick, done) = (board[2].clone(), board[5].clone());
        let a = store.create(new_issue("a")).unwrap();

        // Seeded column: lane syncs (reverse seed_lane mapping), mirror set,
        // position appended within the synced lane.
        let entered = store.enter_column(&a.id, &done.id, None).unwrap();
        assert_eq!(entered.lane, Lane::Done);
        assert_eq!(entered.column_id.as_deref(), Some(done.id.as_str()));
        assert_eq!(entered.position, 0);

        // Non-seeded column (Quick): mirror moves, lane UNCHANGED (interim
        // until the E18-07 authority flip — the legacy board cannot render
        // non-seeded columns, so the stale lane is invisible there).
        let entered = store.enter_column(&a.id, &quick.id, None).unwrap();
        assert_eq!(entered.lane, Lane::Done); // untouched
        assert_eq!(entered.column_id.as_deref(), Some(quick.id.as_str()));

        // Unknown and cross-project columns are typed rejections.
        assert!(matches!(
            store.enter_column(&a.id, "col_nope", None),
            Err(Error::BoardColumnNotFound(_))
        ));
        let p2_board = col_store.list_for_project("p2").unwrap();
        assert!(matches!(
            store.enter_column(&a.id, &p2_board[0].id, None),
            Err(Error::InvalidBoardColumnInput(_))
        ));
        assert!(matches!(
            store.enter_column("nope", &done.id, None),
            Err(Error::IssueNotFound(_))
        ));
    }

    /// GOLDEN (E18-04 parity): lane-addressed moves route through the
    /// enter primitive with identical lane/position/event behavior — the
    /// only new effect is the maintained mirror. The fallback (seeded
    /// column deleted) keeps the legacy lane write and clears the mirror.
    #[test]
    fn move_to_routes_through_the_enter_primitive() {
        let store = fixture();
        let col_store = columns::ColumnStore::new(store.db.clone());
        let mut rx = store.event_bus.subscribe();
        let a = store.create(new_issue("a")).unwrap();
        let b = store.create(new_issue("b")).unwrap();
        let _ = rx.try_recv(); // drain IssueCreated a
        let _ = rx.try_recv(); // drain IssueCreated b

        // Same contract as before routing: lane set, append-to-lane
        // positioning, explicit position honored, exactly ONE IssueUpdated.
        let moved = store.move_to(&a.id, Lane::InProgress, None).unwrap();
        assert_eq!(moved.lane, Lane::InProgress);
        assert_eq!(moved.position, 0);
        let moved_b = store.move_to(&b.id, Lane::InProgress, None).unwrap();
        assert_eq!(moved_b.position, 1);
        let front = store.move_to(&b.id, Lane::InProgress, Some(0)).unwrap();
        assert_eq!(front.position, 0);
        assert!(matches!(
            rx.try_recv().unwrap(),
            InternalEvent::IssueUpdated { ref id, .. } if id == &a.id
        ));
        assert!(matches!(
            rx.try_recv().unwrap(),
            InternalEvent::IssueUpdated { ref id, .. } if id == &b.id
        ));
        assert!(matches!(
            rx.try_recv().unwrap(),
            InternalEvent::IssueUpdated { ref id, .. } if id == &b.id
        ));
        assert!(rx.try_recv().is_err(), "exactly one event per move");
        // New effect: the mirror now points at the seeded In Progress
        // column.
        let in_progress = col_store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.seed_lane.as_deref() == Some("in_progress"))
            .unwrap();
        assert_eq!(front.column_id.as_deref(), Some(in_progress.id.as_str()));

        // Fallback: with the Ready column deleted, a move to lane ready
        // still works (legacy write) and the mirror clears — no column can
        // represent that lane.
        let ready = col_store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.seed_lane.as_deref() == Some("ready"))
            .unwrap();
        col_store.delete(&ready.id).unwrap();
        let moved = store.move_to(&a.id, Lane::Ready, None).unwrap();
        assert_eq!(moved.lane, Lane::Ready);
        assert!(moved.column_id.is_none());
    }

    /// Fix round (finding 13): a same-lane reorder is position-only — it
    /// must never clobber the mirror of a card resident in a non-seeded
    /// column (the legacy board renders that card in its stale lane, so
    /// innocuous reorders there are routine).
    #[test]
    fn same_lane_reorder_keeps_position_only() {
        let store = fixture();
        let col_store = columns::ColumnStore::new(store.db.clone());
        let quick = col_store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.name == "Quick")
            .unwrap();
        let a = store.create(new_issue("a")).unwrap(); // lane backlog
        let _b = store.create(new_issue("b")).unwrap();
        let entered = store.enter_column(&a.id, &quick.id, None).unwrap();
        assert_eq!(entered.lane, Lane::Backlog); // stale lane, by design
        assert_eq!(entered.column_id.as_deref(), Some(quick.id.as_str()));

        // Reorder while resident in Quick: the mirror and lane are
        // untouched. The position is now an index in the card's effective
        // COLUMN (fix round, finding 9) — A is Quick's only card, so an
        // out-of-range request clamps to 0 instead of writing a sparse
        // absolute value into the stale lane's namespace.
        let reordered = store.move_to(&a.id, Lane::Backlog, Some(5)).unwrap();
        assert_eq!(reordered.position, 0);
        assert_eq!(reordered.lane, Lane::Backlog);
        assert_eq!(reordered.column_id.as_deref(), Some(quick.id.as_str()));

        // A cross-lane move still routes through the enter primitive.
        let moved = store.move_to(&a.id, Lane::Ready, None).unwrap();
        assert_eq!(moved.lane, Lane::Ready);
        assert_ne!(moved.column_id.as_deref(), Some(quick.id.as_str()));
    }

    #[test]
    fn blocked_is_derived_from_direct_blocker_lanes() {
        let store = fixture();
        let a = store.create(new_issue("a")).unwrap();
        let b = store.create(new_issue("b")).unwrap();
        let c = store.create(new_issue("c")).unwrap();
        store.add_dependency(&a.id, &b.id).unwrap();
        store.add_dependency(&c.id, &a.id).unwrap();

        // B not done → A blocked; A not done → C blocked (through A).
        assert!(store.get(&a.id).unwrap().unwrap().blocked);
        assert!(store.get(&c.id).unwrap().unwrap().blocked);

        // B done → A unblocks with no write to A. C still blocked: its
        // DIRECT blocker A is not done — derivation never looks past it.
        store.move_to(&b.id, Lane::Done, None).unwrap();
        assert!(!store.get(&a.id).unwrap().unwrap().blocked);
        assert!(store.get(&c.id).unwrap().unwrap().blocked);

        store.move_to(&a.id, Lane::Done, None).unwrap();
        assert!(!store.get(&c.id).unwrap().unwrap().blocked);
    }

    /// E18-03 (#77): the counts_as_done COLUMN flag — not the 'done' lane
    /// string — decides what finishes a blocker.
    #[test]
    fn counts_as_done_flag_drives_blocked_derivation() {
        let store = fixture();
        let col_store = columns::ColumnStore::new(store.db.clone());
        let a = store.create(new_issue("a")).unwrap();
        let b = store.create(new_issue("b")).unwrap();
        store.add_dependency(&a.id, &b.id).unwrap();

        let column_id = |seed_lane: &str| -> String {
            col_store
                .list_for_project("p1")
                .unwrap()
                .into_iter()
                .find(|c| c.seed_lane.as_deref() == Some(seed_lane))
                .unwrap()
                .id
        };

        // Seeded board: In Review does not count as done → blocked.
        store.move_to(&b.id, Lane::InReview, None).unwrap();
        assert!(store.get(&a.id).unwrap().unwrap().blocked);

        // A SECOND counts_as_done column (In Review, flagged through the
        // public column API) unblocks the dependent even though the
        // blocker's lane string is 'in_review', not 'done'.
        col_store
            .update(
                &column_id("in_review"),
                columns::ColumnPatch {
                    counts_as_done: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
        let a_read = store.get(&a.id).unwrap().unwrap();
        assert!(!a_read.blocked);
        assert!(a_read.blockers[0].counts_as_done);

        // Strip the flag from Done: a blocker sitting in lane 'done' no
        // longer finishes anything — proof the string test is gone.
        col_store
            .update(
                &column_id("done"),
                columns::ColumnPatch {
                    counts_as_done: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        store.move_to(&b.id, Lane::Done, None).unwrap();
        let a_read = store.get(&a.id).unwrap().unwrap();
        assert!(a_read.blocked);
        assert!(!a_read.blockers[0].counts_as_done);

        // A blocker whose column cannot be resolved AT ALL counts as
        // UNFINISHED — fail toward blocked. Membership is mirror-first, so
        // orphaning it takes clearing BOTH pointers: unmap the flagged
        // column's seed_lane and drop the card's mirror. (Nulling the
        // seed_lane alone no longer orphans anything, which is the point
        // of the fix — the card really is in that column.)
        store.move_to(&b.id, Lane::InReview, None).unwrap();
        assert!(!store.get(&a.id).unwrap().unwrap().blocked); // flagged + mapped
        // Each `conn()` is a temporary — holding the guard across a
        // `store.get()` would re-lock the same mutex and deadlock.
        store
            .conn()
            .unwrap()
            .execute(
                "UPDATE board_columns SET seed_lane = NULL
                  WHERE project_id = 'p1' AND seed_lane = 'in_review'",
                [],
            )
            .unwrap();
        // Still resolvable through the mirror: the flag stands.
        assert!(!store.get(&a.id).unwrap().unwrap().blocked);
        store
            .conn()
            .unwrap()
            .execute("UPDATE issues SET column_id = NULL WHERE id = ?1", [&b.id])
            .unwrap();
        assert!(store.get(&a.id).unwrap().unwrap().blocked);
    }

    /// Fix round (finding 6): dragging a finished card OUT of a
    /// counts_as_done column into a non-seeded one (Quick, on every
    /// seeded board) must re-block its dependents. Lane stays stale for
    /// non-seeded columns, so a lane-only resolution kept reporting the
    /// card as done — and the dependent's agent was told the work had
    /// already landed.
    #[test]
    fn reopening_a_done_card_into_a_non_seeded_column_reblocks_dependents() {
        let store = fixture();
        let col_store = columns::ColumnStore::new(store.db.clone());
        let quick = col_store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.name == "Quick")
            .unwrap();
        let a = store.create(new_issue("a")).unwrap();
        let b = store.create(new_issue("b")).unwrap();
        store.add_dependency(&a.id, &b.id).unwrap();

        store.move_to(&b.id, Lane::Done, None).unwrap();
        let read = store.get(&a.id).unwrap().unwrap();
        assert!(!read.blocked, "a blocker in Done finishes the dependency");
        assert!(read.blockers[0].counts_as_done);

        // Reopen: the mirror moves to Quick, the lane stays 'done'.
        let reopened = store.enter_column(&b.id, &quick.id, None).unwrap();
        assert_eq!(reopened.lane, Lane::Done, "stale lane, by design");
        assert_eq!(reopened.column_id.as_deref(), Some(quick.id.as_str()));

        let read = store.get(&a.id).unwrap().unwrap();
        assert!(read.blocked, "the card is in Quick now, not Done");
        assert!(!read.blockers[0].counts_as_done);
        assert_eq!(
            read.blockers[0].column_id.as_deref(),
            Some(quick.id.as_str()),
            "the blocker row names the column the card is really in"
        );
    }

    /// Fix round (finding 9): a drop lands where it was dropped. Position
    /// used to be an absolute single-row write with no renumbering, so
    /// dropping onto slot 0 tied with the card already there and a stable
    /// sort put the dropped card second.
    #[test]
    fn drops_land_at_the_index_they_were_dropped_on() {
        let store = fixture();
        let order = |store: &IssueStore| -> Vec<String> {
            store
                .list_for_project("p1")
                .unwrap()
                .into_iter()
                .filter(|i| i.lane == Lane::Backlog)
                .map(|i| i.title)
                .collect()
        };
        let _a = store.create(new_issue("a")).unwrap();
        let b = store.create(new_issue("b")).unwrap();
        let c = store.create(new_issue("c")).unwrap();
        assert_eq!(order(&store), vec!["a", "b", "c"]);

        // Drag C to the top.
        store.move_to(&c.id, Lane::Backlog, Some(0)).unwrap();
        assert_eq!(order(&store), vec!["c", "a", "b"]);

        // Drag B to the middle (after-removal index 1).
        store.move_to(&b.id, Lane::Backlog, Some(1)).unwrap();
        assert_eq!(order(&store), vec!["c", "b", "a"]);

        // Drag C to the bottom.
        store.move_to(&c.id, Lane::Backlog, Some(2)).unwrap();
        assert_eq!(order(&store), vec!["b", "a", "c"]);

        // Positions stay dense, so no tie can reappear.
        let mut positions: Vec<i64> = store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .filter(|i| i.lane == Lane::Backlog)
            .map(|i| i.position)
            .collect();
        positions.sort_unstable();
        assert_eq!(positions, vec![0, 1, 2]);
    }

    /// Fix round (finding 9, second half): a NON-seeded column orders its
    /// own cards. Positions used to be allocated in the stale lane, so two
    /// cards in Quick could both hold position 0 in a lane neither of them
    /// is displayed in.
    #[test]
    fn non_seeded_columns_order_their_own_cards() {
        let store = fixture();
        let col_store = columns::ColumnStore::new(store.db.clone());
        let quick = col_store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.name == "Quick")
            .unwrap();
        let a = store.create(new_issue("a")).unwrap();
        let b = store.create(new_issue("b")).unwrap();

        store.enter_column(&a.id, &quick.id, None).unwrap();
        // B is dropped ABOVE A.
        store.enter_column(&b.id, &quick.id, Some(0)).unwrap();

        let in_quick: Vec<(String, i64)> = store
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .filter(|i| i.column_id.as_deref() == Some(quick.id.as_str()))
            .map(|i| (i.title, i.position))
            .collect();
        assert_eq!(
            in_quick,
            vec![("b".to_string(), 0), ("a".to_string(), 1)],
            "distinct, dense positions in drop order"
        );
    }

    #[test]
    fn blockers_attached_for_the_hover_list() {
        let store = fixture();
        let a = store.create(new_issue("a")).unwrap();
        let b = store.create(new_issue("b")).unwrap();
        let c = store.create(new_issue("c")).unwrap();
        store.add_dependency(&a.id, &b.id).unwrap();
        store.add_dependency(&a.id, &c.id).unwrap();

        let a = store.get(&a.id).unwrap().unwrap();
        let titles: Vec<&str> = a.blockers.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(titles, vec!["b", "c"]); // title-ordered
        assert_eq!(a.blockers[0].lane, Lane::Backlog);
        assert!(!a.blockers[0].counts_as_done);
        // The flag rides along per blocker (E18-03): c lands in Done —
        // counts as done on the seeded board — while b stays backlog.
        store.move_to(&c.id, Lane::Done, None).unwrap();
        let a = store.get(&a.id).unwrap().unwrap();
        let flags: Vec<bool> = a.blockers.iter().map(|b| b.counts_as_done).collect();
        assert_eq!(flags, vec![false, true]);
        // Blockers list is on the DEPENDENT only.
        assert!(store.get(&b.id).unwrap().unwrap().blockers.is_empty());
    }

    #[test]
    fn add_dependency_rejects_cycles_self_and_cross_project() {
        let store = fixture();
        let a = store.create(new_issue("a")).unwrap();
        let b = store.create(new_issue("b")).unwrap();
        let c = store.create(new_issue("c")).unwrap();
        store.add_dependency(&a.id, &b.id).unwrap();
        store.add_dependency(&b.id, &c.id).unwrap();

        // Transitive cycle: c blocked-by a would close a→b→c→a.
        assert!(matches!(
            store.add_dependency(&c.id, &a.id),
            Err(Error::IssueDependencyCycle { .. })
        ));
        // Self-edge.
        assert!(matches!(
            store.add_dependency(&a.id, &a.id),
            Err(Error::IssueDependencyCycle { .. })
        ));
        // Duplicate edge is idempotent, not an error.
        store.add_dependency(&a.id, &b.id).unwrap();
        assert_eq!(store.get(&a.id).unwrap().unwrap().blockers.len(), 1);
        // Unknown endpoints.
        assert!(matches!(
            store.add_dependency(&a.id, "nope"),
            Err(Error::IssueNotFound(_))
        ));
        // Cross-project edge.
        let mut foreign = new_issue("foreign");
        foreign.project_id = "p2".into();
        let foreign = store.create(foreign).unwrap();
        assert!(matches!(
            store.add_dependency(&a.id, &foreign.id),
            Err(Error::InvalidIssueInput(_))
        ));
    }

    #[test]
    fn remove_dependency_round_trip_and_missing_edge() {
        let store = fixture();
        let a = store.create(new_issue("a")).unwrap();
        let b = store.create(new_issue("b")).unwrap();
        store.add_dependency(&a.id, &b.id).unwrap();
        assert!(store.get(&a.id).unwrap().unwrap().blocked);

        let a = store.remove_dependency(&a.id, &b.id).unwrap();
        assert!(!a.blocked);
        assert!(a.blockers.is_empty());
        assert!(matches!(
            store.remove_dependency(&a.id, &b.id),
            Err(Error::InvalidIssueInput(_))
        ));
    }

    #[test]
    fn delete_cascades_edges_in_both_directions() {
        let store = fixture();
        let a = store.create(new_issue("a")).unwrap();
        let b = store.create(new_issue("b")).unwrap();
        let c = store.create(new_issue("c")).unwrap();
        store.add_dependency(&a.id, &b.id).unwrap();
        store.add_dependency(&c.id, &a.id).unwrap();

        store.delete(&a.id).unwrap();
        assert!(store.get(&a.id).unwrap().is_none());
        // Edge where A was the blocker is gone — C is unblocked.
        let c = store.get(&c.id).unwrap().unwrap();
        assert!(!c.blocked);
        assert!(c.blockers.is_empty());
        assert!(matches!(store.delete(&a.id), Err(Error::IssueNotFound(_))));
    }

    #[test]
    fn linked_task_set_clear_and_task_delete_clears() {
        let store = fixture();
        with_task(&store, "task-1");
        let a = store.create(new_issue("a")).unwrap();

        let linked = store.set_linked_task(&a.id, Some("task-1")).unwrap();
        assert_eq!(linked.linked_task_id.as_deref(), Some("task-1"));
        assert!(matches!(
            store.set_linked_task(&a.id, Some("nope")),
            Err(Error::TaskNotFound(_))
        ));

        // Task deletion clears the pointer (ON DELETE SET NULL) — the card
        // survives its task's teardown (ADR-0032).
        store
            .conn()
            .unwrap()
            .execute("DELETE FROM tasks WHERE id = 'task-1'", [])
            .unwrap();
        assert!(store.get(&a.id).unwrap().unwrap().linked_task_id.is_none());

        // Explicit clear path.
        with_task(&store, "task-2");
        store.set_linked_task(&a.id, Some("task-2")).unwrap();
        let cleared = store.set_linked_task(&a.id, None).unwrap();
        assert!(cleared.linked_task_id.is_none());
    }

    #[test]
    fn mutations_emit_events_with_project_scope() {
        let store = fixture();
        let mut rx = store.event_bus.subscribe();
        let a = store.create(new_issue("a")).unwrap();
        store.move_to(&a.id, Lane::Ready, None).unwrap();
        store.delete(&a.id).unwrap();

        assert!(matches!(
            rx.try_recv().unwrap(),
            InternalEvent::IssueCreated { ref id, ref project_id, .. }
                if id == &a.id && project_id == "p1"
        ));
        assert!(matches!(
            rx.try_recv().unwrap(),
            InternalEvent::IssueUpdated { ref id, .. } if id == &a.id
        ));
        assert!(matches!(
            rx.try_recv().unwrap(),
            InternalEvent::IssueDeleted { ref id, .. } if id == &a.id
        ));
    }

    #[test]
    fn external_ref_dedupes_imports() {
        let store = fixture();
        let mut gh = new_issue("#42 Fix the flaky test");
        gh.external_ref = Some("https://github.com/o/r/issues/42".into());
        let first = store.create(gh).unwrap();
        assert_eq!(
            first.external_ref.as_deref(),
            Some("https://github.com/o/r/issues/42")
        );

        // Importing the same GitHub issue again returns the existing card.
        let mut dup = new_issue("#42 Fix the flaky test (again)");
        dup.external_ref = Some("https://github.com/o/r/issues/42".into());
        let second = store.create(dup).unwrap();
        assert_eq!(second.id, first.id);
        assert_eq!(store.list_for_project("p1").unwrap().len(), 1);

        // A different external ref (or none) creates a new card.
        let mut other = new_issue("#43 Something else");
        other.external_ref = Some("https://github.com/o/r/issues/43".into());
        store.create(other).unwrap();
        assert_eq!(store.list_for_project("p1").unwrap().len(), 2);
    }

    #[test]
    fn dispatch_prompt_packets_sections_and_references() {
        let store = fixture();
        let mut issue = store
            .create(NewIssue {
                body: Some("Store tokens encrypted.".into()),
                acceptance: vec!["round-trips a token".into()],
                prd_path: Some("docs/prds/oauth.md".into()),
                prd_section: Some("## Flow".into()),
                ..new_issue("Token storage")
            })
            .unwrap();
        let blockers = vec!["Design schema".to_string()];
        let prompt = build_dispatch_prompt(&issue, &blockers);
        assert!(prompt.contains("# Issue\nToken storage"));
        assert!(prompt.contains("Store tokens encrypted."));
        assert!(prompt.contains("- round-trips a token"));
        assert!(prompt.contains("PRD: docs/prds/oauth.md (section: ## Flow)"));
        // Honest flag-keyed wording (E18-03): "marked finished on the
        // board", never a bare merge-state assertion from a lane name.
        assert!(prompt.contains("Dependency \"Design schema\" is marked finished on the board"));
        assert!(prompt.contains("expect its work on the default branch"));
        assert!(!prompt.contains("is done — its work is in the default branch"));
        assert!(prompt.contains("dedicated branch in a git worktree"));

        // Empty sections are omitted, never rendered blank.
        issue.body = None;
        issue.acceptance = vec![];
        issue.prd_path = None;
        issue.prd_section = None;
        let minimal = build_dispatch_prompt(&issue, &[]);
        assert!(!minimal.contains("# Acceptance criteria"));
        assert!(!minimal.contains("# Context"));
        assert!(minimal.contains("# Conventions"));
    }
}
