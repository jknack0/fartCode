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

    let (task, _linked) = provision_issue_task(app, &issue)?;
    let issue = app
        .issues
        .move_to(&issue.id, Lane::InProgress, None)
        .map_err(String::from)?;

    Ok(DispatchOutcome {
        task,
        issue,
        prompt,
        provider,
        reattached: false,
    })
}

/// The provisioning tail of a dispatch (E17-03), shared verbatim with the
/// E18-04 step engine (first `agent_step` entry — reuse, not a fork):
/// create the task (worktree + issue-derived name + `linked_issue` local
/// variant, created by the user gesture), then link it to the issue.
/// Returns the created task and the re-read (linked) issue.
pub(crate) fn provision_issue_task(app: &App, issue: &Issue) -> Result<(TaskDto, Issue), String> {
    let params = create_task_params(
        app,
        &issue.project_id,
        &issue.title,
        None,
        None,
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

    let linked = app
        .issues
        .set_linked_task(&issue.id, Some(&created.task.id))
        .map_err(String::from)?;
    Ok((TaskDto::from(&created.task), linked))
}

/// Agent-settle trigger (E18-05 generalization of the E17-03 auto-flip):
/// both exit hooks below funnel into
/// [`crate::step_engine::settle_issues_for_task`], each carrying its
/// SESSION identity (fix round: settle is session-scoped — stale
/// triggers from finished/earlier sessions must no-op). The seeded In
/// Progress column (`on_settle: advance`, pinned to In Review)
/// reproduces the old In Progress → In Review flip exactly.
///
/// This identity-less wrapper survives for callers with no session
/// context (integration tests, restart-style paths): it settles via the
/// engine's registry-empty heuristic rules only.
pub fn flip_issues_for_task(app: &App, task_id: &str) -> usize {
    crate::step_engine::settle_issues_for_task(app, task_id, None)
}

/// ACP path: resolve the conversation to its task and settle with the
/// conversation as the session identity. Project-scoped conversations
/// (task_id NULL, the PM chat) never settle anything.
pub fn flip_issues_for_conversation(app: &App, conversation_id: &str) {
    let conv = match app.conversations.get(conversation_id) {
        Ok(Some(conv)) => conv,
        _ => return,
    };
    if let Some(task_id) = &conv.task_id {
        let session = format!("acp:{conversation_id}");
        crate::step_engine::settle_issues_for_task(app, task_id, Some(&session));
    }
}

/// Terminal-pump hook: an agent terminal exited. `terminal_id` is the
/// exiting entry's id — the PTY side of the session identity. No-op when
/// the App state isn't managed (standalone-manager tests).
pub fn flip_for_exited_agent<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    task_id: &str,
    terminal_id: &str,
) {
    if let Some(state) = app.try_state::<Arc<App>>() {
        let session = format!("pty:{terminal_id}");
        crate::step_engine::settle_issues_for_task(&state, task_id, Some(&session));
    }
}
