//! Diff line comments (E4-10, #50; ARCHITECTURE.md §14).
//!
//! GitHub-style review comments anchored to diff lines. Two modes: "Add
//! Note" (human-only) and "Create Task" (comment → new task via
//! [`build_comment_prompt`]). Comments persist across restarts and link
//! bidirectionally to tasks: `line_comments.linked_task_id` ↔
//! `tasks.source_comment_id`. Resolution is manual — a linked task
//! finishing flips the badge to "→ done" but does NOT auto-resolve (§14
//! decision).
//!
//! Schema: base table in migration 0000; Phase-1 columns (source_side,
//! line_end, linked_task_id, resolved, resolved_at, created_by,
//! tasks.source_comment_id) in migration 0001.

use std::sync::Arc;

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::events::{EventBus, InternalEvent};
use crate::Error;

/// Which side of a split diff the comment anchors to (§14: "Before" for
/// understanding old code; "After" for suggesting changes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceSide {
    Before,
    After,
}

impl SourceSide {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceSide::Before => "before",
            SourceSide::After => "after",
        }
    }

    pub fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "before" => Ok(SourceSide::Before),
            "after" | "" => Ok(SourceSide::After),
            other => Err(Error::Internal(format!(
                "invalid comment side: {other:?} (expected 'before' | 'after')"
            ))),
        }
    }
}

/// A line comment row (columns mirror `line_comments` verbatim).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineComment {
    pub id: String,
    /// The task whose diff is being reviewed (NOT the linked task).
    pub task_id: String,
    pub file_path: String,
    pub line_number: i64,
    /// End of the selection (== line_number for single-line).
    pub line_end: Option<i64>,
    pub source_side: SourceSide,
    /// Snapshot of the first selected line at comment time.
    pub line_content: Option<String>,
    pub content: String,
    pub resolved: bool,
    pub resolved_at: Option<String>,
    pub created_by: String,
    /// Task spawned from this comment ("Create Task" flow), if any.
    pub linked_task_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Options for [`LineCommentStore::add`].
pub struct AddLineCommentOptions {
    pub task_id: String,
    pub file_path: String,
    pub line_number: i64,
    pub line_end: Option<i64>,
    pub source_side: SourceSide,
    pub line_content: Option<String>,
    pub content: String,
    /// `"user"` (default) or `"agent:<conversation_id>"` (§14 tool surface).
    pub created_by: Option<String>,
}

pub struct LineCommentStore {
    db: Arc<dyn Db>,
    event_bus: Arc<dyn EventBus>,
}

const COLUMNS: &str = "id, task_id, file_path, line_number, line_end, source_side, \
     line_content, content, resolved, resolved_at, created_by, linked_task_id, \
     created_at, updated_at";

fn comment_from_row(row: &rusqlite::Row) -> rusqlite::Result<LineComment> {
    let side: String = row.get(5)?;
    let resolved: i64 = row.get(8)?;
    Ok(LineComment {
        id: row.get(0)?,
        task_id: row.get(1)?,
        file_path: row.get(2)?,
        line_number: row.get(3)?,
        line_end: row.get(4)?,
        source_side: SourceSide::parse(&side).unwrap_or(SourceSide::After),
        line_content: row.get(6)?,
        content: row.get(7)?,
        resolved: resolved != 0,
        resolved_at: row.get(9)?,
        created_by: row.get(10)?,
        linked_task_id: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

impl LineCommentStore {
    pub fn new(db: Arc<dyn Db>, event_bus: Arc<dyn EventBus>) -> Self {
        Self { db, event_bus }
    }

    /// Creates a comment (id `lc_<uuid>`, §14 event flow) and emits
    /// `CommentCreated`.
    pub fn add(&self, opts: AddLineCommentOptions) -> Result<LineComment, Error> {
        if opts.content.trim().is_empty() {
            return Err(Error::Internal("comment content is empty".into()));
        }
        if opts.line_number < 1 {
            return Err(Error::Internal(format!(
                "invalid line number: {}",
                opts.line_number
            )));
        }
        if let Some(end) = opts.line_end {
            if end < opts.line_number {
                return Err(Error::Internal(format!(
                    "line_end {end} before line_number {}",
                    opts.line_number
                )));
            }
        }
        let id = format!("lc_{}", uuid::Uuid::new_v4());
        {
            let conn = self.conn()?;
            let task_exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?1)",
                [&opts.task_id],
                |row| row.get(0),
            )?;
            if !task_exists {
                return Err(Error::TaskNotFound(opts.task_id));
            }
            conn.execute(
                "INSERT INTO line_comments
                     (id, task_id, file_path, line_number, line_end, source_side,
                      line_content, content, created_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    id,
                    opts.task_id,
                    opts.file_path,
                    opts.line_number,
                    opts.line_end,
                    opts.source_side.as_str(),
                    opts.line_content,
                    opts.content,
                    opts.created_by.as_deref().unwrap_or("user"),
                ],
            )?;
        }
        self.event_bus.send(InternalEvent::CommentCreated {
            id: id.clone(),
            task_id: opts.task_id.clone(),
            file_path: opts.file_path.clone(),
            line_number: opts.line_number,
        });
        self.get(&id)?
            .ok_or_else(|| Error::Internal(format!("comment vanished after insert: {id}")))
    }

    /// Lists comments for a task, optionally narrowed to one file; ordered
    /// by file then line (gutter render order).
    pub fn list_for_task(
        &self,
        task_id: &str,
        file_path: Option<&str>,
    ) -> Result<Vec<LineComment>, Error> {
        let conn = self.conn()?;
        let mut sql = format!("SELECT {COLUMNS} FROM line_comments WHERE task_id = ?1");
        if file_path.is_some() {
            sql.push_str(" AND file_path = ?2");
        }
        sql.push_str(" ORDER BY file_path, line_number, created_at");
        let mut stmt = conn.prepare(&sql)?;
        let rows = match file_path {
            Some(fp) => stmt.query_map(rusqlite::params![task_id, fp], comment_from_row)?,
            None => stmt.query_map(rusqlite::params![task_id], comment_from_row)?,
        };
        rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
    }

    pub fn get(&self, id: &str) -> Result<Option<LineComment>, Error> {
        let conn = self.conn()?;
        conn.query_row(
            &format!("SELECT {COLUMNS} FROM line_comments WHERE id = ?1"),
            [id],
            comment_from_row,
        )
        .optional()
        .map_err(Error::from)
    }

    /// Manual resolution (§14 decision): idempotent — re-resolving returns
    /// the comment unchanged. Emits `CommentResolved` on the open→resolved
    /// transition only.
    pub fn resolve(&self, id: &str) -> Result<LineComment, Error> {
        let comment = self
            .get(id)?
            .ok_or_else(|| Error::Internal(format!("line comment not found: {id}")))?;
        if !comment.resolved {
            {
                let conn = self.conn()?;
                conn.execute(
                    "UPDATE line_comments
                        SET resolved = 1, resolved_at = datetime('now'), updated_at = datetime('now')
                      WHERE id = ?1",
                    [id],
                )?;
            }
            self.event_bus
                .send(InternalEvent::CommentResolved { id: id.into() });
        }
        self.get(id)?
            .ok_or_else(|| Error::Internal(format!("line comment vanished: {id}")))
    }

    pub fn delete(&self, id: &str) -> Result<(), Error> {
        let conn = self.conn()?;
        let n = conn.execute("DELETE FROM line_comments WHERE id = ?1", [id])?;
        if n == 0 {
            return Err(Error::Internal(format!("line comment not found: {id}")));
        }
        Ok(())
    }

    /// Bidirectional task link (§14): sets `line_comments.linked_task_id`
    /// AND `tasks.source_comment_id` atomically. Both rows must exist.
    pub fn link_task(&self, comment_id: &str, task_id: &str) -> Result<(), Error> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let comment_exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM line_comments WHERE id = ?1)",
            [comment_id],
            |row| row.get(0),
        )?;
        if !comment_exists {
            return Err(Error::Internal(format!(
                "line comment not found: {comment_id}"
            )));
        }
        let task_exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?1)",
            [task_id],
            |row| row.get(0),
        )?;
        if !task_exists {
            return Err(Error::TaskNotFound(task_id.into()));
        }
        tx.execute(
            "UPDATE line_comments SET linked_task_id = ?2, updated_at = datetime('now')
              WHERE id = ?1",
            rusqlite::params![comment_id, task_id],
        )?;
        tx.execute(
            "UPDATE tasks SET source_comment_id = ?2, updated_at = datetime('now')
              WHERE id = ?1",
            rusqlite::params![task_id, comment_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>, Error> {
        self.db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))
    }

    // -- Agent tool surface (E4-11, #51) ------------------------------------

    /// Resolves the reviewed task's materialized worktree path (the agent
    /// tool validates anchors against it). `None` when the task has no
    /// workspace row or it isn't on disk yet.
    fn task_workspace_path(&self, task_id: &str) -> Result<Option<std::path::PathBuf>, Error> {
        let conn = self.conn()?;
        let path: Option<String> = conn
            .query_row(
                "SELECT w.path FROM tasks t
                   JOIN workspaces w ON w.id = t.workspace_id
                  WHERE t.id = ?1",
                [task_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        Ok(path
            .map(std::path::PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty()))
    }

    /// Agent-authored comment (§14 tool surface): validated against the
    /// task's workspace before persisting — path containment (no escape),
    /// the file must exist on disk, and the anchor must be a non-empty,
    /// in-range, ordered line span. Attribution lands in `created_by` as
    /// `agent:<provider>`. Guardrail failures are typed
    /// ([`Error::InvalidLineComment`] / [`Error::PathEscape`]).
    pub fn add_agent_comment(
        &self,
        opts: AddLineCommentOptions,
        provider: &str,
    ) -> Result<LineComment, Error> {
        let worktree = self.task_workspace_path(&opts.task_id)?.ok_or_else(|| {
            Error::InvalidLineComment(format!(
                "task {} has no materialized workspace to validate against",
                opts.task_id
            ))
        })?;

        validate_comment_anchor(&worktree, &opts.file_path, opts.line_number, opts.line_end)?;

        let mut with_author = opts;
        with_author.created_by = Some(format!("agent:{provider}"));
        self.add(with_author)
    }
}

/// Anchor guardrails shared by the agent tool: lexical+canonical path
/// containment, file existence, and a non-empty in-range line span.
/// `line_count` caps the anchor at the file's real length so an agent can't
/// point past EOF.
fn validate_comment_anchor(
    worktree: &std::path::Path,
    rel_path: &str,
    line_number: i64,
    line_end: Option<i64>,
) -> Result<(), Error> {
    // Containment is owned by files::resolve_contained. The worktree is
    // canonicalized first so a broken task workspace reads as a task-state
    // problem, not a bad anchor path; the remaining io failure mode is the
    // target itself not resolving.
    let canonical_worktree = worktree
        .canonicalize()
        .map_err(|e| Error::InvalidLineComment(format!("worktree not resolvable: {e}")))?;
    let resolved = crate::files::resolve_contained(
        &canonical_worktree,
        rel_path,
        crate::files::ResolveMode::MustExist,
    )
    .map_err(|e| match e {
        Error::PathEscape(_) => e,
        _ => Error::InvalidLineComment(format!("file not found in workspace: {rel_path}")),
    })?;

    // Anchor range.
    if line_number < 1 {
        return Err(Error::InvalidLineComment(format!(
            "line_number {line_number} is out of range"
        )));
    }
    let end = line_end.unwrap_or(line_number);
    if end < line_number {
        return Err(Error::InvalidLineComment(format!(
            "line_end {end} before line_number {line_number}"
        )));
    }
    let line_count = count_lines(&resolved)?;
    if (line_number as usize) > line_count {
        return Err(Error::InvalidLineComment(format!(
            "line_number {line_number} past end of file ({line_count} lines)"
        )));
    }
    Ok(())
}

/// Line count of a file (counts newline-terminated lines plus a trailing
/// partial line). Bounded read — a huge file still streams line by line.
fn count_lines(path: &std::path::Path) -> Result<usize, Error> {
    use std::io::BufRead;
    let file = std::fs::File::open(path)?;
    let mut count = 0usize;
    for _ in std::io::BufReader::new(file).lines() {
        count += 1;
    }
    Ok(count)
}

/// Context for the §14 agent prompt template.
pub struct CommentPromptContext<'a> {
    pub file_path: &'a str,
    /// Current branch of the reviewed worktree (omitted when unknown).
    pub branch: Option<&'a str>,
    /// Best-effort enclosing function/class line (omitted when unknown).
    pub enclosing_function: Option<&'a str>,
    pub line_start: i64,
    pub line_end: i64,
    /// The selected code verbatim.
    pub selected_code: &'a str,
    pub comment: &'a str,
}

/// Builds the task's initial prompt — the ARCHITECTURE.md §14 template
/// verbatim, omitting the BRANCH / ENCLOSING FUNCTION lines when unknown.
pub fn build_comment_prompt(ctx: &CommentPromptContext<'_>) -> String {
    let mut out = String::from("You are reviewing code in a git diff.\n\n");
    out.push_str(&format!("FILE: {}\n", ctx.file_path));
    if let Some(branch) = ctx.branch {
        out.push_str(&format!("BRANCH: {branch}\n"));
    }
    if let Some(enclosing) = ctx.enclosing_function {
        out.push_str(&format!("ENCLOSING FUNCTION: {enclosing}\n"));
    }
    let range = if ctx.line_start == ctx.line_end {
        format!("line {}", ctx.line_start)
    } else {
        format!("lines {}-{}", ctx.line_start, ctx.line_end)
    };
    out.push_str(&format!(
        "\nSELECTED CODE ({range}):\n{}\n\nCOMMENT FROM REVIEWER:\n{}\n\nTASK:\nFix the code based on the comment above. Write the corrected implementation\nand verify it compiles.",
        ctx.selected_code, ctx.comment
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDb;
    use crate::events::BroadcastEventBus;

    fn fixture() -> Arc<LineCommentStore> {
        let db: Arc<dyn Db> = SqliteDb::init(Some(":memory:")).unwrap();
        let bus = Arc::new(BroadcastEventBus::new(16));
        // Project FK target for task rows inserted below.
        db.conn()
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO projects (id, name, path) VALUES ('p1', 'demo', '/tmp/demo')",
                [],
            )
            .unwrap();
        Arc::new(LineCommentStore::new(db, bus))
    }

    fn add_opts(task_id: &str) -> AddLineCommentOptions {
        AddLineCommentOptions {
            task_id: task_id.into(),
            file_path: "src/main.rs".into(),
            line_number: 42,
            line_end: Some(56),
            source_side: SourceSide::After,
            line_content: Some("let x = unwrap();".into()),
            content: "use Result instead".into(),
            created_by: None,
        }
    }

    #[test]
    fn add_and_list_round_trip() {
        let store = fixture();
        let task_id = "task-1";
        // Insert the task directly (bypasses the projects FK for unit scope).
        store.conn().unwrap().execute(
            "INSERT INTO tasks (id, project_id, name, status) VALUES ('task-1', 'p1', 't', 'in_progress')",
            [],
        ).unwrap();

        let c = store.add(add_opts(task_id)).unwrap();
        assert!(c.id.starts_with("lc_"));
        assert_eq!(c.line_end, Some(56));
        assert_eq!(c.source_side, SourceSide::After);
        assert_eq!(c.created_by, "user");
        assert!(!c.resolved);

        let all = store.list_for_task(task_id, None).unwrap();
        assert_eq!(all.len(), 1);
        let by_file = store.list_for_task(task_id, Some("src/main.rs")).unwrap();
        assert_eq!(by_file.len(), 1);
        assert!(store
            .list_for_task(task_id, Some("other.rs"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn list_orders_by_file_then_line() {
        let store = fixture();
        store.conn().unwrap().execute(
            "INSERT INTO tasks (id, project_id, name, status) VALUES ('task-1', 'p1', 't', 'in_progress')",
            [],
        ).unwrap();

        // Insert out of order to prove the ORDER BY, not insertion order.
        for (file, line) in [("b.rs", 9), ("a.rs", 20), ("b.rs", 2), ("a.rs", 3)] {
            let mut opts = add_opts("task-1");
            opts.file_path = file.into();
            opts.line_number = line;
            opts.line_end = None;
            store.add(opts).unwrap();
        }

        let all = store.list_for_task("task-1", None).unwrap();
        let order: Vec<(&str, i64)> = all
            .iter()
            .map(|c| (c.file_path.as_str(), c.line_number))
            .collect();
        assert_eq!(
            order,
            vec![("a.rs", 3), ("a.rs", 20), ("b.rs", 2), ("b.rs", 9)]
        );
    }

    #[test]
    fn list_filters_by_file() {
        let store = fixture();
        store.conn().unwrap().execute(
            "INSERT INTO tasks (id, project_id, name, status) VALUES ('task-1', 'p1', 't', 'in_progress')",
            [],
        ).unwrap();

        for file in ["a.rs", "b.rs", "a.rs"] {
            let mut opts = add_opts("task-1");
            opts.file_path = file.into();
            store.add(opts).unwrap();
        }

        let a = store.list_for_task("task-1", Some("a.rs")).unwrap();
        assert_eq!(a.len(), 2);
        assert!(a.iter().all(|c| c.file_path == "a.rs"));
        assert_eq!(
            store.list_for_task("task-1", Some("b.rs")).unwrap().len(),
            1
        );
        // Exact match only — no path-prefix or glob semantics.
        assert!(store.list_for_task("task-1", Some("a")).unwrap().is_empty());
    }

    #[test]
    fn list_is_scoped_to_the_task() {
        let store = fixture();
        store
            .conn()
            .unwrap()
            .execute_batch(
                "INSERT INTO tasks (id, project_id, name, status) VALUES
                ('task-1', 'p1', 't1', 'in_progress'),
                ('task-2', 'p1', 't2', 'in_progress');",
            )
            .unwrap();

        store.add(add_opts("task-1")).unwrap();
        store.add(add_opts("task-2")).unwrap();

        let one = store.list_for_task("task-1", None).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].task_id, "task-1");
        // Same file filter, other task's comment stays invisible.
        assert_eq!(
            store
                .list_for_task("task-1", Some("src/main.rs"))
                .unwrap()
                .len(),
            1
        );
        // Unknown task is an empty list, not an error.
        assert!(store.list_for_task("nope", None).unwrap().is_empty());
    }

    #[test]
    fn list_round_trips_all_fields() {
        let store = fixture();
        store.conn().unwrap().execute(
            "INSERT INTO tasks (id, project_id, name, status) VALUES ('task-1', 'p1', 't', 'in_progress')",
            [],
        ).unwrap();
        let mut opts = add_opts("task-1");
        opts.source_side = SourceSide::Before;
        opts.created_by = Some("agent:conv-1".into());
        let added = store.add(opts).unwrap();

        let listed = store.list_for_task("task-1", None).unwrap();
        assert_eq!(listed, vec![added]);
        assert_eq!(listed[0].source_side, SourceSide::Before);
        assert_eq!(listed[0].created_by, "agent:conv-1");
        assert_eq!(listed[0].line_content.as_deref(), Some("let x = unwrap();"));
    }

    #[test]
    fn add_validates_inputs() {
        let store = fixture();
        store.conn().unwrap().execute(
            "INSERT INTO tasks (id, project_id, name, status) VALUES ('task-1', 'p1', 't', 'in_progress')",
            [],
        ).unwrap();

        let mut opts = add_opts("task-1");
        opts.content = "   ".into();
        assert!(store.add(opts).is_err());

        let mut opts = add_opts("task-1");
        opts.line_end = Some(10);
        assert!(store.add(opts).is_err());

        let mut opts = add_opts("missing-task");
        opts.line_number = 1;
        opts.line_end = None;
        assert!(matches!(store.add(opts), Err(Error::TaskNotFound(_))));
    }

    #[test]
    fn resolve_is_manual_and_idempotent() {
        let store = fixture();
        store.conn().unwrap().execute(
            "INSERT INTO tasks (id, project_id, name, status) VALUES ('task-1', 'p1', 't', 'in_progress')",
            [],
        ).unwrap();
        let c = store.add(add_opts("task-1")).unwrap();

        let resolved = store.resolve(&c.id).unwrap();
        assert!(resolved.resolved);
        assert!(resolved.resolved_at.is_some());
        // Idempotent.
        assert!(store.resolve(&c.id).unwrap().resolved);
        assert!(store.resolve("lc_missing").is_err());
    }

    #[test]
    fn link_task_is_bidirectional() {
        let store = fixture();
        store
            .conn()
            .unwrap()
            .execute_batch(
                "INSERT INTO tasks (id, project_id, name, status) VALUES
                ('task-1', 'p1', 'reviewed', 'in_progress'),
                ('task-2', 'p1', 'fix', 'in_progress');",
            )
            .unwrap();
        let c = store.add(add_opts("task-1")).unwrap();

        store.link_task(&c.id, "task-2").unwrap();
        let c = store.get(&c.id).unwrap().unwrap();
        assert_eq!(c.linked_task_id.as_deref(), Some("task-2"));

        let source: Option<String> = store
            .conn()
            .unwrap()
            .query_row(
                "SELECT source_comment_id FROM tasks WHERE id = 'task-2'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source.as_deref(), Some(c.id.as_str()));

        assert!(store.link_task(&c.id, "missing").is_err());
        assert!(store.link_task("lc_missing", "task-2").is_err());
    }

    #[test]
    fn delete_removes_and_errors_on_missing() {
        let store = fixture();
        store.conn().unwrap().execute(
            "INSERT INTO tasks (id, project_id, name, status) VALUES ('task-1', 'p1', 't', 'in_progress')",
            [],
        ).unwrap();
        let c = store.add(add_opts("task-1")).unwrap();
        store.delete(&c.id).unwrap();
        assert!(store.get(&c.id).unwrap().is_none());
        assert!(store.delete(&c.id).is_err());
    }

    #[test]
    fn prompt_matches_section_14_template() {
        let prompt = build_comment_prompt(&CommentPromptContext {
            file_path: "src/auth/middleware.rs",
            branch: Some("fartCode/fix-error-handling-a3f2"),
            enclosing_function: Some("fn validate_token(token: &str) -> Result<Claims, AuthError>"),
            line_start: 42,
            line_end: 56,
            selected_code: "    let claims = validate_token(&token).unwrap();",
            comment: "Propagate errors properly so the caller can handle them.",
        });
        let expected = "You are reviewing code in a git diff.\n\n\
FILE: src/auth/middleware.rs\n\
BRANCH: fartCode/fix-error-handling-a3f2\n\
ENCLOSING FUNCTION: fn validate_token(token: &str) -> Result<Claims, AuthError>\n\n\
SELECTED CODE (lines 42-56):\n    let claims = validate_token(&token).unwrap();\n\n\
COMMENT FROM REVIEWER:\nPropagate errors properly so the caller can handle them.\n\n\
TASK:\nFix the code based on the comment above. Write the corrected implementation\nand verify it compiles.";
        assert_eq!(prompt, expected);

        // Single line + no branch/enclosing → lines omitted.
        let prompt = build_comment_prompt(&CommentPromptContext {
            file_path: "a.rs",
            branch: None,
            enclosing_function: None,
            line_start: 3,
            line_end: 3,
            selected_code: "x",
            comment: "y",
        });
        assert!(prompt.contains("SELECTED CODE (line 3):"));
        assert!(!prompt.contains("BRANCH:"));
        assert!(!prompt.contains("ENCLOSING FUNCTION:"));
    }

    // -- Agent tool (E4-11) --------------------------------------------------

    /// Task whose workspace is a real tempdir with a source file on disk.
    fn agent_fixture() -> (Arc<LineCommentStore>, tempfile::TempDir) {
        let store = fixture();
        let wt = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(wt.path().join("src")).unwrap();
        std::fs::write(
            wt.path().join("src/main.rs"),
            "fn a() {}\nfn b() {}\nfn c() {}\n",
        )
        .unwrap();
        store
            .conn()
            .unwrap()
            .execute_batch(&format!(
                "INSERT INTO tasks (id, project_id, name, status, workspace_id)
                   VALUES ('task-wt', 'p1', 't', 'in_progress', 'ws-1');
                 INSERT INTO workspaces (id, path) VALUES ('ws-1', '{}');",
                wt.path().to_string_lossy()
            ))
            .unwrap();
        (store, wt)
    }

    fn agent_opts(file: &str, line: i64) -> AddLineCommentOptions {
        AddLineCommentOptions {
            task_id: "task-wt".into(),
            file_path: file.into(),
            line_number: line,
            line_end: None,
            source_side: SourceSide::After,
            line_content: None,
            content: "agent feedback".into(),
            created_by: None,
        }
    }

    #[test]
    fn agent_comment_is_attributed_and_persisted() {
        let (store, _wt) = agent_fixture();
        let c = store
            .add_agent_comment(agent_opts("src/main.rs", 2), "claude")
            .unwrap();
        assert_eq!(c.created_by, "agent:claude");
        assert_eq!(c.line_number, 2);
        assert!(!c.resolved);
        // Visible in the task's list like any other comment.
        let listed = store.list_for_task("task-wt", None).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].created_by, "agent:claude");
    }

    #[test]
    fn agent_comment_rejects_path_escape() {
        let (store, _wt) = agent_fixture();
        for bad in ["../out.rs", "/etc/passwd", "src/../../x.rs"] {
            let err = store
                .add_agent_comment(agent_opts(bad, 1), "claude")
                .unwrap_err();
            assert!(matches!(err, Error::PathEscape(_)), "{bad}: got {err:?}");
        }
    }

    #[test]
    fn agent_comment_accepts_curdir_spelling() {
        // Same CurDir policy as files::resolve_contained — `./x` can't
        // escape and must not be rejected (the checks used to disagree).
        let (store, _wt) = agent_fixture();
        let c = store
            .add_agent_comment(agent_opts("./src/main.rs", 1), "claude")
            .unwrap();
        assert_eq!(c.created_by, "agent:claude");
    }

    #[test]
    fn agent_comment_rejects_missing_file() {
        let (store, _wt) = agent_fixture();
        let err = store
            .add_agent_comment(agent_opts("src/nope.rs", 1), "claude")
            .unwrap_err();
        assert!(matches!(err, Error::InvalidLineComment(_)), "{err:?}");
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn agent_comment_rejects_out_of_range_anchor() {
        let (store, _wt) = agent_fixture();
        // File has 3 lines: line 4 is past EOF, line 0 invalid.
        let err = store
            .add_agent_comment(agent_opts("src/main.rs", 4), "claude")
            .unwrap_err();
        assert!(matches!(err, Error::InvalidLineComment(_)));
        assert!(err.to_string().contains("past end"));

        let err = store
            .add_agent_comment(agent_opts("src/main.rs", 0), "claude")
            .unwrap_err();
        assert!(matches!(err, Error::InvalidLineComment(_)));

        // line_end before line_number.
        let mut opts = agent_opts("src/main.rs", 2);
        opts.line_end = Some(1);
        let err = store.add_agent_comment(opts, "claude").unwrap_err();
        assert!(matches!(err, Error::InvalidLineComment(_)));
    }

    #[test]
    fn agent_comment_without_workspace_is_rejected() {
        let store = fixture();
        store.conn().unwrap().execute(
            "INSERT INTO tasks (id, project_id, name, status) VALUES ('task-nows', 'p1', 't', 'in_progress')",
            [],
        ).unwrap();
        let mut opts = agent_opts("src/main.rs", 1);
        opts.task_id = "task-nows".into();
        let err = store.add_agent_comment(opts, "claude").unwrap_err();
        assert!(matches!(err, Error::InvalidLineComment(_)));
        assert!(err.to_string().contains("no materialized workspace"));
    }

    #[test]
    fn task_deletion_cascades_comments() {
        // E4-11 guardrail: FK ON DELETE CASCADE leaves no orphan comments
        // when the reviewed task is removed.
        let store = fixture();
        store
            .conn()
            .unwrap()
            .execute_batch(
                "INSERT INTO tasks (id, project_id, name, status) VALUES
               ('task-a', 'p1', 'a', 'in_progress'),
               ('task-b', 'p1', 'b', 'in_progress');",
            )
            .unwrap();
        store.add(add_opts("task-a")).unwrap();
        store.add(add_opts("task-b")).unwrap();
        assert_eq!(store.list_for_task("task-a", None).unwrap().len(), 1);

        store
            .conn()
            .unwrap()
            .execute("DELETE FROM tasks WHERE id = 'task-a'", [])
            .unwrap();
        // task-a's comment is gone; task-b's survives.
        assert!(store.list_for_task("task-a", None).unwrap().is_empty());
        assert_eq!(store.list_for_task("task-b", None).unwrap().len(), 1);
        let total: i64 = store
            .conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM line_comments", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 1, "no orphan rows after cascade");
    }
}
