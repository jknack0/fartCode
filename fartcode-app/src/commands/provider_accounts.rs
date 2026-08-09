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
//!
//! **UI thread (#80):** every command here blocks on something the main
//! thread must never wait for — keyring round trips (`add`/`list`/`remove`;
//! `list` loads one secret PER ROW to compute the mask), a PATH scan plus a
//! child process polled for up to 5s (`provider_auth_status`), and a PTY
//! fork (`provider_auth_login`). A non-async `#[tauri::command]` runs its
//! body inline on the IPC thread — the macOS main thread — which stalls the
//! NSRunLoop and freezes the window. Each therefore hands its body to
//! `spawn_blocking` and awaits the join handle; merely marking them `async`
//! would only move the block onto a tokio worker. The wire contract
//! (argument names, serialized results, error strings, side-effect
//! ordering, the keyring rollback path) is unchanged.
//! `set_default_provider_account` and `list_providers` stay synchronous —
//! one indexed DB write and a static registry read, no keyring, no I/O.

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
pub async fn add_provider_account(
    app: State<'_, Arc<App>>,
    provider_id: String,
    account_id: String,
    secret: String,
    label: Option<String>,
    auth_method: Option<String>,
) -> Result<ProviderAccountDto, String> {
    // `State` cannot cross into the blocking closure; the managed value is
    // an `Arc<App>`, so clone the handle and move that.
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        add_provider_account_blocking(&app, provider_id, account_id, secret, label, auth_method)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Body of [`add_provider_account`], run on the blocking pool — the former
/// inline body verbatim (store secret → insert row → mask, with the
/// keyring rollback on insert failure).
fn add_provider_account_blocking(
    app: &App,
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
pub async fn list_provider_accounts(
    app: State<'_, Arc<App>>,
    provider_id: Option<String>,
) -> Result<Vec<ProviderAccountDto>, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || list_provider_accounts_blocking(&app, provider_id))
        .await
        .map_err(|e| e.to_string())?
}

/// Body of [`list_provider_accounts`], run on the blocking pool — one
/// keyring `load_secret` per row (for the mask), which is exactly why this
/// must not run on the main thread.
fn list_provider_accounts_blocking(
    app: &App,
    provider_id: Option<String>,
) -> Result<Vec<ProviderAccountDto>, String> {
    app.provider_accounts
        .list(provider_id.as_deref())
        .map(|accounts| accounts.iter().map(to_dto).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_provider_account(app: State<'_, Arc<App>>, id: String) -> Result<(), String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || remove_provider_account_blocking(&app, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// Body of [`remove_provider_account`], run on the blocking pool.
fn remove_provider_account_blocking(app: &App, id: &str) -> Result<(), String> {
    let account = app
        .provider_accounts
        .remove(id)
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
pub async fn provider_auth_status(provider_id: String) -> Result<AuthStatusDto, String> {
    tauri::async_runtime::spawn_blocking(move || provider_auth_status_blocking(provider_id))
        .await
        .map_err(|e| e.to_string())?
}

/// Body of [`provider_auth_status`], run on the blocking pool: a PATH scan
/// plus a child process polled to the 5s [`AUTH_STATUS_TIMEOUT`].
pub(crate) fn provider_auth_status_blocking(provider_id: String) -> Result<AuthStatusDto, String> {
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
pub async fn provider_auth_login(
    app: State<'_, Arc<App>>,
    terminals: State<'_, Arc<TerminalManager>>,
    provider_id: String,
    task_id: Option<String>,
    rows: u16,
    cols: u16,
) -> Result<String, String> {
    provider_auth_login_off_thread(
        app.inner().clone(),
        terminals.inner().clone(),
        provider_id,
        task_id,
        rows,
        cols,
    )
    .await
}

/// [`provider_auth_login`] with the Tauri `State` already unwrapped and
/// generic over the runtime, so tests can drive it with
/// `tauri::test::MockRuntime`.
pub(crate) async fn provider_auth_login_off_thread<R: tauri::Runtime>(
    app: Arc<App>,
    terminals: Arc<TerminalManager<R>>,
    provider_id: String,
    task_id: Option<String>,
    rows: u16,
    cols: u16,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        provider_auth_login_blocking(&app, &terminals, provider_id, task_id, rows, cols)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Body of [`provider_auth_login`], run on the blocking pool: PATH scan,
/// task-context DB lookup, then a PTY fork.
fn provider_auth_login_blocking<R: tauri::Runtime>(
    app: &App,
    terminals: &TerminalManager<R>,
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
    use std::sync::Mutex;
    use tauri::Manager;

    /// Every await here is bounded — an unbounded wait would wedge the
    /// suite instead of failing it.
    const TEST_TIMEOUT: Duration = Duration::from_secs(30);

    /// Drives a command future on a **single-threaded** runtime: the shape
    /// of the IPC thread, so a body that fails to leave the thread is
    /// observable (see the `*_yields_the_calling_thread` tests).
    fn block_on_bounded<F: std::future::Future>(fut: F) -> F::Output {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move {
            tokio::time::timeout(TEST_TIMEOUT, fut)
                .await
                .expect("command future timed out")
        })
    }

    fn app() -> Arc<App> {
        App::init(Some(":memory:")).expect("app init")
    }

    fn terminal_manager() -> Arc<TerminalManager<tauri::test::MockRuntime>> {
        Arc::new(TerminalManager::new(
            tauri::test::mock_app().handle().clone(),
        ))
    }

    /// Records the order a co-scheduled task and the awaited command
    /// complete in. On a single-threaded runtime the probe can only run
    /// while the command is pending — `["probe", "command"]` therefore
    /// proves the command left the calling thread (#80).
    fn assert_yields<F, T>(make: impl FnOnce() -> F)
    where
        F: std::future::Future<Output = T>,
    {
        let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let probe_order = order.clone();
        let command_order = order.clone();
        block_on_bounded(async move {
            tokio::spawn(async move { probe_order.lock().unwrap().push("probe") });
            let _ = make().await;
            command_order.lock().unwrap().push("command");
        });
        assert_eq!(
            order.lock().unwrap().as_slice(),
            ["probe", "command"],
            "the command blocked its caller instead of yielding"
        );
    }

    // ── add / list / remove ────────────────────────────────────────────
    //
    // These use the `claude-login` (cli-login) method on purpose: it
    // stores NO keyring secret, so the round trip exercises the real
    // command path without writing to the user's keychain. `to_dto`
    // still performs its keyring read for the mask and falls back to the
    // full mask, exactly as it does for login accounts in production.

    #[test]
    fn add_then_list_then_remove_round_trips_through_the_off_thread_path() {
        let app = app();
        let mock = tauri::test::mock_app();
        mock.manage(app.clone());

        let dto = block_on_bounded(add_provider_account(
            mock.state::<Arc<App>>(),
            "claude".into(),
            "me@example.com".into(),
            String::new(),
            Some("Subscription".into()),
            Some("claude-login".into()),
        ))
        .expect("add_provider_account");

        assert_eq!(dto.provider_id, "claude");
        assert_eq!(dto.account_id, "me@example.com");
        assert_eq!(dto.label.as_deref(), Some("Subscription"));
        assert_eq!(dto.auth_method.as_deref(), Some("claude-login"));
        // Login accounts store no secret: the mask is the full fallback.
        assert_eq!(dto.masked_secret, "••••");

        let listed = block_on_bounded(list_provider_accounts(
            mock.state::<Arc<App>>(),
            Some("claude".into()),
        ))
        .expect("list_provider_accounts");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, dto.id);
        assert_eq!(listed[0].masked_secret, dto.masked_secret);
        assert_eq!(listed[0].auth_method, dto.auth_method);

        // Filtering by another provider still returns nothing.
        assert!(block_on_bounded(list_provider_accounts(
            mock.state::<Arc<App>>(),
            Some("codex".into()),
        ))
        .expect("list_provider_accounts")
        .is_empty());

        // Removal must not fail on the missing keyring entry.
        block_on_bounded(remove_provider_account(mock.state::<Arc<App>>(), dto.id))
            .expect("remove_provider_account");
        assert!(
            block_on_bounded(list_provider_accounts(mock.state::<Arc<App>>(), None))
                .expect("list_provider_accounts")
                .is_empty()
        );
    }

    #[test]
    fn add_provider_account_preserves_the_error_strings() {
        let mock = tauri::test::mock_app();
        mock.manage(app());

        let unknown_provider = block_on_bounded(add_provider_account(
            mock.state::<Arc<App>>(),
            "not-a-provider".into(),
            "a".into(),
            "s".into(),
            None,
            None,
        ))
        .expect_err("unknown provider");
        assert_eq!(unknown_provider, "unknown provider: not-a-provider");

        let unknown_method = block_on_bounded(add_provider_account(
            mock.state::<Arc<App>>(),
            "claude".into(),
            "a".into(),
            "s".into(),
            None,
            Some("not-a-method".into()),
        ))
        .expect_err("unknown auth method");
        assert_eq!(
            unknown_method,
            "unknown auth method not-a-method for claude"
        );
    }

    #[test]
    fn remove_provider_account_preserves_the_error_string() {
        let app = app();
        let expected = remove_provider_account_blocking(&app, "no-such-account")
            .expect_err("unknown account id");

        let mock = tauri::test::mock_app();
        mock.manage(app);
        let got = block_on_bounded(remove_provider_account(
            mock.state::<Arc<App>>(),
            "no-such-account".into(),
        ))
        .expect_err("unknown account id");

        assert_eq!(got, expected);
    }

    #[test]
    fn list_provider_accounts_is_empty_on_a_fresh_db() {
        let mock = tauri::test::mock_app();
        mock.manage(app());
        let listed = block_on_bounded(list_provider_accounts(mock.state::<Arc<App>>(), None))
            .expect("list_provider_accounts");
        assert!(listed.is_empty());
    }

    #[test]
    fn add_provider_account_yields_the_calling_thread() {
        let mock = tauri::test::mock_app();
        mock.manage(app());
        assert_yields(|| {
            add_provider_account(
                mock.state::<Arc<App>>(),
                "claude".into(),
                "yield@example.com".into(),
                String::new(),
                None,
                Some("claude-login".into()),
            )
        });
    }

    #[test]
    fn list_provider_accounts_yields_the_calling_thread() {
        let mock = tauri::test::mock_app();
        mock.manage(app());
        assert_yields(|| list_provider_accounts(mock.state::<Arc<App>>(), None));
    }

    #[test]
    fn remove_provider_account_yields_the_calling_thread() {
        let mock = tauri::test::mock_app();
        mock.manage(app());
        assert_yields(|| remove_provider_account(mock.state::<Arc<App>>(), "missing".into()));
    }

    // ── auth status / login ────────────────────────────────────────────
    //
    // Only the pre-spawn error paths are exercised: probing a real agent
    // would shell out to the user's installed CLI (up to the 5s
    // AUTH_STATUS_TIMEOUT) and forking the login PTY would open an
    // interactive OAuth flow.

    #[test]
    fn provider_auth_status_preserves_the_error_strings() {
        let unknown = block_on_bounded(provider_auth_status("not-a-provider".into()))
            .expect_err("unknown provider");
        assert_eq!(unknown, "unknown provider: not-a-provider");
        assert_eq!(
            unknown,
            provider_auth_status_blocking("not-a-provider".into()).unwrap_err()
        );

        // `codex` has no auth methods → no CLI login method.
        let no_login =
            block_on_bounded(provider_auth_status("codex".into())).expect_err("no login method");
        assert_eq!(no_login, "codex has no CLI login method");
    }

    #[test]
    fn provider_auth_status_yields_the_calling_thread() {
        assert_yields(|| provider_auth_status("not-a-provider".into()));
    }

    #[test]
    fn provider_auth_login_preserves_the_error_strings() {
        let app = app();
        let unknown = block_on_bounded(provider_auth_login_off_thread(
            app.clone(),
            terminal_manager(),
            "not-a-provider".into(),
            None,
            24,
            80,
        ))
        .expect_err("unknown provider");
        assert_eq!(unknown, "unknown provider: not-a-provider");

        let no_login = block_on_bounded(provider_auth_login_off_thread(
            app,
            terminal_manager(),
            "codex".into(),
            None,
            24,
            80,
        ))
        .expect_err("no login method");
        assert_eq!(no_login, "codex has no CLI login method");
    }

    #[test]
    fn provider_auth_login_yields_the_calling_thread() {
        let app = app();
        let terminals = terminal_manager();
        assert_yields(move || {
            provider_auth_login_off_thread(app, terminals, "not-a-provider".into(), None, 24, 80)
        });
    }

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
