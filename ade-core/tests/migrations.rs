//! Migration-chain tests (CI job `db` runs this binary: `cargo test --package
//! ade-core --test migrations`). Runs the full embedded migration chain from
//! scratch and verifies the FTS version-gated bootstrap.

use ade_core::db::schema::{DOMAIN_TABLES, FTS_TABLES};
use ade_core::db::{Db, SqliteDb};

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

    // Journal: both migrations (0000 + 0001) applied, with real sha256 hashes.
    let (count, hash_len): (i64, usize) = conn
        .query_row("SELECT COUNT(*), hash FROM migrations", [], |row| {
            Ok((row.get(0)?, row.get::<_, String>(1)?.len()))
        })
        .unwrap();
    assert_eq!(count, 2, "expected both journal migrations applied");
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
    assert_eq!(count, 2);
    drop(db);

    let db2 = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();
    let count2: i64 = db2
        .conn()
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM migrations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count2, 2, "re-init must not re-apply migrations");
}
