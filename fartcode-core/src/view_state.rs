//! View-state persistence (E1-08): per-view UI state (sidebar, panes, open
//! views) in the `kv` table under the `view-state:` namespace, with orphan
//! pruning on boot.
//!
//! Port of the reference `view-state-service.ts`: keys are
//! `view-state:<scope>:<id>` (`task:*`, `project:*`, plus task tab blobs
//! `task:<id>:tabs`). `prune_orphans` drops rows whose project/task no
//! longer exists.

use std::sync::Arc;

use serde_json::Value;

use crate::db::Db;
use crate::Error;

pub const VIEW_STATE_PREFIX: &str = "view-state:";

/// Persists a view-state snapshot for a key (`view-state:<scope>:<id>`).
pub fn save(db: &Arc<dyn Db>, key: &str, snapshot: &Value) -> Result<(), Error> {
    if !key.starts_with(VIEW_STATE_PREFIX) {
        return Err(Error::InvalidTaskInput(format!(
            "view-state key must start with '{VIEW_STATE_PREFIX}': {key}"
        )));
    }
    db.kv_set(key, &snapshot.to_string())
}

/// Reads a view-state snapshot.
pub fn get(db: &Arc<dyn Db>, key: &str) -> Result<Option<Value>, Error> {
    match db.kv_get(key)? {
        Some(raw) => serde_json::from_str(&raw).map(Some).map_err(Error::from),
        None => Ok(None),
    }
}

/// Deletes a view-state snapshot.
pub fn delete(db: &Arc<dyn Db>, key: &str) -> Result<(), Error> {
    db.kv_delete(key)
}

/// Boot-time cleanup (reference `pruneOrphans`): drop view-state rows whose
/// project/task no longer exists. Runs once per app start.
pub fn prune_orphans(db: &Arc<dyn Db>) -> Result<(), Error> {
    let conn = db
        .conn()
        .lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
    // Task blobs (and the dedicated :tabs suffix entries).
    conn.execute(
        "DELETE FROM kv WHERE key LIKE 'view-state:task:%' AND key NOT LIKE 'view-state:task:%:tabs' \
         AND SUBSTR(key, LENGTH('view-state:task:') + 1) NOT IN (SELECT id FROM tasks)",
        [],
    )?;
    conn.execute(
        "DELETE FROM kv WHERE key LIKE 'view-state:task:%:tabs' \
         AND SUBSTR(key, LENGTH('view-state:task:') + 1, LENGTH(key) - LENGTH('view-state:task:') - LENGTH(':tabs')) \
         NOT IN (SELECT id FROM tasks)",
        [],
    )?;
    conn.execute(
        "DELETE FROM kv WHERE key LIKE 'view-state:project:%' \
         AND SUBSTR(key, LENGTH('view-state:project:') + 1) NOT IN (SELECT id FROM projects)",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDb;
    use serde_json::json;

    fn db() -> Arc<dyn Db> {
        SqliteDb::init_in_memory().unwrap()
    }

    #[test]
    fn round_trips_and_rejects_bad_keys() {
        let db = db();
        save(&db, "view-state:project:p1", &json!({"collapsed": true})).unwrap();
        assert_eq!(
            get(&db, "view-state:project:p1").unwrap().unwrap()["collapsed"],
            true
        );
        delete(&db, "view-state:project:p1").unwrap();
        assert!(get(&db, "view-state:project:p1").unwrap().is_none());
        assert!(
            save(&db, "other-key", &Value::Null).is_err(),
            "prefix enforced"
        );
    }

    #[test]
    fn prunes_orphaned_project_and_task_rows() {
        let db = db();
        // Seed some view state.
        save(&db, "view-state:project:ghost-p", &json!({"x": 1})).unwrap();
        save(&db, "view-state:project:live-p", &json!({"x": 1})).unwrap();
        save(&db, "view-state:task:ghost-t", &json!({"x": 1})).unwrap();
        save(&db, "view-state:task:live-t", &json!({"x": 1})).unwrap();
        save(&db, "view-state:task:live-t:tabs", &json!({"tabs": []})).unwrap();
        save(&db, "view-state:task:ghost-t:tabs", &json!({"tabs": []})).unwrap();

        // Live rows: one project + one task (with its :tabs blob).
        let conn = db.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO projects (id, name, path) VALUES ('live-p', 'p', '/p')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (id, project_id, name, status) VALUES ('live-t', 'live-p', 't', 'in_progress')",
            [],
        )
        .unwrap();
        drop(conn);

        prune_orphans(&db).unwrap();

        assert!(get(&db, "view-state:project:live-p").unwrap().is_some());
        assert!(get(&db, "view-state:task:live-t").unwrap().is_some());
        assert!(get(&db, "view-state:task:live-t:tabs").unwrap().is_some());
        assert!(get(&db, "view-state:project:ghost-p").unwrap().is_none());
        assert!(get(&db, "view-state:task:ghost-t").unwrap().is_none());
        assert!(get(&db, "view-state:task:ghost-t:tabs").unwrap().is_none());
    }
}
