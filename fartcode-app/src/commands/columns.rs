//! Board column commands (E18-02, ADR-0037) — thin CRUD over
//! [`ColumnStore`] for the configurable pipeline. Since the E18-07 flip
//! (#66) columns are authoritative for board placement; `issues.lane` is
//! display-only.

use std::sync::Arc;

use serde::Deserialize;
use tauri::State;

use fartcode_core::issues::columns::{
    BoardColumn, ColumnKind, ColumnPatch, NewColumn, OnEnter, OnSettle,
};

use super::serde_util::double_option;
use crate::app::App;

/// Request body for [`column_create`] (frontend sends one object). Enum
/// fields travel as their stored strings (`shelf|agent_step|human_gate`,
/// `run|queue`, `hold|advance`) and are parsed here, matching `issue_move`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateColumnRequest {
    pub project_id: String,
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub counts_as_done: bool,
    /// `true` moves the landing flag onto the new column.
    #[serde(default)]
    pub is_landing: bool,
    pub on_enter: Option<String>,
    pub on_settle: Option<String>,
    /// Must reference a same-project column; omitted/null = next column.
    pub advance_to: Option<String>,
    pub step_prompt: Option<String>,
    pub step_provider: Option<String>,
    pub step_model: Option<String>,
    pub step_effort: Option<String>,
    /// Omitted/null = unrestricted; a list (even empty) = allowlist.
    pub step_tools: Option<Vec<String>>,
}

/// Request body for [`column_update`]. Tri-state per clearable field:
/// absent = leave alone, explicit `null` = clear, value = set (the
/// `double_option` deserializer keeps `null` distinguishable from absent).
/// `isLanding: true` moves the landing flag; `false` on the landing column
/// is rejected.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateColumnRequest {
    pub name: Option<String>,
    pub kind: Option<String>,
    pub counts_as_done: Option<bool>,
    pub is_landing: Option<bool>,
    pub on_enter: Option<String>,
    pub on_settle: Option<String>,
    /// `null` clears back to "next column"; a value must reference a
    /// same-project column ≠ this one.
    #[serde(default, deserialize_with = "double_option")]
    pub advance_to: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub step_prompt: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub step_provider: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub step_model: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub step_effort: Option<Option<String>>,
    /// `null` clears the allowlist (back to unrestricted).
    #[serde(default, deserialize_with = "double_option")]
    pub step_tools: Option<Option<Vec<String>>>,
}

/// Columns for a project in board order (`position`).
#[tauri::command]
pub fn column_list(
    app: State<'_, Arc<App>>,
    project_id: String,
) -> Result<Vec<BoardColumn>, String> {
    app.columns
        .list_for_project(&project_id)
        .map_err(String::from)
}

#[tauri::command]
pub fn column_create(
    app: State<'_, Arc<App>>,
    request: CreateColumnRequest,
) -> Result<BoardColumn, String> {
    let kind = ColumnKind::parse(&request.kind).map_err(String::from)?;
    let on_enter = request
        .on_enter
        .as_deref()
        .map(OnEnter::parse)
        .transpose()
        .map_err(String::from)?;
    let on_settle = request
        .on_settle
        .as_deref()
        .map(OnSettle::parse)
        .transpose()
        .map_err(String::from)?;
    app.columns
        .create(NewColumn {
            project_id: request.project_id,
            name: request.name,
            kind,
            counts_as_done: request.counts_as_done,
            is_landing: request.is_landing,
            on_enter,
            on_settle,
            advance_to: request.advance_to,
            step_prompt: request.step_prompt,
            step_provider: request.step_provider,
            step_model: request.step_model,
            step_effort: request.step_effort,
            step_tools: request.step_tools,
        })
        .map_err(String::from)
}

#[tauri::command]
pub fn column_update(
    app: State<'_, Arc<App>>,
    column_id: String,
    patch: UpdateColumnRequest,
) -> Result<BoardColumn, String> {
    let kind = patch
        .kind
        .as_deref()
        .map(ColumnKind::parse)
        .transpose()
        .map_err(String::from)?;
    let on_enter = patch
        .on_enter
        .as_deref()
        .map(OnEnter::parse)
        .transpose()
        .map_err(String::from)?;
    let on_settle = patch
        .on_settle
        .as_deref()
        .map(OnSettle::parse)
        .transpose()
        .map_err(String::from)?;
    // E18-04 fix round (finding 14): an edit that removes the column's
    // queue-ness invalidates its pending parks — capture the before
    // state so they can be dropped (with step:queue_cleared) after.
    let was_queue_step = app
        .columns
        .get(&column_id)
        .map_err(String::from)?
        .is_some_and(|c| {
            c.kind == fartcode_core::issues::columns::ColumnKind::AgentStep
                && c.on_enter == fartcode_core::issues::columns::OnEnter::Queue
        });
    let updated = app
        .columns
        .update(
            &column_id,
            ColumnPatch {
                name: patch.name,
                kind,
                counts_as_done: patch.counts_as_done,
                is_landing: patch.is_landing,
                on_enter,
                on_settle,
                advance_to: patch.advance_to,
                step_prompt: patch.step_prompt,
                step_provider: patch.step_provider,
                step_model: patch.step_model,
                step_effort: patch.step_effort,
                step_tools: patch.step_tools,
            },
        )
        .map_err(String::from)?;
    let still_queue_step = updated.kind == fartcode_core::issues::columns::ColumnKind::AgentStep
        && updated.on_enter == fartcode_core::issues::columns::OnEnter::Queue;
    if was_queue_step && !still_queue_step {
        let still_runnable = updated.kind == fartcode_core::issues::columns::ColumnKind::AgentStep;
        crate::step_engine::on_column_lost_queue(&app, &column_id, still_runnable);
    }
    Ok(updated)
}

/// Rejected while the column is occupied (occupancy is strictly by the
/// authoritative `column_id` — E18-07), for the landing column (move
/// `isLanding` first), and for a column that is another column's
/// `advance_to` target (repoint it first). Remaining positions compact
/// to 0..n-1.
#[tauri::command]
pub fn column_delete(app: State<'_, Arc<App>>, column_id: String) -> Result<(), String> {
    app.columns.delete(&column_id).map_err(String::from)
}

/// Full-board reorder: `column_ids` must list each of the project's column
/// ids exactly once (new board order). Returns the reordered columns.
#[tauri::command]
pub fn column_reorder(
    app: State<'_, Arc<App>>,
    project_id: String,
    column_ids: Vec<String>,
) -> Result<Vec<BoardColumn>, String> {
    app.columns
        .reorder(&project_id, &column_ids)
        .map_err(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reviewers' reproduced defect: without `double_option`, the wire
    /// `null` collapsed to "absent" and a clear silently no-opped. The
    /// request DTO must keep all three states distinguishable.
    #[test]
    fn update_request_keeps_absent_null_and_value_distinct() {
        // Absent → keep (None).
        let absent: UpdateColumnRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(absent.step_prompt, None);
        assert_eq!(absent.advance_to, None);
        assert_eq!(absent.step_tools, None);

        // Explicit null → clear (Some(None)) — the reproduced case.
        let cleared: UpdateColumnRequest = serde_json::from_str(
            r#"{"stepPrompt":null,"stepProvider":null,"stepModel":null,
                "stepEffort":null,"advanceTo":null,"stepTools":null}"#,
        )
        .unwrap();
        assert_eq!(cleared.step_prompt, Some(None));
        assert_eq!(cleared.step_provider, Some(None));
        assert_eq!(cleared.step_model, Some(None));
        assert_eq!(cleared.step_effort, Some(None));
        assert_eq!(cleared.advance_to, Some(None));
        assert_eq!(cleared.step_tools, Some(None));

        // Value → set (Some(Some(v))).
        let set: UpdateColumnRequest =
            serde_json::from_str(r#"{"stepPrompt":"p","advanceTo":"col_1","stepTools":["read"]}"#)
                .unwrap();
        assert_eq!(set.step_prompt, Some(Some("p".into())));
        assert_eq!(set.advance_to, Some(Some("col_1".into())));
        assert_eq!(set.step_tools, Some(Some(vec!["read".to_string()])));
        // Untouched fields in the same payload stay "keep".
        assert_eq!(set.step_model, None);
    }
}
