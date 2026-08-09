//! Step-engine commands (E18-04, #63) — thin wrappers over
//! [`crate::step_engine`]: the generalized column entry and the queue
//! confirm. The legacy `issue_move`/`issue_dispatch` commands stay for the
//! current UI; the UI wave migrates drags here.

use std::sync::Arc;

use tauri::State;

use crate::app::App;
use crate::step_engine::EnterOutcome;

/// The engine's move: enter a column, then run/queue/nothing per the
/// column's kind and `on_enter`. `position: None` appends.
#[tauri::command]
pub fn issue_enter_column(
    app: State<'_, Arc<App>>,
    issue_id: String,
    column_id: String,
    position: Option<i64>,
) -> Result<EnterOutcome, String> {
    // Command path: the returned launch is DELIVERED — the frontend opens
    // the session from this outcome.
    crate::step_engine::enter_column_from_command(&app, &issue_id, &column_id, position)
}

/// Fires the parked (queue-mode) step for an issue. Single-shot; typed
/// error when nothing is parked (never parked, already confirmed, or
/// cleared by a drag).
#[tauri::command]
pub fn step_confirm(app: State<'_, Arc<App>>, issue_id: String) -> Result<EnterOutcome, String> {
    crate::step_engine::confirm_step(&app, &issue_id)
}
