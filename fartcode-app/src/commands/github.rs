//! PR section + GitHub token commands (E4-07/E4-09, #47/#49).
//!
//! The PR tab renders instantly from the sync cache and updates live:
//! `pr_section_get` reads the cache and kicks a background sync;
//! `pr_section_sync` awaits one sync pass (explicit refresh). The
//! scheduler keeps the cache warm in between. Token management:
//! keyring-backed, `gh auth token` import; nothing secret ever lands in
//! SQLite or logs.

use std::path::Path;
use std::sync::Arc;

use fartcode_core::github::{token, PrDto};
use fartcode_core::Error;
use serde::Serialize;
use tauri::State;

use crate::app::App;
use crate::commands::git::{project_push_remote, workspace_path};
use crate::commands::off_main_thread;

/// Payload for the Pull Requests tab: distinguishes "no token", "not a
/// GitHub repo / detached HEAD", "no PR for the branch", and "render".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrSectionDto {
    pub token_present: bool,
    /// `owner/repo` when the worktree has a GitHub remote.
    pub repository: Option<String>,
    pub branch: Option<String>,
    pub pr: Option<PrDto>,
}

/// Reads the PR section from the sync cache (instant, offline-safe) and
/// kicks a background sync so the cache converges on the live state.
#[tauri::command]
pub async fn pr_section_get(
    app: State<'_, Arc<App>>,
    workspace_id: String,
) -> Result<PrSectionDto, String> {
    let app = app.inner().clone();
    let section = section_from_cache(&app, &workspace_id)?;
    spawn_sync(app, workspace_id);
    Ok(section)
}

/// Awaits one sync pass for the workspace, then returns the cache. Used by
/// the tab's explicit refresh.
#[tauri::command]
pub async fn pr_section_sync(
    app: State<'_, Arc<App>>,
    workspace_id: String,
) -> Result<PrSectionDto, String> {
    let app = app.inner().clone();
    run_sync(&app, &workspace_id).await;
    section_from_cache(&app, &workspace_id)
}

/// Cache read + the empty-state labels (repo/branch from git, token state
/// from the keyring).
fn section_from_cache(app: &App, workspace_id: &str) -> Result<PrSectionDto, String> {
    let token_present = token::get_token().map_err(|e| e.to_string())?.is_some();
    let (repository, branch) = match resolve_target(app, workspace_id) {
        Ok(Some(t)) => (Some(t.repository), Some(t.branch)),
        _ => (None, None),
    };
    let prs = app
        .pr_sync
        .list_for_workspace(workspace_id)
        .map_err(|e| e.to_string())?;
    // The store orders open-first: the first row is "the" current PR.
    Ok(PrSectionDto {
        token_present,
        repository,
        branch,
        pr: prs.into_iter().next(),
    })
}

fn resolve_target(
    app: &App,
    workspace_id: &str,
) -> Result<Option<fartcode_git::pr_target::PrTarget>, String> {
    let worktree = workspace_path(app, workspace_id)?;
    let push_remote = project_push_remote(app, workspace_id)?;
    fartcode_git::pr_target::resolve_pr_target(&worktree, &push_remote).map_err(|e| e.to_string())
}

/// Fire-and-forget sync (tab-open hook).
fn spawn_sync(app: Arc<App>, workspace_id: String) {
    tauri::async_runtime::spawn(async move {
        run_sync(&app, &workspace_id).await;
    });
}

/// One sync pass with cursor bookkeeping. Sync failures are logged, never
/// surfaced — the cache stays as the render source and the scheduler
/// retries with backoff.
async fn run_sync(app: &App, workspace_id: &str) {
    let (worktree, push_remote) = match (
        workspace_path(app, workspace_id),
        project_push_remote(app, workspace_id),
    ) {
        (Ok(w), Ok(r)) => (w, r),
        _ => return, // workspace not on disk yet — nothing to sync
    };
    match sync_once(app, workspace_id, &worktree, &push_remote).await {
        Ok(()) => {
            let _ = app.pr_sync.record_success(workspace_id);
        }
        Err(e) => {
            let failures = app.pr_sync.record_failure(workspace_id).unwrap_or(1);
            tracing::debug!(workspace = %workspace_id, failures, error = %e, "pr sync failed");
        }
    }
}

async fn sync_once(
    app: &App,
    workspace_id: &str,
    worktree: &Path,
    push_remote: &str,
) -> Result<(), Error> {
    fartcode_git::pr_sync::sync_workspace(&app.pr_sync, workspace_id, worktree, push_remote).await
}

/// Token presence + display mask (never the token).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubTokenStatusDto {
    pub connected: bool,
    pub mask: Option<String>,
}

fn token_status() -> Result<GithubTokenStatusDto, String> {
    match token::get_token().map_err(|e| e.to_string())? {
        Some(t) => Ok(GithubTokenStatusDto {
            connected: true,
            mask: Some(token::mask(&t)),
        }),
        None => Ok(GithubTokenStatusDto {
            connected: false,
            mask: None,
        }),
    }
}

/// Current token state for the PR tab's empty-state CTA.
///
/// The four token commands are `async` + `spawn_blocking` for the same
/// reason as the git commands (#80): the keyring is a synchronous IPC call
/// to the OS, and a locked keychain turns it into an unbounded wait behind
/// a modal the app itself must keep repainting to show. `import` is worse
/// still — it scans `PATH` and spawns `gh`.
#[tauri::command]
pub async fn github_token_status() -> Result<GithubTokenStatusDto, String> {
    off_main_thread(token_status).await
}

/// Imports the active gh CLI token into the keyring.
#[tauri::command]
pub async fn github_token_import() -> Result<GithubTokenStatusDto, String> {
    off_main_thread(|| {
        token::import_from_gh().map_err(|e| e.to_string())?;
        token_status()
    })
    .await
}

/// Stores a pasted PAT. The value is never echoed back (status only).
#[tauri::command]
pub async fn github_token_set(token: String) -> Result<GithubTokenStatusDto, String> {
    off_main_thread(move || {
        token::set_token(&token).map_err(|e| e.to_string())?;
        token_status()
    })
    .await
}

/// Forgets the stored token.
#[tauri::command]
pub async fn github_token_clear() -> Result<(), String> {
    off_main_thread(|| token::clear_token().map_err(|e| e.to_string())).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The token commands must stay `async` (#80). This asserts exactly
    /// that and nothing else: an `async fn` runs none of its body until
    /// polled, so building the futures and dropping them never reaches the
    /// OS keychain — which a test must never read, write, or clear, since
    /// it is the developer's real one.
    ///
    /// A sync `fn` returns `Result<…>`, which is not a `Future`, so a
    /// regression fails this at compile time rather than at runtime.
    #[test]
    fn token_commands_stay_async_without_touching_the_keyring() {
        fn assert_future<F: std::future::Future>(_: F) {}
        assert_future(github_token_status());
        assert_future(github_token_import());
        assert_future(github_token_set("never-stored".into()));
        assert_future(github_token_clear());
    }
}
