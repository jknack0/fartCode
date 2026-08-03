//! View-state commands (E1-08): per-view UI state persisted in the KV store.

use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use crate::app::App;

#[tauri::command]
pub fn get_view_state(app: State<'_, Arc<App>>, key: String) -> Result<Option<Value>, String> {
    ade_core::view_state::get(&app.db, &key).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_view_state(app: State<'_, Arc<App>>, key: String, value: Value) -> Result<(), String> {
    ade_core::view_state::save(&app.db, &key, &value).map_err(|e| e.to_string())
}
