//! Workspaces domain (ARCHITECTURE.md §2): the one home for SQL against the
//! `workspaces` table.
//!
//! Before this module, `FROM workspaces` was hand-rolled at a dozen call
//! sites, each restating the row conventions. The conventions live here
//! exactly once:
//!
//! - **`location` defaults to `'local'`** — a NULL column means local. The
//!   rule is applied in [`row_from`], the single row mapper every read goes
//!   through; nothing else may re-derive it.
//! - `path`, `kind`, and `config` are nullable; [`WorkspaceRow::local_path`]
//!   is the shared "has a non-empty stored path" filter. It does NOT check
//!   the directory exists — callers with a materialization requirement layer
//!   their own `is_dir` (they disagree about the fallback, see the audit).
//!
//! Three access layers, narrowest visibility wins:
//! - [`WorkspaceStore`] over `Arc<dyn Db>` — the public API; one connection
//!   guard per call.
//! - `pub(crate)` fns over `&dyn Db` — for core's free-function modules
//!   (`fs_watch`, `projects::adoption`, `projects` pool checks).
//! - `pub(crate)` fns over `&rusqlite::Connection` — for sites composing a
//!   transaction or a single-guard read-modify-write (project delete,
//!   `ensure_repository_workspace`, the BYOI provision record). The other
//!   two layers are these fns under their own guard, so every operation has
//!   one SQL string.
//!
//! A poisoned connection mutex is `Error::Internal` here (the DB stores'
//! majority convention); a few pre-port sites recovered the guard instead.
//!
//! Specialized reads stay with their domains by design: the BYOI join
//! (`kind = 'byoi'` + ssh routing, `tasks::byoi`), the remote-target join
//! (`location = 'remote'`, `projects::remote`), and the deletion snapshot
//! (`tasks`) select different shapes and would not collapse onto this API.

use std::path::{Path, PathBuf};
use std::sync::{Arc, MutexGuard};

use rusqlite::{Connection, OptionalExtension};

use crate::db::Db;
use crate::Error;

/// One `workspaces` row, in the shape the call sites actually read.
/// (`ssh_connection_id`/`branch_name`/counters stay domain-specific — see
/// the module doc.)
#[derive(Debug, Clone)]
pub struct WorkspaceRow {
    pub id: String,
    /// Stored path — may be NULL (unprovisioned/BYOI) or stale on disk.
    pub path: Option<String>,
    /// `worktree` | `project-root` | `byoi`; NULL only on legacy rows.
    pub kind: Option<String>,
    /// `local` | `remote`; never empty — the NULL default is applied by the
    /// row mapper.
    pub location: String,
    /// Versioned JSON (workspace intent / BYOI machine record).
    pub config: Option<String>,
}

impl WorkspaceRow {
    /// E12-04 vocabulary: `location = 'remote'` routes terminals/agents over
    /// SSH.
    pub fn is_remote(&self) -> bool {
        self.location == "remote"
    }

    /// The stored path when non-NULL and non-empty. Not checked against disk.
    pub fn local_path(&self) -> Option<PathBuf> {
        self.path
            .as_deref()
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
    }
}

/// Insert shape. Optional columns default to NULL (`created_at`/`updated_at`
/// default to now in the schema).
#[derive(Debug, Clone, Default)]
pub struct NewWorkspace<'a> {
    pub id: &'a str,
    /// Dedup key (`sha256("local:<path>")` etc.) — repository workspaces only.
    pub key: Option<&'a str>,
    pub r#type: &'a str,
    pub kind: &'a str,
    pub location: &'a str,
    pub path: Option<&'a str>,
    pub ssh_connection_id: Option<&'a str>,
    pub config: Option<&'a str>,
}

/// A task/workspace pair the app layer should register with the file
/// watcher at boot or on provision (`fs_watch` re-exports this).
#[derive(Debug, Clone)]
pub struct WatchTarget {
    pub task_id: String,
    pub project_id: String,
    pub workspace_id: String,
    pub worktree: PathBuf,
}

const ROW_COLUMNS: &str = "id, path, kind, location, config";

/// THE row mapper: every read of a workspace row funnels through here, so
/// the location default exists in exactly one place.
fn row_from(row: &rusqlite::Row) -> rusqlite::Result<WorkspaceRow> {
    Ok(WorkspaceRow {
        id: row.get(0)?,
        path: row.get(1)?,
        kind: row.get(2)?,
        location: row
            .get::<_, Option<String>>(3)?
            .unwrap_or_else(|| "local".into()),
        config: row.get(4)?,
    })
}

fn lock(db: &dyn Db) -> Result<MutexGuard<'_, Connection>, Error> {
    db.conn()
        .lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))
}

// -- Connection-level (transaction / single-guard composition) --------------

pub(crate) fn get_row(conn: &Connection, id: &str) -> Result<Option<WorkspaceRow>, Error> {
    Ok(conn
        .query_row(
            &format!("SELECT {ROW_COLUMNS} FROM workspaces WHERE id = ?1"),
            [id],
            row_from,
        )
        .optional()?)
}

pub(crate) fn insert_row(conn: &Connection, ws: &NewWorkspace) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO workspaces (id, key, type, kind, location, path, ssh_connection_id, config)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            ws.id,
            ws.key,
            ws.r#type,
            ws.kind,
            ws.location,
            ws.path,
            ws.ssh_connection_id,
            ws.config,
        ],
    )?;
    Ok(())
}

/// Deletes the row only — derived state (`workspace_file_index*`) is the
/// task-deletion flow's to clean.
pub(crate) fn delete_row(conn: &Connection, id: &str) -> Result<(), Error> {
    conn.execute("DELETE FROM workspaces WHERE id = ?1", [id])?;
    Ok(())
}

/// Repository-workspace dedup lookup (`idx_workspaces_key`).
pub(crate) fn id_by_key(conn: &Connection, key: &str) -> Result<Option<String>, Error> {
    Ok(conn
        .query_row("SELECT id FROM workspaces WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?)
}

// -- &dyn Db level (core's free-function modules) ---------------------------

/// `(workspace_id, path)` of a project's `kind = 'worktree'` rows with a
/// stored path (pool adoption). The sibling below is the same join with the
/// project filter inverted.
pub(crate) fn worktree_rows_for_project(
    db: &dyn Db,
    project_id: &str,
) -> Result<Vec<(String, String)>, Error> {
    let conn = lock(db)?;
    let mut stmt = conn.prepare(
        "SELECT w.id, w.path FROM workspaces w
         JOIN tasks t ON t.workspace_id = w.id
         WHERE t.project_id = ?1 AND w.kind = 'worktree' AND w.path IS NOT NULL",
    )?;
    let rows = stmt.query_map([project_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Worktree paths belonging to every OTHER project (F2b foreign-worktree
/// guard before a pool teardown).
pub(crate) fn worktree_paths_of_other_projects(
    db: &dyn Db,
    project_id: &str,
) -> Result<Vec<String>, Error> {
    let conn = lock(db)?;
    let mut stmt = conn.prepare(
        "SELECT w.path FROM workspaces w
         JOIN tasks t ON t.workspace_id = w.id
         WHERE t.project_id != ?1 AND w.kind = 'worktree' AND w.path IS NOT NULL",
    )?;
    let rows = stmt.query_map([project_id], |row| row.get(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub(crate) fn set_path(db: &dyn Db, id: &str, path: &Path) -> Result<(), Error> {
    let conn = lock(db)?;
    conn.execute(
        "UPDATE workspaces SET path = ?1, updated_at = datetime('now') WHERE id = ?2",
        rusqlite::params![path.to_string_lossy(), id],
    )?;
    Ok(())
}

/// Boot-time targets: every non-archived task whose workspace has a stored
/// path. Stale rows (paths gone from disk) fail registration individually
/// and are skipped by the caller.
pub(crate) fn watch_targets(db: &dyn Db) -> Result<Vec<WatchTarget>, Error> {
    let conn = lock(db)?;
    let mut stmt = conn.prepare(
        "SELECT t.id, t.project_id, t.workspace_id, w.path
         FROM tasks t JOIN workspaces w ON w.id = t.workspace_id
         WHERE t.archived_at IS NULL AND w.path IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(WatchTarget {
            task_id: row.get(0)?,
            project_id: row.get(1)?,
            workspace_id: row.get(2)?,
            worktree: PathBuf::from(row.get::<_, String>(3)?),
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Target for a just-provisioned task (`TaskProvisioned` handler). `None`
/// when the workspace has no stored path (e.g. remote/BYOI).
pub(crate) fn watch_target_for(
    db: &dyn Db,
    task_id: &str,
    workspace_id: &str,
) -> Result<Option<WatchTarget>, Error> {
    let conn = lock(db)?;
    conn.query_row(
        "SELECT t.project_id, w.path
         FROM tasks t JOIN workspaces w ON w.id = ?2
         WHERE t.id = ?1",
        [task_id, workspace_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
    )
    .optional()?
    .map_or(Ok(None), |(project_id, path)| {
        Ok(path.map(|p| WatchTarget {
            task_id: task_id.into(),
            project_id,
            workspace_id: workspace_id.into(),
            worktree: PathBuf::from(p),
        }))
    })
}

// -- Public store -----------------------------------------------------------

/// Workspace-row access for services holding the shared DB handle. Cheap to
/// construct per use (one `Arc` clone) — services that cannot grow a field
/// (constructor signatures are wired in `App`) build it on the fly.
#[derive(Clone)]
pub struct WorkspaceStore {
    db: Arc<dyn Db>,
}

impl WorkspaceStore {
    pub fn new(db: Arc<dyn Db>) -> Self {
        Self { db }
    }

    pub fn get(&self, id: &str) -> Result<Option<WorkspaceRow>, Error> {
        let conn = lock(self.db.as_ref())?;
        get_row(&conn, id)
    }

    /// The row's `kind`. `None` for a missing row AND for a NULL kind
    /// (legacy rows only) — callers that must distinguish use [`Self::get`].
    pub fn kind(&self, id: &str) -> Result<Option<String>, Error> {
        Ok(self.get(id)?.and_then(|row| row.kind))
    }

    /// The task's workspace path as stored — non-empty, NOT checked against
    /// disk (see the module doc).
    pub fn path_for_task(&self, task_id: &str) -> Result<Option<PathBuf>, Error> {
        let conn = lock(self.db.as_ref())?;
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
        Ok(path.filter(|p| !p.is_empty()).map(PathBuf::from))
    }

    pub fn insert(&self, ws: &NewWorkspace) -> Result<(), Error> {
        let conn = lock(self.db.as_ref())?;
        insert_row(&conn, ws)
    }

    /// See [`delete_row`]: the row only, not derived index state.
    pub fn delete(&self, id: &str) -> Result<(), Error> {
        let conn = lock(self.db.as_ref())?;
        delete_row(&conn, id)
    }

    pub fn set_path(&self, id: &str, path: &Path) -> Result<(), Error> {
        set_path(self.db.as_ref(), id, path)
    }

    pub fn set_config(&self, id: &str, config: &str) -> Result<(), Error> {
        let conn = lock(self.db.as_ref())?;
        conn.execute(
            "UPDATE workspaces SET config = ?1, updated_at = datetime('now') WHERE id = ?2",
            rusqlite::params![config, id],
        )?;
        Ok(())
    }

    pub fn watch_targets(&self) -> Result<Vec<WatchTarget>, Error> {
        watch_targets(self.db.as_ref())
    }

    pub fn watch_target_for(
        &self,
        task_id: &str,
        workspace_id: &str,
    ) -> Result<Option<WatchTarget>, Error> {
        watch_target_for(self.db.as_ref(), task_id, workspace_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDb;

    fn db() -> Arc<SqliteDb> {
        SqliteDb::init(Some(":memory:")).unwrap()
    }

    fn store(db: &Arc<SqliteDb>) -> WorkspaceStore {
        WorkspaceStore::new(db.clone())
    }

    fn seed_task(db: &SqliteDb, task_id: &str, project_id: &str, workspace_id: &str) {
        let conn = db.conn().lock().unwrap();
        // `projects.path` is unique — derive it from the id so a second
        // project's seed is not silently dropped by OR IGNORE.
        conn.execute(
            "INSERT OR IGNORE INTO projects (id, name, path) VALUES (?1, 'P', '/proj/' || ?1)",
            [project_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (id, project_id, name, status, workspace_id)
             VALUES (?1, ?2, 'T', 'running', ?3)",
            [task_id, project_id, workspace_id],
        )
        .unwrap();
    }

    #[test]
    fn location_defaults_to_local_in_the_one_mapper() {
        let db = db();
        db.conn()
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO workspaces (id, kind) VALUES ('w1', 'worktree')",
                [],
            )
            .unwrap();
        let row = store(&db).get("w1").unwrap().unwrap();
        assert_eq!(row.location, "local");
        assert!(!row.is_remote());
        assert_eq!(row.local_path(), None);
    }

    #[test]
    fn insert_get_delete_round_trip() {
        let db = db();
        let s = store(&db);
        s.insert(&NewWorkspace {
            id: "w1",
            key: Some("k1"),
            r#type: "project-ssh",
            kind: "project-root",
            location: "remote",
            path: Some("/repo"),
            ssh_connection_id: Some("conn1"),
            config: Some("{\"version\":\"2\"}"),
        })
        .unwrap();
        let row = s.get("w1").unwrap().unwrap();
        assert!(row.is_remote());
        assert_eq!(row.kind.as_deref(), Some("project-root"));
        assert_eq!(row.local_path(), Some(PathBuf::from("/repo")));
        assert_eq!(row.config.as_deref(), Some("{\"version\":\"2\"}"));
        assert_eq!(s.kind("w1").unwrap().as_deref(), Some("project-root"));
        {
            let conn = db.conn().lock().unwrap();
            assert_eq!(id_by_key(&conn, "k1").unwrap().as_deref(), Some("w1"));
            assert_eq!(id_by_key(&conn, "nope").unwrap(), None);
        }
        s.delete("w1").unwrap();
        assert!(s.get("w1").unwrap().is_none());
        assert_eq!(s.kind("w1").unwrap(), None);
    }

    #[test]
    fn set_path_and_config_touch_updated_at_columns() {
        let db = db();
        let s = store(&db);
        s.insert(&NewWorkspace {
            id: "w1",
            r#type: "local",
            kind: "worktree",
            location: "local",
            ..Default::default()
        })
        .unwrap();
        s.set_path("w1", Path::new("/wt/one")).unwrap();
        s.set_config("w1", "{\"version\":\"2\"}").unwrap();
        let row = s.get("w1").unwrap().unwrap();
        assert_eq!(row.local_path(), Some(PathBuf::from("/wt/one")));
        assert_eq!(row.config.as_deref(), Some("{\"version\":\"2\"}"));
    }

    #[test]
    fn path_for_task_filters_null_and_empty_paths() {
        let db = db();
        let s = store(&db);
        {
            let conn = db.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO workspaces (id, kind, path) VALUES ('w1', 'worktree', '/wt1')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO workspaces (id, kind) VALUES ('w2', 'byoi')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO workspaces (id, kind, path) VALUES ('w3', 'worktree', '')",
                [],
            )
            .unwrap();
        }
        seed_task(&db, "t1", "p1", "w1");
        seed_task(&db, "t2", "p1", "w2");
        seed_task(&db, "t3", "p1", "w3");
        assert_eq!(s.path_for_task("t1").unwrap(), Some(PathBuf::from("/wt1")));
        assert_eq!(s.path_for_task("t2").unwrap(), None);
        assert_eq!(s.path_for_task("t3").unwrap(), None);
        assert_eq!(s.path_for_task("missing").unwrap(), None);
    }

    #[test]
    fn worktree_joins_filter_kind_path_and_project() {
        let db = db();
        {
            let conn = db.conn().lock().unwrap();
            for (id, kind, path) in [
                ("w1", "worktree", Some("/wt1")),
                ("w2", "project-root", Some("/proj")),
                ("w3", "worktree", None),
                ("w4", "worktree", Some("/other")),
            ] {
                conn.execute(
                    "INSERT INTO workspaces (id, kind, path) VALUES (?1, ?2, ?3)",
                    rusqlite::params![id, kind, path],
                )
                .unwrap();
            }
        }
        seed_task(&db, "t1", "p1", "w1");
        seed_task(&db, "t2", "p1", "w2");
        seed_task(&db, "t3", "p1", "w3");
        seed_task(&db, "t4", "p2", "w4");
        assert_eq!(
            worktree_rows_for_project(db.as_ref(), "p1").unwrap(),
            vec![("w1".to_string(), "/wt1".to_string())]
        );
        assert_eq!(
            worktree_paths_of_other_projects(db.as_ref(), "p1").unwrap(),
            vec!["/other".to_string()]
        );
    }

    #[test]
    fn watch_targets_skip_archived_and_pathless() {
        let db = db();
        let s = store(&db);
        {
            let conn = db.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO workspaces (id, kind, path) VALUES ('w1', 'worktree', '/wt1')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO workspaces (id, kind) VALUES ('w2', 'byoi')",
                [],
            )
            .unwrap();
        }
        seed_task(&db, "t1", "p1", "w1");
        seed_task(&db, "t2", "p1", "w2");
        {
            let conn = db.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO tasks (id, project_id, name, status, workspace_id, archived_at)
                 VALUES ('t3', 'p1', 'A', 'done', 'w1', datetime('now'))",
                [],
            )
            .unwrap();
        }
        let targets = s.watch_targets().unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].task_id, "t1");
        assert_eq!(targets[0].worktree, PathBuf::from("/wt1"));

        let one = s.watch_target_for("t1", "w1").unwrap().unwrap();
        assert_eq!(one.project_id, "p1");
        assert!(s.watch_target_for("t2", "w2").unwrap().is_none());
        assert!(s.watch_target_for("missing", "w1").unwrap().is_none());
    }
}
