//! Board dispatch engine (E17-03, #57; ARCHITECTURE.md §13, ADR-0032).
//!
//! Drag-into-In-Progress composes the existing machinery: create the task
//! (worktree + issue-derived name + `linked_issue` local variant), link it
//! to the issue, move the card. The frontend then opens the agent terminal
//! and bracket-pastes the prompt packet (same flow as E4-10's
//! create-task-from-comment).
//!
//! The board never kills: re-dispatch of a card with a live linked task
//! REATTACHES (no second worktree); a deleted task clears the link
//! (ON DELETE SET NULL) and the next dispatch spawns fresh. Completion
//! auto-flips cards In Progress → In Review from two agent-exit hooks: the
//! terminal pump (agent PTY exit) and the ACP transcript (turn committed).

use std::sync::Arc;

use tauri::Manager as _;

use fartcode_core::conversations::ConversationStore;
use fartcode_core::issues::{build_dispatch_prompt, Issue, Lane};
use fartcode_core::settings::DEFAULT_AGENT;
use fartcode_core::tasks::operations::TaskConfigParams;
use fartcode_core::tasks::{LinkedIssue, TaskDto, TaskStore};

use crate::app::App;
use crate::commands::tasks::create_task_params;

/// What the frontend needs to launch the agent after a dispatch.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchOutcome {
    pub task: TaskDto,
    pub issue: Issue,
    /// The prompt packet — the frontend bracket-pastes it into the agent
    /// terminal (empty on reattach).
    pub prompt: String,
    /// Resolved provider id (issue override else the defaultAgent setting).
    pub provider: String,
    /// True when the card already had a live linked task — focus it, never
    /// spawn a second worktree.
    pub reattached: bool,
}

/// Drag-into-In-Progress: create + link + move, or reattach to the live
/// linked task. Testable without Tauri State.
pub fn issue_dispatch_core(app: &App, issue_id: &str) -> Result<DispatchOutcome, String> {
    let issue = app
        .issues
        .get(issue_id)
        .map_err(String::from)?
        .ok_or_else(|| format!("issue not found: {issue_id}"))?;

    // Reattach: the linked task still exists → no spawn, no lane change.
    if let Some(task_id) = &issue.linked_task_id {
        if let Some(task) = app.tasks.get(task_id).map_err(String::from)? {
            return Ok(DispatchOutcome {
                task: TaskDto::from(&task),
                issue,
                prompt: String::new(),
                provider: String::new(),
                reattached: true,
            });
        }
    }

    let provider = match &issue.provider {
        Some(p) => p.clone(),
        None => app.settings.get(&DEFAULT_AGENT).map_err(String::from)?,
    };
    // Finished-blocker summary keys off counts_as_done (E18-03, #77;
    // ADR-0037 item 6) — the blocker's COLUMN flag, never the 'done' lane
    // string, decides what the prompt calls finished.
    let finished_blockers: Vec<String> = issue
        .blockers
        .iter()
        .filter(|b| b.counts_as_done)
        .map(|b| b.title.clone())
        .collect();
    let prompt = build_dispatch_prompt(&issue, &finished_blockers);

    let params = create_task_params(
        app,
        &issue.project_id,
        &issue.title,
        TaskConfigParams {
            name: issue.title.clone(),
            initial_status: None,
            linked_issue: Some(LinkedIssue {
                provider: "local".into(),
                identifier: issue.id.clone(),
                title: issue.title.clone(),
                url: String::new(),
                display_identifier: None,
                description: None,
                branch_name: None,
                status: None,
            }),
            initial_conversation: None,
        },
    )?;
    let created = app
        .task_creation
        .create_with_provision(params)
        .map_err(|e| e.to_string())?;

    app.issues
        .set_linked_task(&issue.id, Some(&created.task.id))
        .map_err(String::from)?;
    let issue = app
        .issues
        .move_to(&issue.id, Lane::InProgress, None)
        .map_err(String::from)?;

    Ok(DispatchOutcome {
        task: TaskDto::from(&created.task),
        issue,
        prompt,
        provider,
        reattached: false,
    })
}

/// Auto-flip In Progress → In Review for issues linked to `task_id`.
/// Returns the flip count. Only In Progress cards move — a card the user
/// dragged elsewhere stays put.
pub fn flip_issues_for_task(app: &App, task_id: &str) -> usize {
    let issues = match app.issues.list_by_linked_task(task_id) {
        Ok(issues) => issues,
        Err(e) => {
            tracing::warn!(task_id, error = %e, "dispatch flip lookup failed");
            return 0;
        }
    };
    let mut flipped = 0;
    for issue in issues {
        if issue.lane != Lane::InProgress {
            continue;
        }
        match app.issues.move_to(&issue.id, Lane::InReview, None) {
            Ok(_) => flipped += 1,
            Err(e) => tracing::warn!(issue = %issue.id, error = %e, "dispatch flip failed"),
        }
    }
    flipped
}

/// ACP path: resolve the conversation to its task and flip. Project-scoped
/// conversations (task_id NULL, the PM chat) never flip anything.
pub fn flip_issues_for_conversation(app: &App, conversation_id: &str) {
    let conv = match app.conversations.get(conversation_id) {
        Ok(Some(conv)) => conv,
        _ => return,
    };
    if let Some(task_id) = &conv.task_id {
        flip_issues_for_task(app, task_id);
    }
}

/// Terminal-pump hook: an agent terminal exited. No-op when the App state
/// isn't managed (standalone-manager tests).
pub fn flip_for_exited_agent<R: tauri::Runtime>(app: &tauri::AppHandle<R>, task_id: &str) {
    if let Some(state) = app.try_state::<Arc<App>>() {
        flip_issues_for_task(&state, task_id);
    }
}
