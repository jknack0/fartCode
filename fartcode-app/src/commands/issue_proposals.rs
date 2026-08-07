//! PM-chat proposal commands (E17-04, #58; ARCHITECTURE.md §13, ADR-0032):
//! the approval-card write path. `issue_parse_proposal` validates a raw
//! block payload (parse failures render as plain transcript text);
//! `issue_apply_proposal` atomically creates the issues + blocked-by edges
//! (compensating rollback in the domain layer).

use std::sync::Arc;

use tauri::State;

use fartcode_core::issue_proposal::{apply_proposal, parse_proposal, Proposal};
use fartcode_core::issues::Issue;

use crate::app::App;

/// Validates a raw ```fartCode-proposal block payload. Err = the frontend
/// renders the block as plain code text (never a card).
#[tauri::command]
pub fn issue_parse_proposal(text: String) -> Result<Proposal, String> {
    parse_proposal(&text).map_err(String::from)
}

/// Applies an approved proposal: creates the issues (backlog, PRD
/// reference attached) and wires blocked-by edges, all-or-nothing.
#[tauri::command]
pub fn issue_apply_proposal(
    app: State<'_, Arc<App>>,
    project_id: String,
    proposal: Proposal,
) -> Result<Vec<Issue>, String> {
    apply_proposal(&app.issues, &project_id, &proposal).map_err(String::from)
}
