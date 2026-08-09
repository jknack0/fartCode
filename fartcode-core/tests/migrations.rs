//! Migration-chain tests (CI job `db` runs this binary: `cargo test --package
//! fartcode-core --test migrations`). Runs the full embedded migration chain from
//! scratch and verifies the FTS version-gated bootstrap.

use fartcode_core::db::schema::{DOMAIN_TABLES, FTS_TABLES};
use fartcode_core::db::{Db, SqliteDb};

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
fn test_full_migration_chain_applies_from_scratch() {
    let db = SqliteDb::init_in_memory().unwrap();
    let conn = db.conn().lock().unwrap();

    // Every domain table from ARCHITECTURE.md §11 + §14.
    for table in DOMAIN_TABLES {
        assert!(table_exists(&conn, table), "missing table: {table}");
    }

    // Journal: all migrations (0000 + 0001 + 0002) applied, with real sha256 hashes.
    let (count, hash_len): (i64, usize) = conn
        .query_row("SELECT COUNT(*), hash FROM migrations", [], |row| {
            Ok((row.get(0)?, row.get::<_, String>(1)?.len()))
        })
        .unwrap();
    // 0000–0009. (Was asserting 8 against a 9-entry journal since #66 added
    // 0008 without bumping it — this binary has been red on main; corrected
    // here rather than left broken under a new migration.)
    assert_eq!(count, 10, "expected all journal migrations applied");
    assert_eq!(hash_len, 64, "sha256 hex must be 64 chars");

    // FTS tables exist and the kv gates are set to the expected versions.
    for table in FTS_TABLES {
        assert!(table_exists(&conn, table), "missing FTS table: {table}");
    }
    let fts_version: String = conn
        .query_row(
            "SELECT value FROM kv WHERE key = 'fts_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let file_index_version: String = conn
        .query_row(
            "SELECT value FROM kv WHERE key = 'file_index_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(fts_version, "3");
    assert_eq!(file_index_version, "4");
}

#[test]
fn test_search_index_is_queryable() {
    let db = SqliteDb::init_in_memory().unwrap();
    let conn = db.conn().lock().unwrap();

    conn.execute(
        "INSERT INTO search_index (item_type, item_id, project_id, task_id, title, keywords)
         VALUES ('task', 't1', 'p1', 't1', 'fix auth bug', 'auth login token')",
        [],
    )
    .unwrap();

    let hits: Vec<String> = conn
        .prepare("SELECT item_id FROM search_index WHERE search_index MATCH 'auth'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        hits,
        vec!["t1".to_string()],
        "trigram MATCH must find the row"
    );
}

#[test]
fn test_fts_version_gate_rebuilds_index() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");

    let db = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();
    let conn = db.conn().lock().unwrap();

    // Simulate an old FTS schema: drop the index and rewind the gate.
    conn.execute_batch(
        "DROP TABLE search_index; UPDATE kv SET value = '2' WHERE key = 'fts_version';",
    )
    .unwrap();
    assert!(!table_exists(&conn, "search_index"));
    drop(conn);

    // Re-init must rebuild the index and bump the gate.
    let db2 = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();
    let conn2 = db2.conn().lock().unwrap();
    assert!(table_exists(&conn2, "search_index"));
    let fts_version: String = conn2
        .query_row(
            "SELECT value FROM kv WHERE key = 'fts_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(fts_version, "3", "gate must be bumped back to current");
}

#[test]
fn test_migrations_reinit_is_noop() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");

    let db = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();
    let count: i64 = db
        .conn()
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM migrations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 10);
    drop(db);

    let db2 = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();
    let count2: i64 = db2
        .conn()
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM migrations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count2, 10, "re-init must not re-apply migrations");
}

/// E19-01 (#70): migration 0009 adds `issues.dossier_path` as a plain
/// nullable column. The test that matters is the UPGRADE, not the fresh
/// chain: a DB whose rows predate the column must survive it, keep its data,
/// and read the new field as NULL — nothing anywhere may assume non-NULL.
#[test]
fn test_dossier_path_upgrade_leaves_existing_rows_intact() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");

    // Stand up a DB, then rewind it to the pre-0009 shape: drop the column
    // and forget the journal entry, so re-init has to apply 0009 for real
    // against rows that already exist.
    {
        let db = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();
        let conn = db.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO projects (id, name, path) VALUES ('p1', 'proj', '/tmp/proj')",
            [],
        )
        .unwrap();
        fartcode_core::issues::columns::seed_default_columns(&conn, "p1").unwrap();
        conn.execute(
            "INSERT INTO issues (id, project_id, title, lane, position)
             VALUES ('iss_old', 'p1', 'predates the column', 'backlog', 0)",
            [],
        )
        .unwrap();
        conn.execute("ALTER TABLE issues DROP COLUMN dossier_path", [])
            .unwrap();
        conn.execute(
            "DELETE FROM migrations WHERE created_at = 1800000000009",
            [],
        )
        .unwrap();
    }

    let db = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();
    let conn = db.conn().lock().unwrap();
    let (title, dossier): (String, Option<String>) = conn
        .query_row(
            "SELECT title, dossier_path FROM issues WHERE id = 'iss_old'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(title, "predates the column", "existing data untouched");
    assert_eq!(dossier, None, "backfilled to NULL, and NULL is legal");

    // And the read path (the shared SELECT column list) handles the NULL.
    drop(conn);
    let store = fartcode_core::issues::IssueStore::new(
        db.clone(),
        std::sync::Arc::new(fartcode_core::events::BroadcastEventBus::new(8)),
    );
    let issue = store.get("iss_old").unwrap().unwrap();
    assert_eq!(issue.dossier_path, None);
    assert_eq!(issue.title, "predates the column");
}
