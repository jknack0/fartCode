//! Search + resource-monitor commands (E1-09).

use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use crate::app::App;

/// Item types the palette does not surface YET.
///
/// E19-03 (#72) writes one `feature` row per dossier section, but the row
/// STYLE — `<Column> — <feature title>` with right-meta `feature · #id`,
/// Enter opening the card detail — lands with #75. Until then the palette's
/// generic renderer would show them with a bare `feature` hint and an Enter
/// that does nothing, while consuming result slots from hits that do work.
/// The rows keep being written and stay idempotent; they just do not
/// surface.
///
/// **#75 deletes this constant** and the `query_excluding` call below.
const PALETTE_HIDDEN_TYPES: &[&str] = &[fartcode_core::dossier_index::ITEM_TYPE];

#[tauri::command]
pub fn search(
    app: State<'_, Arc<App>>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<fartcode_core::search::SearchResult>, String> {
    fartcode_core::search::query_excluding(
        &app.db,
        &query,
        limit.unwrap_or(10),
        PALETTE_HIDDEN_TYPES,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::PALETTE_HIDDEN_TYPES;
    use fartcode_core::db::{Db, SqliteDb};
    use fartcode_core::search;
    use std::sync::Arc;

    /// The command's own constant, exercised against the query it passes it
    /// to — so #75 removing the filter is a visible, deliberate deletion
    /// rather than a silent behaviour change.
    #[test]
    fn the_palette_query_holds_feature_rows_back() {
        let db: Arc<dyn Db> = SqliteDb::init_in_memory().unwrap();
        search::upsert(
            &db,
            "task",
            "t1",
            Some("p1"),
            None,
            "navbar work",
            &["navbar"],
        )
        .unwrap();
        search::upsert(
            &db,
            fartcode_core::dossier_index::ITEM_TYPE,
            "iss_1#Plan — x",
            Some("p1"),
            None,
            "Plan — x",
            &["navbar"],
        )
        .unwrap();

        let hits = search::query_excluding(&db, "navbar", 10, PALETTE_HIDDEN_TYPES).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].item_type, "task");
        // The row IS written — it just does not surface until #75.
        assert_eq!(search::query(&db, "navbar", 10).unwrap().len(), 2);
    }
}

#[tauri::command]
pub fn resource_sample(
    app: State<'_, Arc<App>>,
) -> Result<fartcode_core::resource_monitor::ResourceSample, String> {
    // The sample is enabled-gated by the caller (palette); sampling itself
    // is cheap and harmless.
    let _ = app;
    fartcode_core::resource_monitor::sample().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_resource_monitor_enabled(app: State<'_, Arc<App>>) -> Result<bool, String> {
    app.settings
        .get(&fartcode_core::settings::RESOURCE_MONITOR)
        .map(|g: fartcode_core::settings::ResourceMonitorGroup| g.enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_resource_monitor_enabled(app: State<'_, Arc<App>>, enabled: bool) -> Result<(), String> {
    app.settings
        .set(
            &fartcode_core::settings::RESOURCE_MONITOR,
            fartcode_core::settings::ResourceMonitorGroup { enabled },
        )
        .map_err(|e| e.to_string())
}

/// Re-exports the settings value for the frontend `Value`-typed settings API.
#[allow(dead_code)]
pub fn _settings_value(v: Value) -> Value {
    v
}
