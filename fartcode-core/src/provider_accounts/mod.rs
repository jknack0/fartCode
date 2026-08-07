//! Provider accounts (E3-07): one row per provider credential. The secret
//! itself lives in the OS keyring (see [`secrets`]); the DB row holds only
//! `credential_ref` — the keyring entry name. Launchers resolve launch env
//! server-side via [`ProviderAccountStore::resolve_env`]; no Tauri command
//! ever returns a secret value.
//!
//! Table: `provider_accounts` (0000 migration). `meta` is versioned JSON.

pub mod secrets;

use std::sync::Arc;

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::db::{parse_versioned, serialize_versioned, Db, Versioned};
use crate::Error;

/// A stored provider credential reference (secret stays in the keyring).
#[derive(Debug, Clone)]
pub struct ProviderAccount {
    pub id: String,
    pub provider_id: String,
    pub account_id: String,
    pub credential_ref: String,
    pub is_default: bool,
    pub meta: Option<AccountMeta>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Versioned `meta` payload: display-only fields.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AccountMeta {
    /// Human label ("work account").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl Versioned for AccountMeta {
    const VERSION: u32 = 1;
}

/// Options for [`ProviderAccountStore::add`].
pub struct AddAccountOptions {
    pub provider_id: String,
    /// Provider-side account identifier (username/email/org slug).
    pub account_id: String,
    /// Keyring entry name that already holds the secret.
    pub credential_ref: String,
    pub label: Option<String>,
}

pub struct ProviderAccountStore {
    db: Arc<dyn Db>,
}

const COLUMNS: &str =
    "id, provider_id, account_id, credential_ref, is_default, meta, created_at, updated_at";

fn account_from_row(row: &rusqlite::Row) -> rusqlite::Result<ProviderAccount> {
    let is_default: i64 = row.get(4)?;
    let meta_cell: Option<String> = row.get(5)?;
    Ok(ProviderAccount {
        id: row.get(0)?,
        provider_id: row.get(1)?,
        account_id: row.get(2)?,
        credential_ref: row.get(3)?,
        is_default: is_default != 0,
        meta: parse_versioned::<AccountMeta>("meta", meta_cell.as_deref()),
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl ProviderAccountStore {
    pub fn new(db: Arc<dyn Db>) -> Self {
        Self { db }
    }

    /// Inserts an account row. When no default exists for the provider yet,
    /// the new account becomes the default.
    pub fn add(&self, opts: AddAccountOptions) -> Result<ProviderAccount, Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
        let has_default: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM provider_accounts WHERE provider_id = ?1 AND is_default = 1)",
            [&opts.provider_id],
            |row| row.get(0),
        )?;
        let id = uuid::Uuid::new_v4().to_string();
        let ts = now_ms();
        let meta = opts.label.map(|label| AccountMeta { label: Some(label) });
        let meta_cell = match &meta {
            Some(m) => Some(serialize_versioned(m).map_err(|e| match e {
                Error::VersionedJson { reason, .. } => {
                    Error::Internal(format!("serialize account meta: {reason}"))
                }
                other => other,
            })?),
            None => None,
        };
        conn.execute(
            "INSERT INTO provider_accounts (id, provider_id, account_id, credential_ref, is_default, meta, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                id,
                opts.provider_id,
                opts.account_id,
                opts.credential_ref,
                if has_default { 0 } else { 1 },
                meta_cell,
                ts,
                ts,
            ],
        )?;
        Ok(ProviderAccount {
            id,
            provider_id: opts.provider_id,
            account_id: opts.account_id,
            credential_ref: opts.credential_ref,
            is_default: !has_default,
            meta,
            created_at: ts,
            updated_at: ts,
        })
    }

    /// Lists accounts, optionally filtered by provider (default first).
    pub fn list(&self, provider_id: Option<&str>) -> Result<Vec<ProviderAccount>, Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
        let rows = match provider_id {
            Some(pid) => {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {COLUMNS} FROM provider_accounts WHERE provider_id = ?1 ORDER BY is_default DESC, created_at ASC"
                ))?;
                let mapped = stmt.query_map([pid], account_from_row)?;
                mapped.collect::<Result<Vec<_>, _>>()?
            }
            None => {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {COLUMNS} FROM provider_accounts ORDER BY provider_id ASC, is_default DESC, created_at ASC"
                ))?;
                let mapped = stmt.query_map([], account_from_row)?;
                mapped.collect::<Result<Vec<_>, _>>()?
            }
        };
        Ok(rows)
    }

    /// The default account for a provider (used by `resolve_env`).
    pub fn default_for(&self, provider_id: &str) -> Result<Option<ProviderAccount>, Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
        let account = conn
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM provider_accounts WHERE provider_id = ?1 AND is_default = 1"
                ),
                [provider_id],
                account_from_row,
            )
            .optional()?;
        Ok(account)
    }

    /// Makes `id` the provider's default (clearing any sibling default).
    pub fn set_default(&self, id: &str) -> Result<(), Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
        let provider_id: String = conn
            .query_row(
                "SELECT provider_id FROM provider_accounts WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::ProviderAccountNotFound(id.to_string()))?;
        let ts = now_ms();
        conn.execute(
            "UPDATE provider_accounts SET is_default = 0 WHERE provider_id = ?1",
            [&provider_id],
        )?;
        conn.execute(
            "UPDATE provider_accounts SET is_default = 1, updated_at = ?2 WHERE id = ?1",
            rusqlite::params![id, ts],
        )?;
        Ok(())
    }

    /// Deletes the account row. Returns the removed row so callers can
    /// delete the keyring secret (`credential_ref`) too.
    pub fn remove(&self, id: &str) -> Result<ProviderAccount, Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
        let account: ProviderAccount = conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM provider_accounts WHERE id = ?1"),
                [id],
                account_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::ProviderAccountNotFound(id.to_string()))?;
        conn.execute("DELETE FROM provider_accounts WHERE id = ?1", [id])?;
        // Promote the oldest remaining account to default for this provider.
        conn.execute(
            "UPDATE provider_accounts SET is_default = 1
             WHERE id = (SELECT id FROM provider_accounts WHERE provider_id = ?1 ORDER BY created_at ASC LIMIT 1)",
            [&account.provider_id],
        )?;
        Ok(account)
    }

    /// Server-side launch env for a provider's default account: every env
    /// var the provider reads (`fartcode_providers` descriptor `env_vars`) set
    /// to the keyring secret. **No secret value ever leaves this function
    /// for the renderer** — launchers call it in-process.
    pub fn resolve_env(&self, provider_id: &str) -> Result<Vec<(String, String)>, Error> {
        let provider = fartcode_providers::get(provider_id)
            .ok_or_else(|| Error::Internal(format!("unknown provider: {provider_id}")))?;
        let account = self.default_for(provider_id)?.ok_or_else(|| {
            Error::ProviderAccountNotFound(format!("no default account for {provider_id}"))
        })?;
        let secret = secrets::load_secret(&account.credential_ref)?;
        Ok(provider
            .env_vars
            .iter()
            .map(|var| (var.clone(), secret.clone()))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDb;

    fn store() -> ProviderAccountStore {
        ProviderAccountStore::new(SqliteDb::init_in_memory().unwrap())
    }

    #[test]
    fn add_list_and_first_account_becomes_default() {
        let s = store();
        let a = s
            .add(AddAccountOptions {
                provider_id: "claude".into(),
                account_id: "user@example.com".into(),
                credential_ref: "ref-1".into(),
                label: Some("work".into()),
            })
            .unwrap();
        assert!(a.is_default, "first account must become default");
        let b = s
            .add(AddAccountOptions {
                provider_id: "claude".into(),
                account_id: "second@example.com".into(),
                credential_ref: "ref-2".into(),
                label: None,
            })
            .unwrap();
        assert!(!b.is_default);

        let listed = s.list(Some("claude")).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed[0].is_default);
        assert_eq!(
            listed[0].meta.as_ref().unwrap().label.as_deref(),
            Some("work")
        );
    }

    #[test]
    fn set_default_moves_the_flag_within_a_provider() {
        let s = store();
        let a = s
            .add(AddAccountOptions {
                provider_id: "claude".into(),
                account_id: "one".into(),
                credential_ref: "ref-1".into(),
                label: None,
            })
            .unwrap();
        let b = s
            .add(AddAccountOptions {
                provider_id: "claude".into(),
                account_id: "two".into(),
                credential_ref: "ref-2".into(),
                label: None,
            })
            .unwrap();
        s.set_default(&b.id).unwrap();
        let defaults: Vec<_> = s
            .list(Some("claude"))
            .unwrap()
            .into_iter()
            .filter(|x| x.is_default)
            .collect();
        assert_eq!(defaults.len(), 1);
        assert_eq!(defaults[0].id, b.id);
        assert!(s.set_default("missing").is_err());
        let _ = a;
    }

    #[test]
    fn remove_promotes_next_default_and_errors_on_missing() {
        let s = store();
        let a = s
            .add(AddAccountOptions {
                provider_id: "claude".into(),
                account_id: "one".into(),
                credential_ref: "ref-1".into(),
                label: None,
            })
            .unwrap();
        let b = s
            .add(AddAccountOptions {
                provider_id: "claude".into(),
                account_id: "two".into(),
                credential_ref: "ref-2".into(),
                label: None,
            })
            .unwrap();
        let removed = s.remove(&a.id).unwrap();
        assert_eq!(removed.credential_ref, "ref-1");
        assert!(s.default_for("claude").unwrap().unwrap().id == b.id);
        assert!(matches!(
            s.remove("missing").unwrap_err(),
            Error::ProviderAccountNotFound(_)
        ));
    }

    #[test]
    fn resolve_env_requires_a_default_account() {
        let s = store();
        let err = s.resolve_env("claude").unwrap_err();
        assert!(matches!(err, Error::ProviderAccountNotFound(_)));
        assert!(s.resolve_env("not-a-provider").is_err());
    }

    #[test]
    fn resolve_env_maps_secret_to_provider_env_vars() {
        // Deterministic against the seeded registry: claude reads
        // ANTHROPIC_API_KEY. The keyring may be unavailable in CI — the
        // typed error must surface, never a panic or plaintext fallback.
        let s = store();
        let account = s
            .add(AddAccountOptions {
                provider_id: "claude".into(),
                account_id: "user@example.com".into(),
                credential_ref: "resolve-test".into(),
                label: None,
            })
            .unwrap();
        match secrets::store_secret("resolve-test", "sk-test-123") {
            Ok(()) => {
                let env = s.resolve_env("claude").unwrap();
                assert!(env
                    .iter()
                    .any(|(k, v)| k == "ANTHROPIC_API_KEY" && v == "sk-test-123"));
                // Redaction must never reveal the value.
                let redacted = secrets::redact_env(&env);
                assert!(redacted.iter().all(|(_, v)| v != "sk-test-123"));
                secrets::delete_secret("resolve-test").unwrap();
            }
            Err(Error::CredentialStore(_)) => {} // no keyring in this env
            Err(other) => panic!("expected CredentialStore, got {other:?}"),
        }
        let _ = account;
    }
}
