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
    /// Auth method id from the provider's descriptor (`fartcode_providers`
    /// `auth_methods`), e.g. `claude-login` (OAuth) or
    /// `anthropic-api-key`. `None` = legacy row (api-key behavior).
    pub auth_method: Option<String>,
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
#[derive(Debug)]
pub struct AddAccountOptions {
    pub provider_id: String,
    /// Provider-side account identifier (username/email/org slug).
    pub account_id: String,
    /// Keyring entry name that already holds the secret. CLI-login
    /// (OAuth) accounts keep a ref but store NO keyring secret — the
    /// credential lives in the CLI's own store.
    pub credential_ref: String,
    pub label: Option<String>,
    /// Auth method id from the provider descriptor; `None` = legacy
    /// api-key behavior.
    pub auth_method: Option<String>,
}

pub struct ProviderAccountStore {
    db: Arc<dyn Db>,
}

const COLUMNS: &str =
    "id, provider_id, account_id, credential_ref, is_default, meta, created_at, updated_at, auth_method";

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
        auth_method: row.get(8)?,
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
            "INSERT INTO provider_accounts (id, provider_id, account_id, credential_ref, is_default, meta, created_at, updated_at, auth_method)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                id,
                opts.provider_id,
                opts.account_id,
                opts.credential_ref,
                if has_default { 0 } else { 1 },
                meta_cell,
                ts,
                ts,
                opts.auth_method,
            ],
        )?;
        Ok(ProviderAccount {
            id,
            provider_id: opts.provider_id,
            account_id: opts.account_id,
            credential_ref: opts.credential_ref,
            is_default: !has_default,
            meta,
            auth_method: opts.auth_method,
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

    /// The auth method the default account uses, resolved against the
    /// provider descriptor. `None` when the provider has no default
    /// account (launchers then fall back to their legacy env behavior).
    pub fn default_auth_method(
        &self,
        provider_id: &str,
    ) -> Result<Option<&'static fartcode_providers::AuthMethod>, Error> {
        let provider = fartcode_providers::get(provider_id)
            .ok_or_else(|| Error::Internal(format!("unknown provider: {provider_id}")))?;
        let Some(account) = self.default_for(provider_id)? else {
            return Ok(None);
        };
        Ok(match account.auth_method.as_deref() {
            Some(id) => provider.auth_method(id),
            // Legacy row (auth_method NULL) → first api-key method, else
            // the provider's first method (pre-method parity).
            None => provider.default_auth_method(),
        })
    }

    /// Server-side launch env for a provider's default account: every env
    /// var the account's auth method reads (api-key methods) set to the
    /// keyring secret. **No secret value ever leaves this function for
    /// the renderer** — launchers call it in-process.
    ///
    /// CLI-login (OAuth) accounts resolve to an EMPTY env: the CLI's own
    /// credential store authenticates, and injecting e.g.
    /// `ANTHROPIC_API_KEY` would flip it to API-key billing.
    pub fn resolve_env(&self, provider_id: &str) -> Result<Vec<(String, String)>, Error> {
        let provider = fartcode_providers::get(provider_id)
            .ok_or_else(|| Error::Internal(format!("unknown provider: {provider_id}")))?;
        let account = self.default_for(provider_id)?.ok_or_else(|| {
            Error::ProviderAccountNotFound(format!("no default account for {provider_id}"))
        })?;
        let method = match account.auth_method.as_deref() {
            Some(id) => provider.auth_method(id),
            None => provider.default_auth_method(),
        };
        if let Some(m) = method {
            if m.kind == fartcode_providers::AuthMethodKind::CliLogin {
                return Ok(Vec::new());
            }
        }
        let secret = secrets::load_secret(&account.credential_ref)?;
        let vars: Vec<&str> = match method {
            Some(m) if !m.env_vars.is_empty() => m.env_vars.iter().map(String::as_str).collect(),
            _ => provider.env_vars.iter().map(String::as_str).collect(),
        };
        Ok(vars
            .into_iter()
            .map(|var| (var.to_string(), secret.clone()))
            .collect())
    }

    /// Env vars that MUST be stripped from a launch when the provider's
    /// default account authenticates via CLI login (OAuth): passing e.g.
    /// `ANTHROPIC_API_KEY` through (keyring injection OR inherited parent
    /// env) would silently switch the CLI to API-key billing. Empty for
    /// api-key accounts and providers without a default account.
    pub fn resolve_removals(&self, provider_id: &str) -> Result<Vec<String>, Error> {
        let provider = fartcode_providers::get(provider_id)
            .ok_or_else(|| Error::Internal(format!("unknown provider: {provider_id}")))?;
        let Some(method) = self.default_auth_method(provider_id)? else {
            return Ok(Vec::new());
        };
        if method.kind != fartcode_providers::AuthMethodKind::CliLogin {
            return Ok(Vec::new());
        }
        let mut vars = provider.env_vars.clone();
        for v in &method.env_vars {
            if !vars.contains(v) {
                vars.push(v.clone());
            }
        }
        Ok(vars)
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
                auth_method: None,
            })
            .unwrap();
        assert!(a.is_default, "first account must become default");
        let b = s
            .add(AddAccountOptions {
                provider_id: "claude".into(),
                account_id: "second@example.com".into(),
                credential_ref: "ref-2".into(),
                label: None,
                auth_method: None,
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
                auth_method: None,
            })
            .unwrap();
        let b = s
            .add(AddAccountOptions {
                provider_id: "claude".into(),
                account_id: "two".into(),
                credential_ref: "ref-2".into(),
                label: None,
                auth_method: None,
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
                auth_method: None,
            })
            .unwrap();
        let b = s
            .add(AddAccountOptions {
                provider_id: "claude".into(),
                account_id: "two".into(),
                credential_ref: "ref-2".into(),
                label: None,
                auth_method: None,
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
                auth_method: None,
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

    /// ADR-0034: a claude-login (OAuth) default account resolves to an
    /// EMPTY launch env — the CLI's own credential store authenticates, and
    /// an injected ANTHROPIC_API_KEY would force API-key billing.
    #[test]
    fn login_account_resolves_empty_env_and_strips_api_key_vars() {
        let s = store();
        s.add(AddAccountOptions {
            provider_id: "claude".into(),
            account_id: "user@example.com".into(),
            credential_ref: "login-account".into(),
            label: None,
            auth_method: Some("claude-login".into()),
        })
        .unwrap();

        let method = s
            .default_auth_method("claude")
            .unwrap()
            .expect("default account exists");
        assert_eq!(method.id, "claude-login");
        assert_eq!(method.kind, fartcode_providers::AuthMethodKind::CliLogin);

        // No env injection at all (never reaches the keyring).
        assert!(s.resolve_env("claude").unwrap().is_empty());
        // Launchers must strip the API-key var from inherited env too.
        assert_eq!(
            s.resolve_removals("claude").unwrap(),
            vec!["ANTHROPIC_API_KEY"]
        );
    }

    /// An explicit api-key account keeps legacy injection behavior.
    #[test]
    fn api_key_account_resolves_env_and_no_removals() {
        let s = store();
        let account = s
            .add(AddAccountOptions {
                provider_id: "claude".into(),
                account_id: "billing@example.com".into(),
                credential_ref: "api-key-account".into(),
                label: None,
                auth_method: Some("anthropic-api-key".into()),
            })
            .unwrap();
        let method = s
            .default_auth_method("claude")
            .unwrap()
            .expect("default account exists");
        assert_eq!(method.id, "anthropic-api-key");
        assert!(s.resolve_removals("claude").unwrap().is_empty());
        match secrets::store_secret("api-key-account", "sk-test-456") {
            Ok(()) => {
                let env = s.resolve_env("claude").unwrap();
                assert!(env
                    .iter()
                    .any(|(k, v)| k == "ANTHROPIC_API_KEY" && v == "sk-test-456"));
                secrets::delete_secret("api-key-account").unwrap();
            }
            Err(Error::CredentialStore(_)) => {}
            Err(other) => panic!("expected CredentialStore, got {other:?}"),
        }
        let _ = account;
    }

    /// Legacy rows (auth_method NULL) resolve to the first api-key method
    /// — pre-method parity.
    #[test]
    fn legacy_account_behaves_as_api_key() {
        let s = store();
        s.add(AddAccountOptions {
            provider_id: "claude".into(),
            account_id: "legacy@example.com".into(),
            credential_ref: "legacy-account".into(),
            label: None,
            auth_method: None,
        })
        .unwrap();
        let method = s
            .default_auth_method("claude")
            .unwrap()
            .expect("default account exists");
        assert_eq!(method.id, "anthropic-api-key");
        assert!(s.resolve_removals("claude").unwrap().is_empty());
    }

    /// No default account → no method → launchers keep their legacy env
    /// behavior (process-env fallback on the ACP path).
    #[test]
    fn no_account_yields_no_auth_method() {
        let s = store();
        assert!(s.default_auth_method("claude").unwrap().is_none());
        assert!(s.resolve_removals("claude").unwrap().is_empty());
        assert!(matches!(
            s.resolve_env("claude").unwrap_err(),
            Error::ProviderAccountNotFound(_)
        ));
    }
}
