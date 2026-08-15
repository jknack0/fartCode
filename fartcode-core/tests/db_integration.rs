//! DB integration tests (ARCHITECTURE.md §16).
//!
//! All tests use `tempfile::tempdir()` or `:memory:` — never the real app
//! data path.

use std::sync::{Arc, Mutex};

use fartcode_core::db::schema::{DOMAIN_TABLES, FTS_TABLES};
use fartcode_core::db::{
    parse_versioned, read_versioned, resolve_db_path, serialize_versioned, write_versioned, Db,
    SqliteDb, Versioned,
};
use serde::{Deserialize, Serialize};

/// Serializes env-mutating tests so they don't race each other.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct SampleConfig {
    model: String,
    temperature: f64,
}

impl Versioned for SampleConfig {
    const VERSION: u32 = 1;
}

fn table_exists(conn: &rusqlite::Connection, name: &str) -> bool {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('table', 'view') AND name = ?1",
            [name],
            |row| row.get(0),
        )
        .unwrap();
    count > 0
}

#[test]
fn test_fresh_install_creates_all_tables() {
    let db = SqliteDb::init_in_memory().unwrap();
    let conn = db.conn().lock().unwrap();

    for table in DOMAIN_TABLES {
        assert!(table_exists(&conn, table), "missing domain table: {table}");
    }
    for table in FTS_TABLES {
        assert!(table_exists(&conn, table), "missing FTS table: {table}");
    }
    assert!(
        table_exists(&conn, "migrations"),
        "missing migrations journal"
    );
}

#[test]
fn test_migration_runner_idempotent() {
    // Use temp file — never touch the real DB path.
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");

    let db = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();
    let migrations: i64 = db
        .conn()
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM migrations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        migrations, 15,
        "all journal migrations (0000–0014) should have been applied"
    );

    // Running init again on the same file is a no-op.
    let db2 = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();
    let migrations2: i64 = db2
        .conn()
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM migrations", [], |row| row.get(0))
        .unwrap();
    // Compared against the first count, not a literal: idempotency is the
    // property under test, and a hardcoded number goes stale on every new
    // migration (it did — #142 found this test red on main after 0013).
    assert_eq!(
        migrations2, migrations,
        "re-init must not re-apply migrations"
    );
}

#[test]
fn test_restart_reuses_same_db_file() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");

    {
        let db = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();
        db.kv_set("survives", "yes").unwrap();
    } // connection dropped

    let db2 = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();
    assert_eq!(db2.kv_get("survives").unwrap(), Some("yes".into()));
    assert_eq!(db2.path(), db_path.as_path());
}

#[test]
fn test_wal_mode_enabled() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");
    let db = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();
    let mode: String = db
        .conn()
        .lock()
        .unwrap()
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode.to_lowercase(), "wal");
}

#[test]
fn test_foreign_keys_enforced() {
    let db = SqliteDb::init_in_memory().unwrap();
    let conn = db.conn().lock().unwrap();
    // Inserting a task with a non-existent project must raise an FK violation.
    let result = conn.execute(
        "INSERT INTO tasks (id, project_id, name, status) VALUES ('t1', 'no-such-project', 'x', 'todo')",
        [],
    );
    assert!(result.is_err(), "FK violations must raise");
}

#[test]
fn test_kv_roundtrip() {
    let db = SqliteDb::init_in_memory().unwrap();
    db.kv_set("test_key", "test_value").unwrap();
    assert_eq!(db.kv_get("test_key").unwrap(), Some("test_value".into()));
    assert_eq!(db.kv_get("missing").unwrap(), None);
    db.kv_delete("test_key").unwrap();
    assert_eq!(db.kv_get("test_key").unwrap(), None);
}

#[test]
fn test_ade_db_file_env_override() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("env-override.db");

    std::env::set_var("FARTCODE_DB_FILE", db_path.to_str().unwrap());
    let db = SqliteDb::init(None).unwrap();
    std::env::remove_var("FARTCODE_DB_FILE");

    assert_eq!(db.path(), db_path.as_path());
    assert!(db_path.exists(), "FARTCODE_DB_FILE override must be used");
}

#[test]
fn test_resolve_db_path_precedence() {
    // Explicit arg beats env.
    {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FARTCODE_DB_FILE", "/tmp/from-env.db");
        assert_eq!(
            resolve_db_path(Some("/tmp/from-arg.db")),
            std::path::PathBuf::from("/tmp/from-arg.db")
        );
        std::env::remove_var("FARTCODE_DB_FILE");
    }
    // Env beats default.
    {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FARTCODE_DB_FILE", "/tmp/from-env.db");
        assert_eq!(
            resolve_db_path(None),
            std::path::PathBuf::from("/tmp/from-env.db")
        );
        std::env::remove_var("FARTCODE_DB_FILE");
    }
    // `:memory:` passes through.
    assert_eq!(
        resolve_db_path(Some(":memory:")),
        std::path::PathBuf::from(":memory:")
    );
}

#[test]
fn test_legacy_db_copy_and_secrets_cleared() {
    let tmp = tempfile::tempdir().unwrap();
    // A path containing a single quote exercises the VACUUM INTO escaping.
    let dir = tmp.path().join("user's-data");
    std::fs::create_dir_all(&dir).unwrap();

    // Build a prior-version DB (emdash4.db) carrying an app_secrets table.
    let legacy = dir.join("emdash4.db");
    let conn = rusqlite::Connection::open(&legacy).unwrap();
    conn.execute_batch(
        "CREATE TABLE app_secrets (key TEXT PRIMARY KEY, secret TEXT NOT NULL);
         INSERT INTO app_secrets (key, secret) VALUES ('k', 's');",
    )
    .unwrap();
    drop(conn);

    let dest = dir.join("fartCode.db");
    let db = SqliteDb::init(Some(dest.to_str().unwrap())).unwrap();
    assert!(dest.exists(), "legacy DB must be copied to fartCode.db");

    let conn = db.conn().lock().unwrap();
    // Secrets must be wiped in the copy.
    let secrets: i64 = conn
        .query_row("SELECT COUNT(*) FROM app_secrets", [], |row| row.get(0))
        .unwrap();
    assert_eq!(secrets, 0, "app_secrets must be cleared in the copy");
    // The Phase 0 schema must be present on top of the copy.
    assert!(table_exists(&conn, "projects"));
    assert!(table_exists(&conn, "migrations"));
}

#[test]
fn test_versioned_json_corrupt_value_returns_none() {
    let db = SqliteDb::init_in_memory().unwrap();
    let conn = db.conn().lock().unwrap();

    conn.execute("INSERT INTO workspaces (id) VALUES ('w1')", [])
        .unwrap();
    // Corrupt JSON blob written directly (bypassing the helper).
    conn.execute(
        "UPDATE workspaces SET data = '{not valid json' WHERE id = 'w1'",
        [],
    )
    .unwrap();

    // parse_versioned never panics and yields None.
    let parsed = parse_versioned::<SampleConfig>("workspaces.data", Some("{not valid json"));
    assert_eq!(parsed, None);

    let read = read_versioned::<SampleConfig>(&conn, "workspaces", "data", "w1").unwrap();
    assert_eq!(read, None, "corrupt cell must read as None, never panic");
}

#[test]
fn test_versioned_json_roundtrip() {
    let db = SqliteDb::init_in_memory().unwrap();
    let conn = db.conn().lock().unwrap();

    conn.execute("INSERT INTO workspaces (id) VALUES ('w1')", [])
        .unwrap();

    let value = SampleConfig {
        model: "gpt-4o".into(),
        temperature: 0.7,
    };
    write_versioned(&conn, "workspaces", "data", "w1", &value).unwrap();

    let stored: String = conn
        .query_row("SELECT data FROM workspaces WHERE id = 'w1'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(
        stored.contains("\"version\":1"),
        "writes must serialize the latest version: {stored}"
    );

    let read = read_versioned::<SampleConfig>(&conn, "workspaces", "data", "w1").unwrap();
    assert_eq!(read.as_ref(), Some(&value));

    // Missing rows read as None.
    let missing = read_versioned::<SampleConfig>(&conn, "workspaces", "data", "nope").unwrap();
    assert_eq!(missing, None);

    // serialize_versioned round-trips through parse_versioned.
    let json = serialize_versioned(&value).unwrap();
    let parsed = parse_versioned::<SampleConfig>("workspaces.data", Some(&json));
    assert_eq!(parsed, Some(value));
}

#[test]
fn test_versioned_json_future_and_older_versions_return_none() {
    let db = SqliteDb::init_in_memory().unwrap();
    let conn = db.conn().lock().unwrap();
    conn.execute("INSERT INTO workspaces (id) VALUES ('w1')", [])
        .unwrap();

    // Future version.
    conn.execute(
        "UPDATE workspaces SET data = '{\"version\":99,\"data\":{\"model\":\"x\",\"temperature\":0.5}}' WHERE id = 'w1'",
        [],
    )
    .unwrap();
    assert_eq!(
        read_versioned::<SampleConfig>(&conn, "workspaces", "data", "w1").unwrap(),
        None,
        "future version must read as None"
    );

    // Older version.
    conn.execute(
        "UPDATE workspaces SET data = '{\"version\":0,\"data\":{\"model\":\"x\",\"temperature\":0.5}}' WHERE id = 'w1'",
        [],
    )
    .unwrap();
    assert_eq!(
        read_versioned::<SampleConfig>(&conn, "workspaces", "data", "w1").unwrap(),
        None,
        "older version must read as None until an upgrade chain exists"
    );
}

#[test]
fn test_migration_hash_tamper_detected() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");
    let db = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();

    // Simulate a hand-edit of an applied migration: corrupt the stored hash.
    db.conn()
        .lock()
        .unwrap()
        .execute("UPDATE migrations SET hash = 'deadbeef'", [])
        .unwrap();

    let err = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap_err();
    assert!(
        err.to_string().contains("modified after being applied"),
        "tampered migration must fail loudly: {err}"
    );
}

/// Proves the Db trait is object-safe enough to pass around as `Arc<dyn Db>`.
#[test]
fn test_db_trait_object() {
    let db: Arc<dyn Db> = SqliteDb::init_in_memory().unwrap();
    db.kv_set("trait", "object").unwrap();
    assert_eq!(db.kv_get("trait").unwrap(), Some("object".into()));
    assert!(db.path().as_os_str() == ":memory:");
}

// -- kv_update: atomic read-modify-write (E19-04, #73) -----------------------

/// The composition callers cannot write themselves: `kv_get` then `kv_set`
/// takes the guard twice, and the guard may not be held across them, so a
/// concurrent writer's update is lost in between.
#[test]
fn kv_update_applies_a_read_modify_write() {
    let db = SqliteDb::init_in_memory().unwrap();
    // A fresh key sees `None`.
    db.kv_update("counter", &mut |current| {
        assert_eq!(current, None);
        Ok(Some("1".into()))
    })
    .unwrap();
    assert_eq!(db.kv_get("counter").unwrap().as_deref(), Some("1"));

    // ...and an existing one sees what is there.
    db.kv_update("counter", &mut |current| {
        let n: i32 = current.unwrap().parse().unwrap();
        Ok(Some((n + 1).to_string()))
    })
    .unwrap();
    assert_eq!(db.kv_get("counter").unwrap().as_deref(), Some("2"));

    // Returning None deletes.
    db.kv_update("counter", &mut |_| Ok(None)).unwrap();
    assert_eq!(db.kv_get("counter").unwrap(), None);
}

/// A closure that fails leaves the old value intact rather than clobbering
/// it with a half-written one.
#[test]
fn kv_update_rolls_back_when_the_closure_fails() {
    let db = SqliteDb::init_in_memory().unwrap();
    db.kv_set("k", "original").unwrap();
    let err = db
        .kv_update("k", &mut |_| {
            Err(fartcode_core::Error::Internal("cannot serialize".into()))
        })
        .unwrap_err();
    assert!(err.to_string().contains("cannot serialize"));
    assert_eq!(db.kv_get("k").unwrap().as_deref(), Some("original"));
}

/// Concurrent appenders must not lose each other's writes — the failure
/// mode the two-call spelling has, and the reason the telemetry
/// observation log uses this method.
#[test]
fn concurrent_kv_updates_do_not_lose_appends() {
    let tmp = tempfile::tempdir().unwrap();
    let db: Arc<dyn Db> = SqliteDb::init(Some(tmp.path().join("t.db").to_str().unwrap())).unwrap();
    let threads: Vec<_> = (0..8)
        .map(|i| {
            let db = db.clone();
            std::thread::spawn(move || {
                for j in 0..25 {
                    db.kv_update("log", &mut |current| {
                        let mut value = current.unwrap_or_default();
                        value.push_str(&format!("{i}:{j},"));
                        Ok(Some(value))
                    })
                    .unwrap();
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let value = db.kv_get("log").unwrap().unwrap();
    assert_eq!(
        value.split(',').filter(|s| !s.is_empty()).count(),
        200,
        "every append survived"
    );
}
