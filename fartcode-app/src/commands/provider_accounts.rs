//! Provider account commands (E3-07). **Secrets never cross the command
//! boundary:** `add` takes the secret once (stored in the keyring
//! server-side); every other command returns only masked/credential-ref
//! data.
//!
//! Auth methods (ADR-0034): an account can authenticate via a CLI-managed
//! login (OAuth subscription — `claude auth login`) instead of an API key.
//! Login accounts store NO keyring secret (the credential lives in the
//! CLI's own store); `provider_auth_status` probes the CLI for the live
//! login state and `provider_auth_login` opens the interactive flow in a
//! terminal.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fartcode_core::provider_accounts::{AddAccountOptions, ProviderAccount};
use serde::Serialize;
use tauri::State;

use crate::app::App;
use crate::commands::terminals::resolve_task_context;
use crate::terminals::{TerminalManager, TerminalSpec};

/// Renderer-facing DTO: no secret, no credential_ref (the ref is an
/// internal keyring handle).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountDto {
    pub id: String,
    pub provider_id: String,
    pub account_id: String,
    pub label: Option<String>,
    pub is_default: bool,
    /// Server-computed mask of the keyring secret (a mask, never the
    /// secret). Falls back to a full mask when the keyring is unavailable
    /// (CLI-login accounts never store a secret at all).
    pub masked_secret: String,
    /// Auth method id from the provider descriptor (`claude-login` =
    /// OAuth subscription, `anthropic-api-key` = API key, `None` = legacy).
    pub auth_method: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

fn to_dto(account: &ProviderAccount) -> ProviderAccountDto {
    ProviderAccountDto {
        id: account.id.clone(),
        provider_id: account.provider_id.clone(),
        account_id: account.account_id.clone(),
        label: account.meta.as_ref().and_then(|m| m.label.clone()),
        is_default: account.is_default,
        masked_secret: fartcode_core::provider_accounts::secrets::load_secret(
            &account.credential_ref,
        )
        .map(|secret| fartcode_core::provider_accounts::secrets::mask(&secret))
        .unwrap_or_else(|_| "••••".to_string()),
        auth_method: account.auth_method.clone(),
        created_at: account.created_at,
        updated_at: account.updated_at,
    }
}

/// Adds an account. `auth_method` selects the provider's login method
/// (e.g. `claude-login` for OAuth subscription); `None` keeps the legacy
/// api-key behavior (secret required). CLI-login accounts accept any
/// `secret` (it is ignored — nothing is stored in the keyring).
#[tauri::command]
pub fn add_provider_account(
    app: State<'_, Arc<App>>,
    provider_id: String,
    account_id: String,
    secret: String,
    label: Option<String>,
    auth_method: Option<String>,
) -> Result<ProviderAccountDto, String> {
    let provider = fartcode_providers::get(&provider_id)
        .ok_or_else(|| format!("unknown provider: {provider_id}"))?;
    // Resolve the method: explicit id, else the provider's default (first
    // api-key — legacy behavior). Providers without methods keep working
    // exactly as before (secret stored, auth_method NULL).
    let method = match &auth_method {
        Some(id) => Some(
            provider
                .auth_method(id)
                .ok_or_else(|| format!("unknown auth method {id} for {provider_id}"))?,
        ),
        None => provider.default_auth_method(),
    };
    let is_login = method
        .map(|m| m.kind == fartcode_providers::AuthMethodKind::CliLogin)
        .unwrap_or(false);

    let credential_ref = uuid::Uuid::new_v4().to_string();
    if !is_login {
        fartcode_core::provider_accounts::secrets::store_secret(&credential_ref, &secret)
            .map_err(|e| e.to_string())?;
    }
    match app.provider_accounts.add(AddAccountOptions {
        provider_id,
        account_id,
        credential_ref: credential_ref.clone(),
        label,
        auth_method: method.map(|m| m.id.to_string()),
    }) {
        Ok(account) => Ok(to_dto(&account)),
        Err(e) => {
            // Roll the keyring entry back when the row insert fails.
            if !is_login {
                if let Err(delete_err) =
                    fartcode_core::provider_accounts::secrets::delete_secret(&credential_ref)
                {
                    tracing::warn!(error = %delete_err, "rollback keyring delete failed");
                }
            }
            Err(e.to_string())
        }
    }
}

#[tauri::command]
pub fn list_provider_accounts(
    app: State<'_, Arc<App>>,
    provider_id: Option<String>,
) -> Result<Vec<ProviderAccountDto>, String> {
    app.provider_accounts
        .list(provider_id.as_deref())
        .map(|accounts| accounts.iter().map(to_dto).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_provider_account(app: State<'_, Arc<App>>, id: String) -> Result<(), String> {
    let account = app
        .provider_accounts
        .remove(&id)
        .map_err(|e| e.to_string())?;
    if let Err(e) =
        fartcode_core::provider_accounts::secrets::delete_secret(&account.credential_ref)
    {
        // Row is already gone; an orphaned keyring entry is a warning, not
        // a failure (acceptance: removal must not error on secret cleanup).
        // CLI-login accounts never stored a secret — this is expected.
        tracing::warn!(error = %e, credential_ref = %account.credential_ref, "keyring secret delete failed");
    }
    Ok(())
}

#[tauri::command]
pub fn set_default_provider_account(app: State<'_, Arc<App>>, id: String) -> Result<(), String> {
    app.provider_accounts
        .set_default(&id)
        .map_err(|e| e.to_string())
}

/// Registry listing for the accounts UI (no secrets in `ProviderDto`).
#[tauri::command]
pub fn list_providers() -> Vec<fartcode_providers::ProviderDto> {
    fartcode_providers::list_dtos()
}

/// Live auth state of the provider's CLI login (OAuth), from the CLI's own
/// status probe (`claude auth status` — JSON output). Used by the accounts
/// UI to show subscription login state; `add` with the cli-login method
/// records an account only after this reports `authenticated`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatusDto {
    pub provider_id: String,
    pub authenticated: bool,
    /// Account identifier from the CLI (email), when known.
    pub account: Option<String>,
    /// `oauth` | `apiKey` | `none` | `unknown`.
    pub method: String,
}

#[tauri::command]
pub fn provider_auth_status(provider_id: String) -> Result<AuthStatusDto, String> {
    let provider = fartcode_providers::get(&provider_id)
        .ok_or_else(|| format!("unknown provider: {provider_id}"))?;
    let login = provider
        .login_method()
        .ok_or_else(|| format!("{provider_id} has no CLI login method"))?;
    let binary = provider
        .binaries
        .iter()
        .find_map(|b| fartcode_core::pty::launcher::find_on_path(b))
        .ok_or_else(|| format!("agent not installed: {provider_id}"))?;
    let stdout = run_with_timeout(&binary, &login.status_args, AUTH_STATUS_TIMEOUT)?;
    let mut dto = parse_auth_status(&stdout);
    dto.provider_id = provider_id;
    Ok(dto)
}

/// Opens the provider's interactive login flow (`claude auth login`) in a
/// terminal rooted at the task's worktree (or the home dir when `task_id`
/// is empty — the settings-page flow). Returns the terminal id. The CLI
/// drives the OAuth handshake (browser + paste-code); the user completes
/// it there, then `provider_auth_status` reflects the new login.
#[tauri::command]
pub fn provider_auth_login(
    app: State<'_, Arc<App>>,
    terminals: State<'_, Arc<TerminalManager>>,
    provider_id: String,
    task_id: Option<String>,
    rows: u16,
    cols: u16,
) -> Result<String, String> {
    let provider = fartcode_providers::get(&provider_id)
        .ok_or_else(|| format!("unknown provider: {provider_id}"))?;
    let login = provider
        .login_method()
        .ok_or_else(|| format!("{provider_id} has no CLI login method"))?;
    let binary = provider
        .binaries
        .iter()
        .find_map(|b| fartcode_core::pty::launcher::find_on_path(b))
        .ok_or_else(|| format!("agent not installed: {provider_id}"))?;
    let (project_id, cwd) = match task_id {
        Some(task_id) => {
            let ctx = resolve_task_context(&app.db, &task_id)?;
            (ctx.project_id, ctx.cwd)
        }
        None => (
            String::new(),
            std::env::var("HOME").unwrap_or_else(|_| ".".to_string()),
        ),
    };
    terminals
        .open(TerminalSpec {
            task_id: "login",
            project_id: &project_id,
            agent: None,
            tmux: false,
            program: &binary.to_string_lossy(),
            args: &login.login_args,
            env: &[],
            remove: &[],
            cwd: Path::new(&cwd),
            rows: rows.max(24),
            cols: cols.max(80),
            lifecycle: None,
        })
        .map_err(|e| e.to_string())
}

const AUTH_STATUS_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs `program args` and captures stdout, killing the child on timeout.
/// The auth status probe output is tiny, so reading stdout after exit is
/// safe (no pipe-buffer stall).
fn run_with_timeout(program: &Path, args: &[String], timeout: Duration) -> Result<String, String> {
    use std::io::Read;
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("run {}: {e}", program.display()))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(format!(
                        "{} {} exited {}",
                        program.display(),
                        args.join(" "),
                        status
                    ));
                }
                let mut out = String::new();
                if let Some(mut stdout) = child.stdout.take() {
                    stdout
                        .read_to_string(&mut out)
                        .map_err(|e| format!("read {} output: {e}", program.display()))?;
                }
                return Ok(out);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "{} {} timed out after {}s",
                        program.display(),
                        args.join(" "),
                        timeout.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("wait {}: {e}", program.display())),
        }
    }
}

/// Parses `claude auth status` JSON into the DTO. Tolerates the shapes the
/// CLI has produced across versions (`loggedIn`/`authenticated`,
/// `oauthAccount`/`apiKeyAccount`/`account`, `authMethod` string).
fn parse_auth_status(stdout: &str) -> AuthStatusDto {
    let obj = serde_json::from_str::<serde_json::Value>(stdout)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let authenticated = ["loggedIn", "authenticated"]
        .iter()
        .filter_map(|k| obj.get(*k).and_then(|v| v.as_bool()))
        .next()
        .unwrap_or(false);
    let account = ["oauthAccount", "apiKeyAccount", "account"]
        .iter()
        .find_map(|k| obj.get(*k).and_then(|v| v.as_object()))
        .and_then(|acc| {
            ["emailAddress", "email", "accountEmail"]
                .iter()
                .filter_map(|k| acc.get(*k).and_then(|v| v.as_str()))
                .next()
                .map(str::to_string)
        })
        .or_else(|| {
            ["email", "accountEmail"]
                .iter()
                .filter_map(|k| obj.get(*k).and_then(|v| v.as_str()))
                .next()
                .map(str::to_string)
        });
    let method = obj
        .get("authMethod")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if obj.contains_key("oauthAccount") {
                "oauth".to_string()
            } else if obj.contains_key("apiKeyAccount") {
                "apiKey".to_string()
            } else {
                "unknown".to_string()
            }
        });
    AuthStatusDto {
        provider_id: String::new(),
        authenticated,
        account,
        method,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_logged_out_claude_status() {
        let dto = parse_auth_status(
            r#"{"loggedIn": false, "authMethod": "none", "apiProvider": "firstParty"}"#,
        );
        assert!(!dto.authenticated);
        assert_eq!(dto.method, "none");
        assert!(dto.account.is_none());
    }

    #[test]
    fn parses_oauth_account_shape() {
        let dto = parse_auth_status(
            r#"{"loggedIn": true, "authMethod": "oauth", "oauthAccount": {"emailAddress": "user@example.com", "orgId": "o1"}}"#,
        );
        assert!(dto.authenticated);
        assert_eq!(dto.method, "oauth");
        assert_eq!(dto.account.as_deref(), Some("user@example.com"));
    }

    #[test]
    fn parses_api_key_account_shape() {
        let dto = parse_auth_status(
            r#"{"loggedIn": true, "authMethod": "apiKey", "apiProvider": "firstParty", "apiKeyAccount": {"emailAddress": "billing@example.com"}}"#,
        );
        assert!(dto.authenticated);
        assert_eq!(dto.method, "apiKey");
        assert_eq!(dto.account.as_deref(), Some("billing@example.com"));
    }

    #[test]
    fn tolerates_garbage_output() {
        let dto = parse_auth_status("not json at all");
        assert!(!dto.authenticated);
        assert_eq!(dto.method, "unknown");
        assert!(dto.account.is_none());
    }
}
