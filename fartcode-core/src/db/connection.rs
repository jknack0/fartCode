//! Rusqlite connection singleton, PRAGMAs, and path resolution
//! (ARCHITECTURE.md §2, §6.1, §17).
//!
//! - PRAGMAs: `foreign_keys = ON`, `journal_mode = WAL`, `busy_timeout = 5000`
//!   (file-backed DBs) or `journal_mode = MEMORY` (`:memory:` / tests).
//! - Path resolution: explicit `db_path` arg > `FARTCODE_DB_FILE` env override >
//!   per-platform default (`~/Library/Application Support/fartCode` on macOS,
//!   `%APPDATA%/fartCode` on Windows, `$XDG_CONFIG_HOME/fartCode` on Linux).
//! - Legacy copy: if `fartCode.db` is missing but a prior-version DB
//!   (`emdash4.db` / `emdash3.db`) exists next to it, copy it via
//!   `VACUUM INTO` (readonly source) and clear the `app_secrets` table in the
//!   copy.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::db::migrations::{ensure_fts_tables, run_migrations};
use crate::Error;

/// Single connection to the SQLite database, created once at startup and
/// shared via `Arc`. The underlying connection lives behind a mutex so it can
/// be used from multiple threads.
pub trait Db: Send + Sync {
    /// Returns a reference to the underlying rusqlite connection.
    /// Callers use this for queries; the connection mutex is internal.
    ///
    /// Do **not** hold the returned guard while calling other `Db` methods
    /// (`kv_get`/`kv_set`/`kv_delete`) — the mutex is not reentrant and that
    /// would self-deadlock. Take the guard, do the query, and release it.
    fn conn(&self) -> &Mutex<Connection>;

    /// Direct KV access (app_settings + kv tables are the same shape).
    fn kv_get(&self, key: &str) -> Result<Option<String>, Error>;
    fn kv_set(&self, key: &str, value: &str) -> Result<(), Error>;
    fn kv_delete(&self, key: &str) -> Result<(), Error>;

    /// Read-modify-write of one kv value **under a single connection
    /// guard**, inside a transaction.
    ///
    /// The obvious spelling — `kv_get`, edit, `kv_set` — takes the guard
    /// twice, and a caller cannot close that window itself: the guard may
    /// not be held across `kv_get`/`kv_set` (the mutex is not reentrant),
    /// so there is no way to compose them safely from outside. Two writers
    /// racing on one key therefore lose an update, which for an
    /// append-style value means losing whatever the other writer appended.
    ///
    /// `update` receives the current value (`None` when the key is unset)
    /// and returns the new one; returning `None` deletes the key. An `Err`
    /// from `update` rolls the transaction back and is returned unchanged,
    /// so a caller that cannot serialize its new value leaves the old one
    /// intact rather than half-written.
    ///
    /// `&mut dyn FnMut` rather than a generic so `Db` stays object-safe —
    /// every consumer holds it as `Arc<dyn Db>`.
    fn kv_update(
        &self,
        key: &str,
        update: &mut dyn FnMut(Option<String>) -> Result<Option<String>, Error>,
    ) -> Result<(), Error>;

    /// Path to the database file (`:memory:` for in-memory DBs).
    fn path(&self) -> &Path;
}

/// Concrete implementation (not a trait object at runtime — just a struct).
#[derive(Debug)]
pub struct SqliteDb {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl SqliteDb {
    /// Initializes the database: opens/creates, sets PRAGMAs, runs migrations,
    /// and bootstraps the FTS tables. Called exactly once at app startup.
    pub fn init(db_path: Option<&str>) -> Result<Arc<Self>, Error> {
        let path = resolve_db_path(db_path);
        if path.as_os_str() == ":memory:" {
            return Self::init_in_memory();
        }
        maybe_copy_legacy_db(&path)?;
        let mut conn = open_connection(&path)?;
        run_migrations(&mut conn)?;
        ensure_fts_tables(&conn)?;
        Ok(Arc::new(Self {
            conn: Mutex::new(conn),
            path,
        }))
    }

    /// Creates an in-memory database for tests. Runs all migrations and the
    /// FTS bootstrap, and returns a fully-initialized DB.
    pub fn init_in_memory() -> Result<Arc<Self>, Error> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = MEMORY;")?;
        run_migrations(&mut conn)?;
        ensure_fts_tables(&conn)?;
        Ok(Arc::new(Self {
            conn: Mutex::new(conn),
            path: PathBuf::from(":memory:"),
        }))
    }
}

impl Db for SqliteDb {
    fn conn(&self) -> &Mutex<Connection> {
        &self.conn
    }

    fn kv_get(&self, key: &str) -> Result<Option<String>, Error> {
        let conn = lock_conn(&self.conn)?;
        kv_get_raw(&conn, key)
    }

    fn kv_set(&self, key: &str, value: &str) -> Result<(), Error> {
        let conn = lock_conn(&self.conn)?;
        kv_set_raw(&conn, key, value)
    }

    fn kv_delete(&self, key: &str) -> Result<(), Error> {
        let conn = lock_conn(&self.conn)?;
        kv_delete_raw(&conn, key)
    }

    fn kv_update(
        &self,
        key: &str,
        update: &mut dyn FnMut(Option<String>) -> Result<Option<String>, Error>,
    ) -> Result<(), Error> {
        let mut conn = lock_conn(&self.conn)?;
        // One guard for the whole read-modify-write, and a transaction so a
        // crash between the read and the write cannot leave the value
        // half-applied.
        let tx = conn.transaction()?;
        let current = kv_get_raw(&tx, key)?;
        match update(current)? {
            Some(next) => kv_set_raw(&tx, key, &next)?,
            None => kv_delete_raw(&tx, key)?,
        }
        tx.commit()?;
        Ok(())
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

/// Raw kv helpers on a bare connection (used by the trait impl and by the
/// FTS bootstrap in `migrations.rs`).
pub(crate) fn kv_get_raw(conn: &Connection, key: &str) -> Result<Option<String>, Error> {
    Ok(conn
        .query_row("SELECT value FROM kv WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?)
}

pub(crate) fn kv_set_raw(conn: &Connection, key: &str, value: &str) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO kv (key, value, updated_at) VALUES (?1, ?2, unixepoch())
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = unixepoch()",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

pub(crate) fn kv_delete_raw(conn: &Connection, key: &str) -> Result<(), Error> {
    conn.execute("DELETE FROM kv WHERE key = ?1", [key])?;
    Ok(())
}

fn lock_conn(conn: &Mutex<Connection>) -> Result<MutexGuard<'_, Connection>, Error> {
    conn.lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))
}

/// Resolves the database file path: explicit arg > `FARTCODE_DB_FILE` > platform
/// default. A `:memory:` value is passed through untouched.
pub fn resolve_db_path(db_path: Option<&str>) -> PathBuf {
    if let Some(p) = db_path.map(str::trim).filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    if let Ok(env) = std::env::var("FARTCODE_DB_FILE") {
        let env = env.trim();
        if !env.is_empty() {
            return PathBuf::from(env);
        }
    }
    default_db_path()
}

fn default_db_path() -> PathBuf {
    let data_dir = if cfg!(target_os = "macos") {
        home_dir().join("Library/Application Support/fartCode")
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| home_dir().join("AppData/Roaming"))
            .join("fartCode")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| home_dir().join(".config"))
            .join("fartCode")
    };
    data_dir.join("fartCode.db")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn open_connection(path: &Path) -> Result<Connection, Error> {
    if path.as_os_str() == ":memory:" {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = MEMORY;")?;
        return Ok(conn);
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    conn.busy_timeout(Duration::from_millis(5000))?;
    Ok(conn)
}

/// Prior-version DB filenames (reference app lineage), in order of preference.
const LEGACY_DB_FILENAMES: &[&str] = &["emdash4.db", "emdash3.db"];

/// If `dest` does not exist and a prior-version DB sits next to it, copies it
/// via `VACUUM INTO` (readonly source) and clears `app_secrets` in the copy.
///
/// Note: a copied reference DB is *not* schema-identical to the Phase 0
/// schema; the real data-migration path is a later-phase concern (§11
/// `messages` comment). If a legacy DB is found, `init` will surface any
/// migration conflict loudly instead of silently corrupting data.
fn maybe_copy_legacy_db(dest: &Path) -> Result<(), Error> {
    if dest.exists() {
        return Ok(());
    }
    let Some(dir) = dest.parent() else {
        return Ok(());
    };
    if dir.as_os_str().is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    for name in LEGACY_DB_FILENAMES {
        let src = dir.join(name);
        if src.exists() {
            // Note: a source left in WAL mode can fail VACUUM INTO with a
            // SQLITE_CANTOPEN (it needs its -wal/-shm sidecars). Cleanly
            // closed reference DBs have checkpointed WAL data in the main
            // file and copy fine; the failure is loud either way.
            copy_sqlite_database(&src, dest)?;
            clear_copied_app_secrets(dest)?;
            tracing::info!(
                src = %src.display(),
                dest = %dest.display(),
                "copied legacy database and cleared app secrets"
            );
            return Ok(());
        }
    }
    Ok(())
}

fn copy_sqlite_database(src: &Path, dest: &Path) -> Result<(), Error> {
    let source = Connection::open_with_flags(src, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.busy_timeout(Duration::from_millis(5000))?;
    // VACUUM INTO fails if the destination already exists; we only call this
    // when `dest` does not exist. Escape single quotes per SQLite rules.
    let dest_sql = dest
        .to_str()
        .ok_or_else(|| Error::Internal("non-UTF-8 db path".into()))?
        .replace('\'', "''");
    source.execute_batch(&format!("VACUUM INTO '{dest_sql}';"))?;
    drop(source);
    Ok(())
}

fn clear_copied_app_secrets(path: &Path) -> Result<(), Error> {
    let conn = Connection::open(path)?;
    let has_secrets: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'app_secrets'",
        [],
        |row| row.get(0),
    )?;
    if has_secrets > 0 {
        conn.execute_batch("DELETE FROM app_secrets;")?;
    }
    Ok(())
}
