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
    /// Derived at read time (ADR-0032): any direct blocker not in Done.
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
    /// Defaults to [`Lane::Backlog`].
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
     created_at, updated_at";

/// Derived-blocked subquery (E18-03, #77; ADR-0037 item 6): true when any
/// direct blocker is NOT in a `counts_as_done` column.
///
/// The blocker's column is resolved through the seed_lane mapping
/// (`b.lane` → `board_columns.seed_lane`, same project) because lane is
/// still authoritative in the E18 spike; a blocker whose lane maps to no
/// column counts as UNFINISHED (fail toward blocked). When `column_id`
/// becomes authoritative (E18-07 era) only the resolution join changes —
/// to `c.id = b.column_id` — because the `counts_as_done` flag test is
/// already the path doing the work here.
const BLOCKED_SQL: &str = "EXISTS(SELECT 1 FROM issue_dependencies d \
     JOIN issues b ON b.id = d.blocked_by_id \
     WHERE d.issue_id = issues.id \
       AND NOT EXISTS(SELECT 1 FROM board_columns c \
            WHERE c.project_id = b.project_id \
              AND c.seed_lane = b.lane \
              AND c.counts_as_done = 1))";

fn issue_from_row(row: &rusqlite::Row) -> rusqlite::Result<Issue> {
    let lane: String = row.get(5)?;
    let acceptance_cell: Option<String> = row.get(4)?;
    let blocked: i64 = row.get(15)?;
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
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        blocked: blocked != 0,
        blockers: Vec::new(), // attached by the caller (second query)
    })
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

    /// Issues whose dispatch link points at this task (E17-03 auto-flip).
    pub fn list_by_linked_task(&self, task_id: &str) -> Result<Vec<Issue>, Error> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS}, {BLOCKED_SQL} FROM issues WHERE linked_task_id = ?1"
        ))?;
        let mut issues = stmt
            .query_map([task_id], issue_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        attach_blockers(&conn, &mut issues)?;
        Ok(issues)
    }

    /// Creates an issue appended to the end of its lane. Emits `IssueCreated`.
    pub fn create(&self, new: NewIssue) -> Result<Issue, Error> {
        let title = new.title.trim().to_string();
        if title.is_empty() {
            return Err(Error::InvalidIssueInput("title is empty".into()));
        }
        let lane = new.lane.unwrap_or(Lane::Backlog);
        let acceptance = serialize_versioned(&AcceptanceCriteria {
            items: new.acceptance,
        })?;
        let id = format!("iss_{}", uuid::Uuid::new_v4());
        let mut deduped_id: Option<String> = None;
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
                          provider, model, prd_path, prd_section, external_ref)
                     VALUES (
                         ?1, ?2, ?3, ?4, ?5, ?6,
                         (SELECT COALESCE(MAX(position) + 1, 0) FROM issues
                           WHERE project_id = ?2 AND lane = ?6),
                         ?7, ?8, ?9, ?10, ?11
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
                &format!("SELECT {COLUMNS}, {BLOCKED_SQL} FROM issues WHERE id = ?1"),
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
            "SELECT {COLUMNS}, {BLOCKED_SQL} FROM issues
              WHERE project_id = ?1
              ORDER BY CASE lane
                           WHEN 'backlog' THEN 0 WHEN 'ready' THEN 1
                           WHEN 'in_progress' THEN 2 WHEN 'in_review' THEN 3
                           WHEN 'done' THEN 4 ELSE 5
                       END, position, created_at"
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
    pub fn move_to(&self, id: &str, lane: Lane, position: Option<i64>) -> Result<Issue, Error> {
        let issue = self
            .get(id)?
            .ok_or_else(|| Error::IssueNotFound(id.into()))?;
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
                "UPDATE issues SET lane = ?2, position = ?3, updated_at = datetime('now')
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
/// the slice (badge hover list: blocker id/title/lane, title-ordered).
/// Each ref carries the derived `counts_as_done` of the blocker's column
/// (same seed_lane resolution as [`BLOCKED_SQL`]) so consumers key off the
/// flag, never the lane string.
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
                        WHERE c.project_id = b.project_id
                          AND c.seed_lane = b.lane
                          AND c.counts_as_done = 1)
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

        // Unmapped-lane fallback counts as UNFINISHED. Lane authority
        // means no public API can host an issue in a column without a
        // seed_lane (move_to only speaks Lane), so the orphan state is
        // constructed directly: unmap the flagged In Review column and
        // park the blocker's lane on it.
        store.move_to(&b.id, Lane::InReview, None).unwrap();
        assert!(!store.get(&a.id).unwrap().unwrap().blocked); // flagged + mapped
        store
            .conn()
            .unwrap()
            .execute(
                "UPDATE board_columns SET seed_lane = NULL
                  WHERE project_id = 'p1' AND seed_lane = 'in_review'",
                [],
            )
            .unwrap();
        assert!(store.get(&a.id).unwrap().unwrap().blocked);
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
