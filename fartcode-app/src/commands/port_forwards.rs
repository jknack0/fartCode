//! Port-forward commands (E12-09). Thin wrappers over
//! [`crate::port_forwards::PortForwardService`]; the tunnels ride the pooled
//! SSH connections, so nothing here dials a session of its own.

use std::sync::Arc;

use tauri::State;

use crate::app::App;
use crate::port_forwards::PortForwardRecord;

/// Opens (or returns the existing) forward for `id`. The record carries the
/// ACTUAL local port — a busy preferred port falls back to an ephemeral one.
#[tauri::command]
pub async fn port_forward_open(
    app: State<'_, Arc<App>>,
    id: String,
    connection_id: String,
    remote_port: u16,
    preferred_local_port: Option<u16>,
) -> Result<PortForwardRecord, String> {
    let app = app.inner().clone();
    app.port_forwards
        .open(&id, &connection_id, remote_port, preferred_local_port)
        .await
        .map_err(|e| e.to_string())
}

/// Closes one forward; unknown ids are a no-op.
#[tauri::command]
pub async fn port_forward_stop(app: State<'_, Arc<App>>, id: String) -> Result<(), String> {
    app.port_forwards.stop(&id);
    Ok(())
}

/// Live forwards, sorted by id.
#[tauri::command]
pub async fn port_forward_list(app: State<'_, Arc<App>>) -> Result<Vec<PortForwardRecord>, String> {
    Ok(app.port_forwards.list())
}
