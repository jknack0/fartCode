//! Issue commands (E17-01, #55; ARCHITECTURE.md §13, ADR-0032) — thin CRUD
//! over [`IssueStore`] for the project board: lanes, blocked-by edges, and
//! the dispatch link the board dispatch engine (E17-03) drives.

use std::sync::Arc;

use serde::Deserialize;
use tauri::State;

use ade_core::issues::{Issue, IssuePatch, Lane, NewIssue};

use crate::app::App;

/// Request body for [`issue_create`] (frontend sends one object).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIssueRequest {
    pub project_id: String,
    pub title: String,
    pub body: Option<String>,
    pub acceptance: Option<Vec<String>>,
    pub lane: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub prd_path: Option<String>,
    pub prd_section: Option<String>,
}

/// Request body for [`issue_update`]. Missing = leave alone; explicit
/// `null` on a nullable field clears it (serde: absent → `None`,
/// `null` → `Some(None)`, value → `Some(Some(v))`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateIssueRequest {
    pub title: Option<String>,
    pub body: Option<Option<String>>,
    pub acceptance: Option<Vec<String>>,
    pub provider: Option<Option<String>>,
    pub model: Option<Option<String>>,
    pub prd_path: Option<Option<String>>,
    pub prd_section: Option<Option<String>>,
}

#[tauri::command]
pub fn issue_create(
    app: State<'_, Arc<App>>,
    request: CreateIssueRequest,
) -> Result<Issue, String> {
    let lane = request
        .lane
        .as_deref()
        .map(Lane::parse)
        .transpose()
        .map_err(String::from)?;
    app.issues
        .create(NewIssue {
            project_id: request.project_id,
            title: request.title,
            body: request.body,
            acceptance: request.acceptance.unwrap_or_default(),
            lane,
            provider: request.provider,
            model: request.model,
            prd_path: request.prd_path,
            prd_section: request.prd_section,
        })
        .map_err(String::from)
}

/// Issues for a project in board render order (lane rank, position), with
/// derived blocked status and blocker hover lists attached.
#[tauri::command]
pub fn issue_list(app: State<'_, Arc<App>>, project_id: String) -> Result<Vec<Issue>, String> {
    app.issues
        .list_for_project(&project_id)
        .map_err(String::from)
}

#[tauri::command]
pub fn issue_update(
    app: State<'_, Arc<App>>,
    issue_id: String,
    patch: UpdateIssueRequest,
) -> Result<Issue, String> {
    app.issues
        .update(
            &issue_id,
            IssuePatch {
                title: patch.title,
                body: patch.body,
                acceptance: patch.acceptance,
                provider: patch.provider,
                model: patch.model,
                prd_path: patch.prd_path,
                prd_section: patch.prd_section,
            },
        )
        .map_err(String::from)
}

/// Lane move (board drag). `position: None` appends to the lane end.
/// Blocked-dispatch confirmation is a frontend concern (ADR-0032); any
/// transition is permitted here.
#[tauri::command]
pub fn issue_move(
    app: State<'_, Arc<App>>,
    issue_id: String,
    lane: String,
    position: Option<i64>,
) -> Result<Issue, String> {
    let lane = Lane::parse(&lane).map_err(String::from)?;
    app.issues
        .move_to(&issue_id, lane, position)
        .map_err(String::from)
}

#[tauri::command]
pub fn issue_delete(app: State<'_, Arc<App>>, issue_id: String) -> Result<(), String> {
    app.issues.delete(&issue_id).map_err(String::from)
}

/// `issue_id` becomes blocked by `blocked_by_id`. Cycle/cross-project
/// rejections surface as errors for the card-detail UI.
#[tauri::command]
pub fn issue_link(
    app: State<'_, Arc<App>>,
    issue_id: String,
    blocked_by_id: String,
) -> Result<Issue, String> {
    app.issues
        .add_dependency(&issue_id, &blocked_by_id)
        .map_err(String::from)
}

#[tauri::command]
pub fn issue_unlink(
    app: State<'_, Arc<App>>,
    issue_id: String,
    blocked_by_id: String,
) -> Result<Issue, String> {
    app.issues
        .remove_dependency(&issue_id, &blocked_by_id)
        .map_err(String::from)
}
