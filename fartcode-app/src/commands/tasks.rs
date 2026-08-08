//! Task commands (E1-04 sidebar: list, pin toggle; E2-09 delete; E2-04
//! create+provision).

use fartcode_core::projects::ProjectStore;
use fartcode_core::settings::DEFAULT_AGENT;
use fartcode_core::tasks::deletion::DeleteTaskOptions;
use fartcode_core::tasks::naming::{
    generate_task_name, random_suffix, resolve_task_branch_name, BranchNameOptions,
};
use fartcode_core::tasks::operations::{
    CreateTaskParams, GitSetup, SourceBranchRef, TaskConfigParams,
};
use fartcode_core::tasks::{TaskDto, TaskStore, WorkspaceTarget};
use std::sync::Arc;

use tauri::State;

use crate::app::App;
use crate::commands::lifecycle::run_auto_lifecycle_scripts;
use crate::terminals::{TerminalManager, TerminalSpec};

/// Creates a task AND provisions its workspace (worktree + branch) in one
/// shot — the E2-04 create_with_provision flow. The branch name follows the
/// project group settings (prefix `fartCode` + random suffix by default); the
/// source ref is the project's base ref.
#[tauri::command]
pub fn create_task(
    app: State<'_, Arc<App>>,
    terminals: State<'_, Arc<TerminalManager>>,
    project_id: String,
    name: String,
) -> Result<TaskDto, String> {
    let params = create_task_params(
        &app,
        &project_id,
        &name,
        TaskConfigParams {
            name: name.clone(),
            initial_status: None,
            linked_issue: None,
            initial_conversation: None,
        },
    )?;
    let created = app
        .task_creation
        .create_with_provision(params)
        .map_err(|e| e.to_string())?;
    // E1-06: auto-run setup/run scripts on task creation when the project
    // configured them. Best-effort — the task stands either way.
    run_auto_lifecycle_scripts(&terminals, &app, &created.task.id);
    // PRD workflow: Add Task → spawn the chosen agent in its worktree.
    // Best-effort — without the agent binary the frontend's terminal
    // fallback covers the surface; with it, the agent tab IS the task.
    launch_default_agent(&terminals, &app, &created.task.id);
    Ok(TaskDto::from(&created.task))
}

/// Spawns the default agent CLI in the task's worktree (the left-nav
/// create path; dispatch and comment-tasks launch their own). Best-effort:
/// a missing binary or spawn failure logs and returns without failing the
/// task. One agent terminal per task holds (ADR-0033) — a live agent
/// reattaches instead of stacking.
pub fn launch_default_agent<R: tauri::Runtime>(
    terminals: &TerminalManager<R>,
    app: &App,
    task_id: &str,
) {
    let outcome = (|| -> Result<String, String> {
        let provider = app.settings.get(&DEFAULT_AGENT).map_err(String::from)?;
        let registry = fartcode_providers::get(&provider)
            .ok_or_else(|| format!("unknown agent: {provider}"))?;
        let binary = registry
            .binaries
            .iter()
            .find_map(|b| fartcode_core::pty::launcher::find_on_path(b))
            .ok_or_else(|| format!("agent not installed: {provider}"))?;
        let ctx = crate::commands::terminals::resolve_task_context(&app.db, task_id)?;
        let remove = crate::commands::terminals::agent_env_removals(app, &provider);
        terminals
            .open(TerminalSpec {
                task_id,
                project_id: &ctx.project_id,
                agent: Some(&provider),
                tmux: false,
                program: &binary.to_string_lossy(),
                args: &[],
                env: &[],
                remove: &remove,
                cwd: std::path::Path::new(&ctx.cwd),
                rows: 24,
                cols: 80,
                lifecycle: None,
            })
            .map_err(|e| e.to_string())
    })();
    match outcome {
        Ok(_) => {}
        // warn (not debug): the default level surfaces it — a silent
        // launch failure is exactly what left new tasks on the wrong tab.
        Err(e) => tracing::warn!(task_id, error = %e, "default agent launch failed"),
    }
}

/// Builds [`CreateTaskParams`] for the standard flow (E2-04 naming +
/// project base ref), shared by `create_task` and E4-10's
/// `create_task_from_comment` (which layers an initial conversation).
pub(crate) fn create_task_params(
    app: &App,
    project_id: &str,
    name: &str,
    task_config: TaskConfigParams,
) -> Result<CreateTaskParams, String> {
    let project = app
        .projects
        .get(project_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project not found: {project_id}"))?;

    let group = app
        .settings
        .get(&fartcode_core::settings::registry::PROJECT)
        .map_err(|e| e.to_string())?;

    let raw_branch = generate_task_name(Some(name), None, true);
    let branch_name = resolve_task_branch_name(&BranchNameOptions {
        raw_branch: &raw_branch,
        branch_prefix: Some(&group.branch_prefix),
        suffix: &random_suffix(),
        append_random_suffix: group.append_random_branch_suffix,
        linked_issue: None,
        disable_random_suffix: false,
    });

    // Base ref: "origin/main" (remote) or "main" (local).
    let base_ref = project.base_ref();
    let from_branch = match base_ref.split_once('/') {
        Some((remote, branch)) => SourceBranchRef::remote(branch, remote),
        None => SourceBranchRef::local(base_ref),
    };

    Ok(CreateTaskParams {
        id: None,
        project_id: project_id.to_string(),
        task_config,
        git: GitSetup::CreateBranch {
            branch_name,
            from_branch,
            push_branch: group.push_on_create,
        },
        workspace: WorkspaceTarget::NewWorktree,
        automation_run_id: None,
    })
}

/// Idempotent re-provision (E2-04 provision): materializes the task's
/// worktree when it's missing — heals tasks created before the command
/// provisioned at create time.
#[tauri::command]
pub fn provision_task(app: State<'_, Arc<App>>, task_id: String) -> Result<(), String> {
    app.task_creation
        .provision(&task_id)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_tasks(app: State<'_, Arc<App>>, project_id: String) -> Result<Vec<TaskDto>, String> {
    app.tasks
        .list_by_project(&project_id)
        .map(|ts| ts.iter().map(TaskDto::from).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn toggle_pin(app: State<'_, Arc<App>>, id: String) -> Result<TaskDto, String> {
    let task = app.tasks.get(&id).map_err(|e| e.to_string())?;
    let task = task.ok_or_else(|| format!("task not found: {id}"))?;
    app.tasks
        .set_pinned(&id, !task.is_pinned)
        .map(|t| TaskDto::from(&t))
        .map_err(|e| e.to_string())
}

/// E2-09: deletes the task with full teardown (running sessions reaped,
/// view state dropped, worktree removed when unused). Options follow the
/// reference `DeleteTaskOptions` defaults.
#[tauri::command]
pub fn delete_task(
    app: State<'_, Arc<App>>,
    terminals: State<'_, Arc<crate::terminals::TerminalManager>>,
    acp: State<'_, Arc<crate::acp_runtime::AcpRuntime>>,
    project_id: String,
    task_id: String,
    delete_worktree: Option<bool>,
    delete_branch: Option<bool>,
) -> Result<(), String> {
    // E2-11-5 acceptance 3: stop the task's ACP sessions BEFORE the task
    // row deletion cascades the conversation rows away (best-effort, like
    // the PTY reap).
    acp.stop_task(&task_id);
    let options = DeleteTaskOptions {
        delete_worktree: delete_worktree.unwrap_or(true),
        delete_branch: delete_branch.unwrap_or(false),
    };
    app.deletion
        .delete_task(&project_id, &task_id, &options)
        .map_err(|e| e.to_string())?;
    // E2-12: deleting a task closes its interactive terminals; ADR-0025:
    // and sweeps its tmux sessions (surviving orphans from crashes too).
    terminals.close_task(&project_id, &task_id);
    Ok(())
}
