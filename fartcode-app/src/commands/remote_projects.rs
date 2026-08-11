//! Remote project commands (E12-04): projects whose repository lives on an
//! SSH host — plus `clone_project`, the local clone flow that had a store
//! method (`create_clone`) and no command to reach it.
//!
//! **UI thread (#80):** every command here is `async`, so Tauri drives it on
//! the async runtime rather than inlining it into the IPC (main) thread. The
//! one blocking body — local `git clone` — goes to `spawn_blocking`, the same
//! shape `create_project` uses.
//!
//! **Connection lifecycle (E12-06):** commands here borrow the POOLED client
//! from `RemotePtyRegistry` — states, backoff and rehydrate live there, and
//! nothing in this file opens (or owns) a session of its own.

use std::sync::Arc;

use fartcode_core::projects::remote::{RemoteEntry, RemoteHost};
use fartcode_core::projects::{ProjectDto, ProjectStore};
use fartcode_core::ssh_connections::{ConnectionState, SshConnection};
use fartcode_ssh::host::{remote_projects_dir, SshRemoteHost};
use tauri::State;

use crate::app::App;

/// Resolves a stored profile and hands back the POOLED connection for it
/// (E12-06 AC2/AC3). Same session the terminals, agents and tmux views use:
/// a browse costs no handshake, and one reconnect ladder covers them all.
async fn connect(app: &App, connection_id: &str) -> Result<(SshConnection, SshRemoteHost), String> {
    let connection = app
        .ssh_connections
        .get(connection_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("ssh connection not found: {connection_id}"))?;
    let client = app
        .remote_pty
        .client_for(connection_id)
        .await
        .map_err(|e| e.to_string())?;
    let host = SshRemoteHost::new(client)
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

/// Creates a fresh repository on the remote host, then adds it as a project
/// (the "New" of Pick/Clone/New, E12-04).
#[tauri::command]
pub async fn new_remote_project(
    app: State<'_, Arc<App>>,
    connection_id: String,
    name: String,
) -> Result<ProjectDto, String> {
    let app = app.inner().clone();
    let (connection, host) = connect(&app, &connection_id).await?;
    let dir = remote_projects_dir(&host, &connection)
        .await
        .map_err(|e| e.to_string())?;
    app.remote_projects
        .create_remote_new(&host, &connection_id, &name, &dir)
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

/// Opens (or re-opens) a connection and holds it in the shared registry
/// (E12-05 AC13). Explicit, so a failure is reported to the user who asked
/// for it rather than swallowed by a background path.
#[tauri::command]
pub async fn ssh_connect(app: State<'_, Arc<App>>, connection_id: String) -> Result<bool, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.remote_pty
            .connect(&connection_id)
            .map(|()| true)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Drops a connection at the user's request (E12-05 AC13). The intent is
/// remembered: terminals and boot rehydration will NOT dial back on their
/// own — only another `ssh_connect` does.
#[tauri::command]
pub async fn ssh_disconnect(
    app: State<'_, Arc<App>>,
    connection_id: String,
) -> Result<bool, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.remote_pty.disconnect(&connection_id);
        false
    })
    .await
    .map_err(|e| e.to_string())
}

/// Whether this build can provision BYOI remote tasks (E12-10 gate). The UI
/// hides the workspace-provider settings when this is false.
#[tauri::command]
pub fn remote_tasks_enabled() -> bool {
    crate::byoi_tasks::ENABLED
}

/// Lifecycle view of one connection (E12-06 AC1/AC7): where it is, and
/// whether its host is refusing new channels (MaxSessions).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConnectionStatus {
    pub connection_id: String,
    pub state: ConnectionState,
    pub connected: bool,
    pub degraded: bool,
}

/// Current lifecycle state of a connection.
#[tauri::command]
pub async fn ssh_connection_state(
    app: State<'_, Arc<App>>,
    connection_id: String,
) -> Result<SshConnectionStatus, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || SshConnectionStatus {
        state: app.remote_pty.state(&connection_id),
        connected: app.remote_pty.is_connected(&connection_id),
        degraded: app.remote_pty.is_degraded(&connection_id),
        connection_id,
    })
    .await
    .map_err(|e| e.to_string())
}

/// Lifecycle state of every connection this process has touched — what the
/// connections panel renders on mount, before the event stream takes over.
#[tauri::command]
pub async fn ssh_connection_states(
    app: State<'_, Arc<App>>,
) -> Result<Vec<SshConnectionStatus>, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.remote_pty
            .states()
            .into_iter()
            .map(|(connection_id, state)| SshConnectionStatus {
                connected: app.remote_pty.is_connected(&connection_id),
                degraded: app.remote_pty.is_degraded(&connection_id),
                connection_id,
                state,
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}
