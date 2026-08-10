//! Search + resource-monitor commands (E1-09).

use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use crate::app::App;

/// Every indexed row, no type filter.
///
/// E19-03 (#72) held `feature` rows back behind a `PALETTE_HIDDEN_TYPES`
/// constant because the row STYLE did not exist yet — the generic renderer
/// would have shown a dossier section with a bare `feature` hint and an
/// Enter that did nothing. E19-06 (#75) is that style (handoff v3 §8h:
/// `<Column> — <feature title>`, right-meta `feature · #id`, Enter opens
/// the card detail), so the constant and its `query_excluding` call are
/// gone and dossier sections surface like anything else.
#[tauri::command]
pub fn search(
    app: State<'_, Arc<App>>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<fartcode_core::search::SearchResult>, String> {
    fartcode_core::search::query(&app.db, &query, limit.unwrap_or(10)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use fartcode_core::db::{Db, SqliteDb};
    use fartcode_core::search;
    use std::sync::Arc;

    /// The inverse of #72's `the_palette_query_holds_feature_rows_back`:
    /// the filter is gone, so a dossier section reaches the palette. Kept
    /// as a test rather than deleted with the constant — re-hiding feature
    /// rows should have to break something visible.
    #[test]
    fn the_palette_query_now_surfaces_feature_rows() {
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

        // The command's own query path, with no exclusions left to pass.
        let hits = search::query(&db, "navbar", 10).unwrap();
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(
            hits.iter()
                .any(|h| h.item_type == fartcode_core::dossier_index::ITEM_TYPE),
            "#75 turns feature hits on: {hits:?}"
        );
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
