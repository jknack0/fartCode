//! SSH connection profile commands (E12-03's task side, #94).
//!
//! The store has existed since E12-03 with no way to reach it: profiles could
//! only be created by a test. These commands are the UI's door to it.
//!
//! **Secrets never round-trip.** The DTO carries no password or passphrase,
//! and a save with a blank `secret` leaves the keyring entry alone — the same
//! "blank means keep" rule the provider-account form uses.

use std::sync::Arc;

use fartcode_core::ssh_connections::{
    NewSshConnection, SshAuthType, SshConnection, SshConnectionMeta, SshConnectionPatch,
};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::app::App;

/// A saved profile as the UI sees it — no secret material.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConnectionDto {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: String,
    pub private_key_path: Option<String>,
    pub use_agent: bool,
    /// `~/.ssh/config` alias this profile resolves through (E12-03). When set,
    /// host/port/user are resolved by `ssh -G` at connect time and the form's
    /// fields are informational.
    pub alias: Option<String>,
    pub proxy_jump: Option<String>,
    pub forward_agent: bool,
    pub projects_directory: Option<String>,
    /// True when a keyring secret exists for this profile — the form shows
    /// “stored” instead of an empty password box that looks unsaved.
    pub has_secret: bool,
}

impl From<&SshConnection> for SshConnectionDto {
    fn from(c: &SshConnection) -> Self {
        let meta = c.metadata.clone().unwrap_or_default();
        Self {
            id: c.id.clone(),
            name: c.name.clone(),
            host: c.host.clone(),
            port: c.port,
            username: c.username.clone(),
            auth_type: c.auth_type.as_str().to_string(),
            private_key_path: c.private_key_path.clone(),
            use_agent: c.use_agent,
            alias: meta.alias,
            proxy_jump: meta.proxy_jump,
            forward_agent: meta.forward_agent,
            projects_directory: meta.projects_directory,
            has_secret: fartcode_core::ssh_connections::secrets::has_secret(&c.id),
        }
    }
}

/// What the form sends. `id` absent = create.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConnectionInput {
    pub id: Option<String>,
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    pub auth_type: String,
    pub private_key_path: Option<String>,
    pub use_agent: Option<bool>,
    pub alias: Option<String>,
    pub proxy_jump: Option<String>,
    pub forward_agent: Option<bool>,
    pub projects_directory: Option<String>,
    /// Blank/absent means “keep whatever is in the keyring”.
    pub secret: Option<String>,
}

fn blank_to_none(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

impl SshConnectionInput {
    fn meta(&self) -> SshConnectionMeta {
        SshConnectionMeta {
            alias: blank_to_none(self.alias.clone()),
            proxy_jump: blank_to_none(self.proxy_jump.clone()),
            proxy_command: None,
            forward_agent: self.forward_agent.unwrap_or(false),
            projects_directory: blank_to_none(self.projects_directory.clone()),
        }
    }
}

/// Every saved profile, ordered by name.
#[tauri::command]
pub async fn ssh_connection_list(
    app: State<'_, Arc<App>>,
) -> Result<Vec<SshConnectionDto>, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.ssh_connections
            .list()
            .map(|list| list.iter().map(SshConnectionDto::from).collect())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Creates or updates a profile. A blank secret keeps the stored one.
#[tauri::command]
pub async fn ssh_connection_save(
    app: State<'_, Arc<App>>,
    input: SshConnectionInput,
) -> Result<SshConnectionDto, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let auth_type = SshAuthType::parse(&input.auth_type);
        let secret = blank_to_none(input.secret.clone());
        let meta = input.meta();
        let saved = match input.id.clone() {
            Some(id) => {
                // An edited profile may have changed host or auth — the pool's
                // cached session describes the OLD one, so drop it. No
                // disconnect intent: the next open dials with the new fields.
                app.remote_pty.forget(&id);
                app.ssh_connections.update(
                    &id,
                    SshConnectionPatch {
                        name: Some(input.name.trim().to_string()),
                        host: Some(input.host.trim().to_string()),
                        port: input.port,
                        username: Some(input.username.trim().to_string()),
                        auth_type: Some(auth_type),
                        private_key_path: Some(blank_to_none(input.private_key_path.clone())),
                        use_agent: input.use_agent,
                        metadata: Some(meta),
                        // `None` = leave the keyring alone (blank field).
                        secret: secret.map(Some),
                    },
                )
            }
            None => app.ssh_connections.create(NewSshConnection {
                name: input.name.trim().to_string(),
                host: input.host.trim().to_string(),
                port: input.port.unwrap_or(22),
                username: input.username.trim().to_string(),
                auth_type,
                private_key_path: blank_to_none(input.private_key_path.clone()),
                use_agent: input.use_agent.unwrap_or(true),
                metadata: Some(meta),
                secret,
            }),
        };
        saved
            .map(|c| SshConnectionDto::from(&c))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Deletes a profile. Refuses while a project or workspace still points at
/// it — a dangling `ssh_connection_id` is a project that can never open.
#[tauri::command]
pub async fn ssh_connection_delete(
    app: State<'_, Arc<App>>,
    connection_id: String,
) -> Result<(), String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let used = app
            .ssh_connections
            .reference_count(&connection_id)
            .map_err(|e| e.to_string())?;
        if used > 0 {
            return Err(format!(
                "{used} project(s) or workspace(s) still use this connection"
            ));
        }
        app.remote_pty.disconnect(&connection_id);
        app.ssh_connections
            .delete(&connection_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
