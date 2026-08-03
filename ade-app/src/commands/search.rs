//! Search + resource-monitor commands (E1-09).

use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use crate::app::App;

#[tauri::command]
pub fn search(
    app: State<'_, Arc<App>>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<ade_core::search::SearchResult>, String> {
    ade_core::search::query(&app.db, &query, limit.unwrap_or(10)).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn resource_sample(
    app: State<'_, Arc<App>>,
) -> Result<ade_core::resource_monitor::ResourceSample, String> {
    // The sample is enabled-gated by the caller (palette); sampling itself
    // is cheap and harmless.
    let _ = app;
    ade_core::resource_monitor::sample().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_resource_monitor_enabled(app: State<'_, Arc<App>>) -> Result<bool, String> {
    app.settings
        .get(&ade_core::settings::RESOURCE_MONITOR)
        .map(|g: ade_core::settings::ResourceMonitorGroup| g.enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_resource_monitor_enabled(app: State<'_, Arc<App>>, enabled: bool) -> Result<(), String> {
    app.settings
        .set(
            &ade_core::settings::RESOURCE_MONITOR,
            ade_core::settings::ResourceMonitorGroup { enabled },
        )
        .map_err(|e| e.to_string())
}

/// Re-exports the settings value for the frontend `Value`-typed settings API.
#[allow(dead_code)]
pub fn _settings_value(v: Value) -> Value {
    v
}
