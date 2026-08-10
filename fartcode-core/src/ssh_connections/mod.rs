//! SSH connection profiles (E12-03): one row per saved remote host.
//!
//! Mirrors the [`crate::provider_accounts`] split — the DB row holds only
//! non-secret fields; passwords and key passphrases live in the OS keyring
//! (see [`secrets`]) under `ssh-connection:<id>`. DTOs never carry a secret
//! value, so nothing sensitive can cross a Tauri command boundary.
//!
//! Table: `ssh_connections` (0000 migration). `metadata` is versioned JSON.

pub mod secrets;

use std::sync::Arc;

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::db::{parse_versioned, serialize_versioned, Db, Versioned};
use crate::Error;

// ── Types ───────────────────────────────────────────────

/// How a profile authenticates. Stored as the `auth_type` text column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SshAuthType {
    Agent,
    Password,
    KeyFile,
}

impl SshAuthType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Password => "password",
            Self::KeyFile => "key-file",
        }
    }

    /// Unknown values fall back to `agent` — the auth method that needs no
    /// stored secret, so a corrupt row degrades safely instead of half-
    /// authenticating with a stale password.
    pub fn parse(value: &str) -> Self {
        match value {
            "password" => Self::Password,
            "key-file" | "keyfile" | "key" => Self::KeyFile,
            _ => Self::Agent,
        }
    }

    /// True when the profile needs a keyring entry to connect.
    pub fn needs_secret(self) -> bool {
        matches!(self, Self::Password)
    }
}

/// Versioned `metadata` payload: everything resolved from `ssh -G` or set by
/// the user that has no dedicated column.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SshConnectionMeta {
    /// `~/.ssh/config` alias this profile was resolved from, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_jump: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_command: Option<String>,
    #[serde(default)]
    pub forward_agent: bool,
}

impl Versioned for SshConnectionMeta {
    const VERSION: u32 = 1;
}

/// A stored SSH connection profile (no secret material).
#[derive(Debug, Clone, PartialEq)]
pub struct SshConnection {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: SshAuthType,
    pub private_key_path: Option<String>,
    pub use_agent: bool,
    pub metadata: Option<SshConnectionMeta>,
    pub created_at: String,
    pub updated_at: String,
}

impl SshConnection {
    /// Keyring entry ref for this profile's secret. Safe to display: it is a
    /// lookup name, never the secret.
    pub fn secret_ref(&self) -> String {
        secrets::entry_name(&self.id)
    }
}

/// Fields for [`SshConnectionStore::create`].
#[derive(Debug, Clone)]
pub struct NewSshConnection {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: SshAuthType,
    pub private_key_path: Option<String>,
    pub use_agent: bool,
    pub metadata: Option<SshConnectionMeta>,
    /// Password or key passphrase. Goes straight to the keyring, never the DB.
    pub secret: Option<String>,
}

impl Default for NewSshConnection {
    fn default() -> Self {
        Self {
            name: String::new(),
            host: String::new(),
            port: 22,
            username: String::new(),
            auth_type: SshAuthType::Agent,
            private_key_path: None,
            use_agent: true,
            metadata: None,
            secret: None,
        }
    }
}

/// Partial update. `None` leaves a field untouched.
#[derive(Debug, Clone, Default)]
pub struct SshConnectionPatch {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub auth_type: Option<SshAuthType>,
    /// `Some(None)` clears the key path.
    pub private_key_path: Option<Option<String>>,
    pub use_agent: Option<bool>,
    pub metadata: Option<SshConnectionMeta>,
    /// `Some(None)` deletes the keyring secret.
    pub secret: Option<Option<String>>,
}

// ── Store ───────────────────────────────────────────────

pub struct SshConnectionStore {
    db: Arc<dyn Db>,
}

const COLUMNS: &str = "id, name, host, port, username, auth_type, private_key_path, use_agent, metadata, created_at, updated_at";

/// Tables carrying an `ssh_connection_id` that must not dangle.
const REFERRING_TABLES: &[&str] = &["projects", "workspaces"];

fn connection_from_row(row: &rusqlite::Row) -> rusqlite::Result<SshConnection> {
    let port: i64 = row.get(3)?;
    let use_agent: i64 = row.get(7)?;
    let metadata_cell: Option<String> = row.get(8)?;
    let auth_type: String = row.get(5)?;
    Ok(SshConnection {
        id: row.get(0)?,
        name: row.get(1)?,
        host: row.get(2)?,
        port: u16::try_from(port).unwrap_or(22),
        username: row.get(4)?,
        auth_type: SshAuthType::parse(&auth_type),
        private_key_path: row.get(6)?,
        use_agent: use_agent != 0,
        metadata: parse_versioned::<SshConnectionMeta>("metadata", metadata_cell.as_deref()),
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

fn encode_meta(meta: Option<&SshConnectionMeta>) -> Result<Option<String>, Error> {
    match meta {
        None => Ok(None),
        Some(m) => serialize_versioned(m).map(Some).map_err(|e| match e {
            Error::VersionedJson { reason, .. } => {
                Error::Internal(format!("serialize ssh connection metadata: {reason}"))
            }
            other => other,
        }),
    }
}

impl SshConnectionStore {
    pub fn new(db: Arc<dyn Db>) -> Self {
        Self { db }
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>, Error> {
        self.db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))
    }

    /// Inserts a profile. A provided secret is written to the keyring first, so
    /// a keyring failure leaves no half-usable row behind.
    pub fn create(&self, opts: NewSshConnection) -> Result<SshConnection, Error> {
        if opts.host.trim().is_empty() || opts.username.trim().is_empty() {
            return Err(Error::SshConnection(
                "ssh connection requires host and username".into(),
            ));
        }

        let id = uuid::Uuid::new_v4().to_string();
        if let Some(secret) = opts.secret.as_deref() {
            secrets::store_secret(&id, secret)?;
        }

        let meta_cell = encode_meta(opts.metadata.as_ref())?;
        {
            let conn = self.conn()?;
            conn.execute(
                "INSERT INTO ssh_connections
                   (id, name, host, port, username, auth_type, private_key_path, use_agent, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    id,
                    opts.name,
                    opts.host,
                    opts.port,
                    opts.username,
                    opts.auth_type.as_str(),
                    opts.private_key_path,
                    i64::from(opts.use_agent),
                    meta_cell,
                ],
            )?;
        }

        self.get(&id)?
            .ok_or_else(|| Error::Internal("inserted ssh connection vanished".into()))
    }

    pub fn get(&self, id: &str) -> Result<Option<SshConnection>, Error> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM ssh_connections WHERE id = ?1"),
                [id],
                connection_from_row,
            )
            .optional()?)
    }

    pub fn list(&self) -> Result<Vec<SshConnection>, Error> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM ssh_connections ORDER BY name ASC, created_at ASC"
        ))?;
        let rows = stmt.query_map([], connection_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Applies a partial update. Secret changes go to the keyring only.
    pub fn update(&self, id: &str, patch: SshConnectionPatch) -> Result<SshConnection, Error> {
        let current = self
            .get(id)?
            .ok_or_else(|| Error::SshConnectionNotFound(id.to_string()))?;

        match patch.secret.as_ref() {
            Some(Some(secret)) => secrets::store_secret(id, secret)?,
            Some(None) => secrets::delete_secret(id)?,
            None => {}
        }

        let name = patch.name.unwrap_or(current.name);
        let host = patch.host.unwrap_or(current.host);
        let port = patch.port.unwrap_or(current.port);
        let username = patch.username.unwrap_or(current.username);
        let auth_type = patch.auth_type.unwrap_or(current.auth_type);
        let private_key_path = patch.private_key_path.unwrap_or(current.private_key_path);
        let use_agent = patch.use_agent.unwrap_or(current.use_agent);
        let metadata = patch.metadata.or(current.metadata);
        let meta_cell = encode_meta(metadata.as_ref())?;

        {
            let conn = self.conn()?;
            conn.execute(
                "UPDATE ssh_connections
                    SET name = ?2, host = ?3, port = ?4, username = ?5, auth_type = ?6,
                        private_key_path = ?7, use_agent = ?8, metadata = ?9,
                        updated_at = datetime('now')
                  WHERE id = ?1",
                rusqlite::params![
                    id,
                    name,
                    host,
                    port,
                    username,
                    auth_type.as_str(),
                    private_key_path,
                    i64::from(use_agent),
                    meta_cell,
                ],
            )?;
        }

        self.get(id)?
            .ok_or_else(|| Error::SshConnectionNotFound(id.to_string()))
    }

    /// Number of `projects`/`workspaces` rows pointing at this profile.
    pub fn reference_count(&self, id: &str) -> Result<i64, Error> {
        let conn = self.conn()?;
        let mut total = 0i64;
        for table in REFERRING_TABLES {
            // Table names are compile-time constants, never user input.
            let count: i64 = conn.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE ssh_connection_id = ?1"),
                [id],
                |row| row.get(0),
            )?;
            total += count;
        }
        Ok(total)
    }

    /// Deletes a profile and its keyring secret.
    ///
    /// Refuses while projects or workspaces still reference it — SQLite has no
    /// FK on these columns, so a blind delete would leave rows pointing at a
    /// profile that no longer exists and fail at connect time instead of here.
    pub fn delete(&self, id: &str) -> Result<(), Error> {
        if self.get(id)?.is_none() {
            return Err(Error::SshConnectionNotFound(id.to_string()));
        }

        let count = self.reference_count(id)?;
        if count > 0 {
            return Err(Error::SshConnectionInUse {
                id: id.to_string(),
                count,
            });
        }

        secrets::delete_secret(id)?;
        let conn = self.conn()?;
        conn.execute("DELETE FROM ssh_connections WHERE id = ?1", [id])?;
        Ok(())
    }
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDb;

    // No test writes a secret: the keyring is real OS state, and MEMORY.md's
    // rule is that tests never touch real app data. Secret paths are covered
    // by `secrets::entry_name` and manual verification.
    fn store() -> SshConnectionStore {
        SshConnectionStore::new(SqliteDb::init_in_memory().unwrap())
    }

    fn profile(name: &str) -> NewSshConnection {
        NewSshConnection {
            name: name.into(),
            host: "box.internal".into(),
            username: "deploy".into(),
            ..Default::default()
        }
    }

    #[test]
    fn create_get_list_roundtrip() {
        let s = store();
        let created = s
            .create(NewSshConnection {
                port: 2222,
                auth_type: SshAuthType::KeyFile,
                private_key_path: Some("~/.ssh/id_ed25519".into()),
                use_agent: false,
                metadata: Some(SshConnectionMeta {
                    alias: Some("prod".into()),
                    forward_agent: true,
                    ..Default::default()
                }),
                ..profile("prod")
            })
            .unwrap();

        assert_eq!(created.port, 2222);
        assert_eq!(created.auth_type, SshAuthType::KeyFile);
        assert!(!created.use_agent);
        let meta = created.metadata.clone().unwrap();
        assert_eq!(meta.alias.as_deref(), Some("prod"));
        assert!(meta.forward_agent);

        assert_eq!(s.get(&created.id).unwrap().as_ref(), Some(&created));

        s.create(profile("alpha")).unwrap();
        let listed = s.list().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "alpha", "sorted by name");
    }

    #[test]
    fn create_rejects_missing_host_or_user() {
        let s = store();
        assert!(s
            .create(NewSshConnection {
                host: "  ".into(),
                ..profile("bad")
            })
            .is_err());
        assert!(s
            .create(NewSshConnection {
                username: String::new(),
                ..profile("bad")
            })
            .is_err());
    }

    #[test]
    fn update_applies_partial_patch() {
        let s = store();
        let c = s.create(profile("prod")).unwrap();

        let updated = s
            .update(
                &c.id,
                SshConnectionPatch {
                    host: Some("new.internal".into()),
                    auth_type: Some(SshAuthType::Password),
                    private_key_path: Some(None),
                    ..Default::default()
                },
            )
            .unwrap();

        assert_eq!(updated.host, "new.internal");
        assert_eq!(updated.auth_type, SshAuthType::Password);
        assert_eq!(updated.private_key_path, None);
        assert_eq!(updated.username, c.username, "untouched field preserved");
        assert_eq!(updated.name, "prod");

        assert!(matches!(
            s.update("missing", SshConnectionPatch::default()),
            Err(Error::SshConnectionNotFound(_))
        ));
    }

    #[test]
    fn delete_blocked_while_referenced() {
        let s = store();
        let c = s.create(profile("prod")).unwrap();

        {
            let conn = s.conn().unwrap();
            conn.execute(
                "INSERT INTO projects (id, name, path, ssh_connection_id) VALUES ('p1', 'p', '/tmp/p', ?1)",
                [&c.id],
            )
            .unwrap();
        }

        assert_eq!(s.reference_count(&c.id).unwrap(), 1);
        match s.delete(&c.id) {
            Err(Error::SshConnectionInUse { count, .. }) => assert_eq!(count, 1),
            other => panic!("expected in-use error, got {other:?}"),
        }

        {
            let conn = s.conn().unwrap();
            conn.execute("DELETE FROM projects WHERE id = 'p1'", [])
                .unwrap();
        }
        s.delete(&c.id).unwrap();
        assert!(s.get(&c.id).unwrap().is_none());
        assert!(matches!(
            s.delete(&c.id),
            Err(Error::SshConnectionNotFound(_))
        ));
    }

    #[test]
    fn auth_type_roundtrips_and_degrades_to_agent() {
        for t in [
            SshAuthType::Agent,
            SshAuthType::Password,
            SshAuthType::KeyFile,
        ] {
            assert_eq!(SshAuthType::parse(t.as_str()), t);
        }
        assert_eq!(SshAuthType::parse("nonsense"), SshAuthType::Agent);
        assert!(SshAuthType::Password.needs_secret());
        assert!(!SshAuthType::Agent.needs_secret());
    }

    #[test]
    fn secret_ref_is_a_lookup_name_not_a_secret() {
        let s = store();
        let c = s.create(profile("prod")).unwrap();
        assert_eq!(c.secret_ref(), format!("ssh-connection:{}", c.id));
    }
}
