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
use fartcode_core::issues::{build_dispatch_prompt, Issue};
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
///
/// E18-07 (#66): the move target is COLUMN CONFIG, not a lane string —
/// the project's seeded In Progress column, resolved up front so a board
/// whose seeded step was deleted (legal since the flip) fails typed and
/// early, before a worktree is provisioned. The board's real path is
/// `issue_enter_column`; this legacy command keeps its wire contract.
/// The project's seeded In Progress column — the legacy dispatch target.
/// Typed error when it was deleted (legal since the E18-07 flip).
fn resolve_dispatch_column(
    app: &App,
    project_id: &str,
) -> Result<fartcode_core::issues::columns::BoardColumn, String> {
    app.columns
        .list_for_project(project_id)
        .map_err(String::from)?
        .into_iter()
        .find(|c| c.seed_lane.as_deref() == Some("in_progress"))
        .ok_or_else(|| {
            format!(
                "project {project_id} has no seeded In Progress column (it was deleted) — \
                 dispatch by moving the card onto an agent-step column instead"
            )
        })
}

/// Fail-closed precheck (#66 fix round): every typed refusal
/// [`issue_dispatch_core`] can produce BEFORE its side effects, checked
/// without any. `issue_dispatch` must not drop the parked step (a
/// destructive engine side effect) until dispatch is known to be able to
/// proceed — a refused dispatch would otherwise destroy the pending
/// confirm gate for an operation that never happened. Mirrors the core's
/// order: a live linked task short-circuits to reattach (which needs no
/// column), otherwise the seeded In Progress column must exist.
pub fn issue_dispatch_precheck(app: &App, issue_id: &str) -> Result<(), String> {
    let issue = app
        .issues
        .get(issue_id)
        .map_err(String::from)?
        .ok_or_else(|| format!("issue not found: {issue_id}"))?;
    if let Some(task_id) = &issue.linked_task_id {
        if app.tasks.get(task_id).map_err(String::from)?.is_some() {
            return Ok(()); // reattach path — no column needed
        }
    }
    resolve_dispatch_column(app, &issue.project_id).map(|_| ())
}

pub fn issue_dispatch_core(app: &App, issue_id: &str) -> Result<DispatchOutcome, String> {
    let issue = app
        .issues
        .get(issue_id)
        .map_err(String::from)?
        .ok_or_else(|| format!("issue not found: {issue_id}"))?;

    // Reattach: the linked task still exists → no spawn, no move.
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

    let dispatch_column = resolve_dispatch_column(app, &issue.project_id)?;

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
        .enter_column(&issue.id, &dispatch_column.id, None)
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
///
/// E19-01 (#70; ADR-0038 item 1): this is also where the feature dossier is
/// born — AFTER the worktree exists, written INSIDE it, so the file starts
/// life on the feature branch. Being here rather than in either caller is
/// what makes "the card's first `agent_step` entry" the single birth
/// moment: a card that already has a live linked task never reaches this
/// function, so a second step column cannot mint a second dossier.
/// Creation is best-effort by contract — a failure leaves `dossier_path`
/// NULL and the dispatch untouched.
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

    // Born with the worktree (ADR-0038 item 1). Infallible from here on:
    // the helper returns the UPDATED issue on success and `None` on every
    // refusal or failure, so there is nothing left that could turn a
    // dossier problem into a failed dispatch. (It could, before the fix
    // round: this used to re-read the row and propagate the read's error.)
    let linked = crate::dossiers::create_for_task(app, &linked, &created.task.id).unwrap_or(linked);
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
