//! Numbered SQL migration runner with a sha256 journal (ARCHITECTURE.md §2).
//!
//! Mirrors the reference (`initialize.ts` + `drizzle/meta/_journal.json`):
//! - journal JSON + SQL files are embedded at build time (`include_str!`), so
//!   a compiled binary is self-contained;
//! - the runner applies journal entries newer than `MAX(migrations.created_at)`,
//!   splitting each file on `--> statement-breakpoint`;
//! - each applied migration records `sha256(sql)`; already-applied migrations
//!   are hash-verified on every init, so hand-editing a numbered migration
//!   after it has been applied fails loudly instead of silently diverging.
//!
//! FTS tables are created *outside* migrations, version-gated via `kv` keys
//! (`fts_version`, `file_index_version`) — later tickets read those gates.

use rusqlite::OptionalExtension;
use serde::Deserialize;

use crate::db::connection::{kv_get_raw, kv_set_raw};
use crate::Error;

/// Names of the kv version gates (must match what later tickets read).
/// Bumped to 4 by E19-03: `item_type` became UNINDEXED. It had been a
/// full-text column, so typing "task" matched every task row through its
/// own type name — noise that only grew as item types multiplied. Nothing
/// reads it as searchable text (the palette uses it for the hint and the
/// navigation branch, both after the row is in hand), and
/// `ensure_fts_tables` rebuilds on a version mismatch with `backfill`
/// repopulating, so the change costs one rebuild.
pub const SEARCH_INDEX_VERSION: &str = "4";
pub const FILE_INDEX_VERSION: &str = "4";

#[derive(Deserialize)]
struct Journal {
    entries: Vec<JournalEntry>,
}

#[derive(Deserialize)]
struct JournalEntry {
    /// Migration identity (ms epoch), stored in `migrations.created_at`.
    when: i64,
    /// Matches the SQL filename prefix, e.g. `0000_initial`.
    tag: String,
}

const JOURNAL_JSON: &str = include_str!("../../migrations/meta/_journal.json");

/// Looks up the embedded SQL for a journal tag.
///
/// When adding a migration: add `migrations/NNNN_name.sql`, append a journal
/// entry, and add a match arm here. Nothing else needs to change.
fn sql_for_tag(tag: &str) -> Option<&'static str> {
    match tag {
        "0000_initial" => Some(include_str!("../../migrations/0000_initial.sql")),
        "0001_line_comments" => Some(include_str!("../../migrations/0001_line_comments.sql")),
        "0002_issues" => Some(include_str!("../../migrations/0002_issues.sql")),
        "0003_issue_external_ref" => {
            Some(include_str!("../../migrations/0003_issue_external_ref.sql"))
        }
        "0004_provider_auth_method" => Some(include_str!(
            "../../migrations/0004_provider_auth_method.sql"
        )),
        "0005_pull_requests" => Some(include_str!("../../migrations/0005_pull_requests.sql")),
        "0006_board_columns" => Some(include_str!("../../migrations/0006_board_columns.sql")),
        "0007_pin_in_progress_advance" => Some(include_str!(
            "../../migrations/0007_pin_in_progress_advance.sql"
        )),
        "0008_column_id_authoritative" => Some(include_str!(
            "../../migrations/0008_column_id_authoritative.sql"
        )),
        "0009_issue_dossier_path" => {
            Some(include_str!("../../migrations/0009_issue_dossier_path.sql"))
        }
        "0010_project_pool_segment" => Some(include_str!(
            "../../migrations/0010_project_pool_segment.sql"
        )),
        "0011_step_ledger" => Some(include_str!("../../migrations/0011_step_ledger.sql")),
        _ => None,
    }
}

fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Applies all pending journal migrations to `conn`.
///
/// Idempotent: re-running on an up-to-date database is a no-op. Verifies the
/// stored hash of every already-applied migration before applying anything new.
pub fn run_migrations(conn: &mut rusqlite::Connection) -> Result<(), Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS migrations (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            hash       TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );",
    )?;

    let last_applied: i64 = conn.query_row(
        "SELECT COALESCE(MAX(created_at), 0) FROM migrations",
        [],
        |row| row.get(0),
    )?;

    let journal: Journal = serde_json::from_str(JOURNAL_JSON)
        .map_err(|e| Error::Migration(format!("corrupt embedded journal: {e}")))?;

    // Verify hashes of already-applied migrations (append-only enforcement).
    for entry in journal.entries.iter().filter(|e| e.when <= last_applied) {
        let sql = sql_for_tag(&entry.tag).ok_or_else(|| {
            Error::Migration(format!(
                "journal references missing migration file for tag '{}'",
                entry.tag
            ))
        })?;
        let stored: Option<String> = conn
            .query_row(
                "SELECT hash FROM migrations WHERE created_at = ?1",
                [entry.when],
                |row| row.get(0),
            )
            .optional()?;
        let Some(stored) = stored else {
            return Err(Error::Migration(format!(
                "migration '{}' (created_at {}) is recorded in the embedded journal but \
                 missing from the migrations table — the journal was tampered with",
                entry.tag, entry.when
            )));
        };
        let expected = sha256_hex(sql);
        if stored != expected {
            return Err(Error::Migration(format!(
                "migration '{}' was modified after being applied (hash mismatch: \
                 stored {stored}, embedded {expected})",
                entry.tag
            )));
        }
    }

    // Apply pending entries, each in its own transaction.
    let mut pending: Vec<&JournalEntry> = journal
        .entries
        .iter()
        .filter(|e| e.when > last_applied)
        .collect();
    pending.sort_by_key(|e| e.when);

    for entry in pending {
        let sql = sql_for_tag(&entry.tag).ok_or_else(|| {
            Error::Migration(format!(
                "journal references missing migration file for tag '{}'",
                entry.tag
            ))
        })?;

        let tx = conn.transaction()?;
        for statement in sql.split("--> statement-breakpoint") {
            let statement = statement.trim();
            if !statement.is_empty() {
                tx.execute_batch(statement)?;
            }
        }
        tx.execute(
            "INSERT INTO migrations (hash, created_at) VALUES (?1, ?2)",
            rusqlite::params![sha256_hex(sql), entry.when],
        )?;
        tx.commit()?;
    }

    Ok(())
}

/// Creates/recreates the FTS tables, gated by `kv` version keys.
///
/// Runs after migrations on every init. If the stored version matches, the
/// tables are assumed present and untouched; otherwise they are dropped and
/// rebuilt, then the gate is bumped. (FTS schema changes bump the constant,
/// never the domain migrations.)
pub fn ensure_fts_tables(conn: &rusqlite::Connection) -> Result<(), Error> {
    let fts_version = kv_get_raw(conn, "fts_version")?;
    if fts_version.as_deref() != Some(SEARCH_INDEX_VERSION) {
        conn.execute_batch(
            "DROP TABLE IF EXISTS search_index;
             CREATE VIRTUAL TABLE search_index USING fts5(
                 item_type UNINDEXED,
                 item_id UNINDEXED,
                 project_id UNINDEXED,
                 task_id UNINDEXED,
                 title,
                 keywords,
                 tokenize='trigram'
             );",
        )?;
        kv_set_raw(conn, "fts_version", SEARCH_INDEX_VERSION)?;
    }

    let file_index_version = kv_get_raw(conn, "file_index_version")?;
    if file_index_version.as_deref() != Some(FILE_INDEX_VERSION) {
        conn.execute_batch(
            "DROP TABLE IF EXISTS workspace_file_index;
             DROP TABLE IF EXISTS workspace_file_index_meta;
             CREATE VIRTUAL TABLE workspace_file_index USING fts5(
                 workspace_id UNINDEXED,
                 file_path,
                 tokenize='trigram'
             );
             CREATE TABLE workspace_file_index_meta (
                 workspace_id TEXT PRIMARY KEY,
                 indexed_at INTEGER NOT NULL
             );",
        )?;
        kv_set_raw(conn, "file_index_version", FILE_INDEX_VERSION)?;
    }

    Ok(())
}
