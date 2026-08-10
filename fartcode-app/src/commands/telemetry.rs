//! Memory-value telemetry command (E19-04, #73; ADR-0038 item 7).
//!
//! One read-only command, for the value dashboard #76 builds at
//! settings → project → Memory. There is deliberately no *write* command:
//! observations are recorded by the settle hooks in `crate::telemetry`, and
//! a metric the frontend could inject is not a measurement.
//!
//! Nothing here can send anything anywhere. The computation lives in
//! `fartcode-telemetry`, which has no network dependency and no I/O at all
//! (its `tests/no_egress.rs` enforces both), and this command returns the
//! report to the local webview like any other DTO.

use std::sync::Arc;

use tauri::State;

use fartcode_telemetry::memory::{MemoryValue, DEFAULT_WINDOW_DAYS};

use crate::app::App;
use crate::commands::git::off_main_thread;

/// The four local memory-value signals for one project.
///
/// `async` + `spawn_blocking` (AGENTS.md § "Tauri commands and the main
/// thread"): the body takes the DB guard and walks up to
/// `MAX_DOSSIERS` small files off the project checkout. Bounded, but
/// filesystem work — it does not belong on the IPC thread, which on macOS
/// is the main thread.
///
/// Computed on demand rather than kept warm: the dashboard is a settings
/// panel somebody opens occasionally, and a cached figure is a figure that
/// can be stale in a way the caveat does not cover.
#[tauri::command]
pub async fn telemetry_memory_value(
    app: State<'_, Arc<App>>,
    project_id: String,
    window_days: Option<u32>,
) -> Result<MemoryValue, String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        Ok(crate::telemetry::memory_value(
            &app,
            &project_id,
            window_days.unwrap_or(DEFAULT_WINDOW_DAYS),
        ))
    })
    .await
}
