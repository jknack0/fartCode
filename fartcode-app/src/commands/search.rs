//! Search + resource-monitor commands (E1-09).

use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use crate::app::App;

/// A palette hit: the indexed row plus what the palette needs to ROUTE it.
///
/// `issue_id` is filled for `feature` rows (dossier sections) and `None`
/// for everything else. It is resolved here, from
/// `dossier_index::issue_id_of`, rather than by the webview re-splitting
/// `item_id` — one parse of the id scheme, in the module that defines it —
/// and rather than by a follow-up command, which left a window (and, when
/// the lookup failed, a permanent state) where ↵ on a feature row closed
/// the palette and opened nothing.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaletteHit {
    #[serde(flatten)]
    pub hit: fartcode_core::search::SearchResult,
    /// The card §8h's ↵ opens. Always present on a `feature` row.
    pub issue_id: Option<String>,
}

/// The palette's query policy: every indexed row, NO type filter.
///
/// E19-03 (#72) held `feature` rows back behind a `PALETTE_HIDDEN_TYPES`
/// constant because the row STYLE did not exist yet — the generic renderer
/// would have shown a dossier section with a bare `feature` hint and an
/// Enter that did nothing. E19-06 (#75) is that style (handoff v3 §8h:
/// `<Column> — <feature title>`, right-meta `feature · #id`, ↵ opens the
/// card detail), so the constant and its `query_excluding` call are gone.
///
/// A plain function, not inlined into the command, so the test exercises
/// the policy itself — re-hiding feature rows has to break something.
fn palette_hits(
    db: &Arc<dyn fartcode_core::db::Db>,
    query: &str,
    limit: usize,
) -> Result<Vec<PaletteHit>, fartcode_core::Error> {
    Ok(fartcode_core::search::query(db, query, limit)?
        .into_iter()
        .map(|hit| PaletteHit {
            issue_id: (hit.item_type == fartcode_core::dossier_index::ITEM_TYPE)
                .then(|| fartcode_core::dossier_index::issue_id_of(&hit.item_id))
                .flatten()
                .map(str::to_string),
            hit,
        })
        .collect())
}

#[tauri::command]
pub fn search(
    app: State<'_, Arc<App>>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<PaletteHit>, String> {
    palette_hits(&app.db, &query, limit.unwrap_or(10)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::palette_hits;
    use fartcode_core::db::{Db, SqliteDb};
    use fartcode_core::dossier_index;
    use fartcode_core::search;
    use std::sync::Arc;

    fn seeded() -> Arc<dyn Db> {
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
            dossier_index::ITEM_TYPE,
            &dossier_index::item_id("iss_1", "Plan — 2026-08-09", 0),
            Some("p1"),
            None,
            "Plan — 2026-08-09",
            &["navbar"],
        )
        .unwrap();
        db
    }

    /// The inverse of #72's `the_palette_query_holds_feature_rows_back`,
    /// pointed at the query POLICY the command runs — re-hiding feature
    /// rows has to break this.
    #[test]
    fn the_palette_query_now_surfaces_feature_rows() {
        let hits = palette_hits(&seeded(), "navbar", 10).unwrap();
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(
            hits.iter()
                .any(|h| h.hit.item_type == dossier_index::ITEM_TYPE),
            "#75 turns feature hits on: {hits:?}"
        );
    }

    /// §8h's ↵ opens the card detail, so every feature row must arrive
    /// already knowing its card — resolving it in a second round trip left
    /// a window where ↵ closed the palette and opened nothing.
    #[test]
    fn a_feature_hit_arrives_with_the_card_its_enter_opens() {
        let hits = palette_hits(&seeded(), "navbar", 10).unwrap();
        let feature = hits
            .iter()
            .find(|h| h.hit.item_type == dossier_index::ITEM_TYPE)
            .unwrap();
        assert_eq!(feature.issue_id.as_deref(), Some("iss_1"));
        // …and nothing else pretends to have one.
        let task = hits.iter().find(|h| h.hit.item_type == "task").unwrap();
        assert_eq!(task.issue_id, None);
    }

    #[test]
    fn the_hit_serializes_flat_with_the_search_row() {
        let hits = palette_hits(&seeded(), "navbar", 10).unwrap();
        let feature = hits
            .iter()
            .find(|h| h.hit.item_type == dossier_index::ITEM_TYPE)
            .unwrap();
        let json = serde_json::to_value(feature).unwrap();
        assert_eq!(json["itemType"], dossier_index::ITEM_TYPE);
        assert_eq!(json["title"], "Plan — 2026-08-09");
        assert_eq!(json["issueId"], "iss_1");
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
