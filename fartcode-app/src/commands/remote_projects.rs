//! Remote project commands (E12-04): projects whose repository lives on an
//! SSH host — plus `clone_project`, the local clone flow that had a store
//! method (`create_clone`) and no command to reach it.
//!
//! **UI thread (#80):** every command here is `async`, so Tauri drives it on
//! the async runtime rather than inlining it into the IPC (main) thread. The
//! one blocking body — local `git clone` — goes to `spawn_blocking`, the same
//! shape `create_project` uses.
//!
//! **Connection lifecycle:** each command opens its own SSH connection and
//! drops it at the end. Pooling, states, and backoff are E12-06; doing it here
//! would build a second, throwaway lifecycle.

use std::sync::Arc;

use fartcode_core::projects::remote::{RemoteEntry, RemoteHost};
use fartcode_core::projects::{ProjectDto, ProjectStore};
use fartcode_core::ssh_connections::SshConnection;
use fartcode_ssh::host::{remote_projects_dir, SshRemoteHost};
use tauri::State;

use crate::app::App;

/// Resolves a stored profile and connects it.
async fn connect(app: &App, connection_id: &str) -> Result<(SshConnection, SshRemoteHost), String> {
    let connection = app
        .ssh_connections
        .get(connection_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("ssh connection not found: {connection_id}"))?;
    let host = SshRemoteHost::connect(&connection)
        .await
        .map_err(|e| e.to_string())?;
    Ok((connection, host))
}

/// Browses a directory on the remote host (the "Pick" half of Pick/Clone/New).
#[tauri::command]
pub async fn remote_browse(
    app: State<'_, Arc<App>>,
    connection_id: String,
    path: Option<String>,
    include_hidden: Option<bool>,
) -> Result<Vec<RemoteEntry>, String> {
    let app = app.inner().clone();
    let (_, host) = connect(&app, &connection_id).await?;
    // No path means "start where the user lands on login".
    let start = match path {
        Some(p) if !p.trim().is_empty() => p,
        _ => host
            .run(&["pwd"], None)
            .await
            .map_err(|e| e.to_string())?
            .stdout_trimmed()
            .to_string(),
    };
    host.list_dir(&start, include_hidden.unwrap_or(false))
        .await
        .map_err(|e| e.to_string())
}

/// Adds an existing remote repository as a project.
#[tauri::command]
pub async fn create_remote_project(
    app: State<'_, Arc<App>>,
    connection_id: String,
    remote_path: String,
) -> Result<ProjectDto, String> {
    let app = app.inner().clone();
    let (_, host) = connect(&app, &connection_id).await?;
    app.remote_projects
        .create_remote(&host, &connection_id, &remote_path)
        .await
        .map(|p| ProjectDto::from(&p))
        .map_err(|e| e.to_string())
}

/// Clones `url` on the remote host, then adds it as a project.
#[tauri::command]
pub async fn clone_remote_project(
    app: State<'_, Arc<App>>,
    connection_id: String,
    url: String,
) -> Result<ProjectDto, String> {
    let app = app.inner().clone();
    let (connection, host) = connect(&app, &connection_id).await?;
    let dir = remote_projects_dir(&host, &connection)
        .await
        .map_err(|e| e.to_string())?;
    app.remote_projects
        .create_remote_clone(&host, &connection_id, &url, &dir)
        .await
        .map(|p| ProjectDto::from(&p))
        .map_err(|e| e.to_string())
}

/// Clones `url` into the configured local projects directory (FLOWS.md F2).
///
/// `ProjectStore::create_clone` has existed since E1-03 with no command
/// pointing at it — e2e FIRST-16 recorded the whole flow as unreachable.
#[tauri::command]
pub async fn clone_project(app: State<'_, Arc<App>>, url: String) -> Result<ProjectDto, String> {
    let app = app.inner().clone();
    // git clone is a blocking subprocess: keep it off the IPC thread (#80).
    tauri::async_runtime::spawn_blocking(move || {
        app.projects
            .create_clone(&url)
            .map(|p| ProjectDto::from(&p))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
