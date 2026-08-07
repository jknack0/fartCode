//! Namespaced KV access over the `kv` table (ARCHITECTURE.md §2).
//!
//! Ports the reference's `main/db/kv.ts`: keys are stored as
//! `"{namespace}:{key}"`, values are JSON strings. Used by view-state
//! persistence (E1-08), host-dependency overrides, and the FTS version gates.
//! (App settings themselves live in the dedicated `app_settings` table and go
//! through `settings::service`.)

use std::sync::Arc;

use rusqlite::OptionalExtension;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::db::Db;
use crate::Error;

/// Namespaced KV over the `kv` table.
#[derive(Clone)]
pub struct KvStore {
    db: Arc<dyn Db>,
}

impl KvStore {
    pub fn new(db: Arc<dyn Db>) -> Self {
        Self { db }
    }

    fn key(namespace: &str, key: &str) -> String {
        format!("{namespace}:{key}")
    }

    /// Reads a value; returns `None` for a missing or unparseable cell.
    /// Never errors on bad data — the cell is logged and treated as absent.
    pub fn get<T: DeserializeOwned>(&self, namespace: &str, key: &str) -> Result<Option<T>, Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db mutex poisoned".into()))?;
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM kv WHERE key = ?1",
                [Self::key(namespace, key)],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::from)?;
        match raw {
            None => Ok(None),
            Some(raw) => match serde_json::from_str(&raw) {
                Ok(value) => Ok(Some(value)),
                Err(e) => {
                    tracing::warn!(namespace, key, error = %e, "kv cell is not valid JSON");
                    Ok(None)
                }
            },
        }
    }

    /// Writes a value (JSON-encoded).
    pub fn set<T: Serialize>(&self, namespace: &str, key: &str, value: &T) -> Result<(), Error> {
        let json = serde_json::to_string(value)?;
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db mutex poisoned".into()))?;
        conn.execute(
            "INSERT INTO kv (key, value, updated_at) VALUES (?1, ?2, unixepoch())
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = unixepoch()",
            rusqlite::params![Self::key(namespace, key), json],
        )?;
        Ok(())
    }

    /// Deletes a single key. Missing keys are a no-op.
    pub fn delete(&self, namespace: &str, key: &str) -> Result<(), Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db mutex poisoned".into()))?;
        conn.execute("DELETE FROM kv WHERE key = ?1", [Self::key(namespace, key)])?;
        Ok(())
    }

    /// Deletes every key in a namespace.
    pub fn clear(&self, namespace: &str) -> Result<(), Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db mutex poisoned".into()))?;
        let prefix = format!("{namespace}:");
        conn.execute("DELETE FROM kv WHERE key LIKE ?1", [format!("{prefix}%")])?;
        Ok(())
    }

    /// Returns all `(key, value)` pairs in a namespace, key names without the
    /// namespace prefix.
    pub fn get_all(&self, namespace: &str) -> Result<Vec<(String, Value)>, Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db mutex poisoned".into()))?;
        let prefix = format!("{namespace}:");
        let mut stmt = conn.prepare("SELECT key, value FROM kv WHERE key LIKE ?1")?;
        let rows = stmt.query_map([format!("{prefix}%")], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (full_key, raw) = row?;
            if let Ok(value) = serde_json::from_str(&raw) {
                out.push((full_key.trim_start_matches(&prefix).to_string(), value));
            }
        }
        Ok(out)
    }
}
