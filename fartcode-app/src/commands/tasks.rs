//! Task commands (E1-04 sidebar: list, pin toggle; E2-09 delete; E2-04
//! create+provision).
//!
//! **Threading (#80):** a non-async `#[tauri::command]` compiles to
//! `ExecutionContext::Blocking` — its body is inlined into the invoke
//! handler and runs on the IPC thread, which on macOS is the main thread.
//! Blocking there stalls the NSRunLoop and the window stops repainting. The
//! commands here that touch git, the network, PTY spawns or tmux
//! (`create_task`, `list_project_branches`, `provision_task`,
//! `delete_task`) are therefore `async fn` whose whole body runs inside
//! [`off_main_thread`] (`spawn_blocking` + join). The bodies live in the
//! `*_blocking` fns below (same code, same order of side effects) so tests
//! can drive them directly, and so no DB guard is ever held across an
//! await: everything inside the closure is synchronous.

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
use crate::commands::off_main_thread;
use crate::terminals::{TerminalManager, TerminalSpec};

/// Creates a task AND provisions its workspace (worktree + branch) in one
/// shot — the E2-04 create_with_provision flow. The branch name follows the
/// project group settings (prefix `fartCode` + random suffix by default); the
/// source ref is the project's base ref. `workspace` selects the target:
/// `new-worktree` (default, isolated) or `project-root` (worktree-less,
/// current checkout — the dogfood mode). `branch` picks an existing branch
/// for the worktree instead of creating one.
#[tauri::command]
pub async fn create_task(
    app: State<'_, Arc<App>>,
    terminals: State<'_, Arc<TerminalManager>>,
    project_id: String,
    name: String,
    workspace: Option<String>,
    branch: Option<String>,
) -> Result<TaskDto, String> {
    let app = app.inner().clone();
    let terminals = terminals.inner().clone();
    // Off the IPC (macOS main) thread: the body fetches, spawns git, walks
    // the fs and forks PTYs — unbounded work behind an un-timed `git fetch`.
    off_main_thread(move || {
        create_task_blocking(
            &app,
            &terminals,
            &project_id,
            &name,
            workspace.as_deref(),
            branch.as_deref(),
        )
    })
    .await
}

/// `create_task`'s body, off the IPC thread. Generic over the Tauri runtime
/// so tests can drive it with `tauri::test::MockRuntime`.
pub fn create_task_blocking<R: tauri::Runtime>(
    app: &Arc<App>,
    terminals: &Arc<TerminalManager<R>>,
    project_id: &str,
    name: &str,
    workspace: Option<&str>,
    branch: Option<&str>,
) -> Result<TaskDto, String> {
    let params = create_task_params(
        app,
        project_id,
        name,
        workspace,
        branch,
        TaskConfigParams {
            name: name.to_string(),
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
    let setup_terminal = run_auto_lifecycle_scripts(terminals.as_ref(), app, &created.task.id);
    // PRD workflow: Add Task → spawn the chosen agent in its worktree.
    // Best-effort — without the agent binary the frontend's terminal
    // fallback covers the surface; with it, the agent tab IS the task.
    match setup_terminal {
        // 7b "A failed setup blocks agent start": when auto-run spawned a
        // setup script, the launch waits (off-thread — creation returns
        // immediately) for the setup terminal to exit 0.
        Some(setup_id) => {
            let terminals = terminals.clone();
            let app = app.clone();
            let task_id = created.task.id.clone();
            std::thread::spawn(move || {
                launch_default_agent_after_setup(terminals.as_ref(), &app, &task_id, &setup_id);
            });
        }
        // No auto-run setup → unchanged: launch right away.
        None => launch_default_agent(terminals.as_ref(), app, &created.task.id),
    }
    // Prompt-length names get an LLM title: a pasted paragraph is a fine
    // prompt but a terrible label. Best-effort + off-thread — the task
    // stands (and returns) with the full text; the rename lands later as
    // `task:renamed`.
    maybe_summarize_task_name(app, &created.task.id, name);
    Ok(TaskDto::from(&created.task))
}

/// A name longer than a label (or multi-line) is a pasted prompt. This
/// asks the claude CLI (print mode, haiku — the step engine's own
/// cheap-model pick) for a short title on a background thread and renames
/// the task when it answers. When claude is missing or fails (no auth, no
/// network), a tiny local model (ollama, gemma3:270m — ~240MB) takes the
/// same prompt. Best-effort at every step: neither CLI on PATH, non-zero
/// exits, or empty output all leave the full name in place — the task
/// never waits on this.
const SUMMARIZE_NAME_THRESHOLD: usize = 60;

/// Local fallback model: small enough to be a non-decision (~240MB), good
/// enough for "paragraph → 8-word title". Pull once: `ollama pull gemma3:270m`.
const OLLAMA_TITLE_MODEL: &str = "gemma3:270m";

pub fn maybe_summarize_task_name(app: &Arc<App>, task_id: &str, name: &str) {
    if name.len() <= SUMMARIZE_NAME_THRESHOLD && !name.contains('\n') {
        return;
    }
    let app = app.clone();
    let task_id = task_id.to_string();
    let name = name.to_string();
    std::thread::spawn(move || {
        let prompt = format!(
            "Summarize this coding task into a short title (at most 8 words). \
             Reply with ONLY the title - no quotes, no trailing period.\n\n{name}"
        );
        let title = summarize_via_claude(&task_id, &prompt)
            .or_else(|| summarize_via_ollama(&task_id, &prompt));
        let Some(title) = title else {
            tracing::debug!(
                task_id,
                "no summarizer produced a title — name stays verbatim"
            );
            return;
        };
        if let Err(e) = app.tasks.rename(&task_id, &title) {
            tracing::warn!(task_id, error = %e, "title rename failed");
        }
    });
}

/// First non-empty stdout line, shorn of the wrappers models add despite
/// instructions (quotes, markdown bold, trailing period), capped at 80 chars.
fn first_line_title(stdout: &[u8]) -> String {
    String::from_utf8_lossy(stdout)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .trim_matches(|c| c == '"' || c == '*')
        .trim_end_matches('.')
        .chars()
        .take(80)
        .collect()
}

fn summarize_via_claude(task_id: &str, prompt: &str) -> Option<String> {
    let binary = fartcode_core::pty::launcher::find_on_path("claude")?;
    let out = std::process::Command::new(&binary)
        .args(["-p", "--model", "haiku"])
        .arg(prompt)
        // Same rule as agent terminals (see terminals.rs): a stray key
        // flips the CLI to API billing — subscription auth must win.
        .env_remove("ANTHROPIC_API_KEY")
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let title = first_line_title(&o.stdout);
            (!title.is_empty()).then_some(title)
        }
        Ok(o) => {
            tracing::warn!(task_id, code = ?o.status.code(), "claude title summary failed");
            None
        }
        Err(e) => {
            tracing::warn!(task_id, error = %e, "claude title spawn failed");
            None
        }
    }
}

fn summarize_via_ollama(task_id: &str, prompt: &str) -> Option<String> {
    let binary = fartcode_core::pty::launcher::find_on_path("ollama")?;
    let out = std::process::Command::new(&binary)
        .args(["run", OLLAMA_TITLE_MODEL])
        .arg(prompt)
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let title = first_line_title(&o.stdout);
            (!title.is_empty()).then_some(title)
        }
        Ok(o) => {
            tracing::warn!(task_id, code = ?o.status.code(), "ollama title summary failed");
            None
        }
        Err(e) => {
            tracing::warn!(task_id, error = %e, "ollama title spawn failed");
            None
        }
    }
}

/// 7b "A failed setup blocks agent start": blocks until the auto-run setup
/// terminal exits, then launches the default agent ONLY on exit 0 — a
/// non-zero (or unreported) code, or a setup tab closed mid-run, skips the
/// launch with a warn trace. Synchronous so the regression test can drive
/// it directly; `create_task` runs it on a background thread.
pub fn launch_default_agent_after_setup<R: tauri::Runtime>(
    terminals: &TerminalManager<R>,
    app: &App,
    task_id: &str,
    setup_terminal_id: &str,
) {
    match terminals.wait_for_exit(setup_terminal_id) {
        Some(Some(0)) => launch_default_agent(terminals, app, task_id),
        Some(code) => tracing::warn!(
            task_id,
            exit_code = ?code,
            "setup script failed — default agent launch skipped"
        ),
        None => tracing::warn!(
            task_id,
            "setup terminal closed before exit — default agent launch skipped"
        ),
    }
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
        let (aa_args, aa_env) = crate::commands::terminals::agent_auto_approve(app, registry);
        terminals
            .open(TerminalSpec {
                task_id,
                project_id: &ctx.project_id,
                agent: Some(&provider),
                tmux: false,
                program: &binary.to_string_lossy(),
                args: &aa_args,
                env: &aa_env,
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
/// `workspace`/`branch` override the default (new worktree + fresh branch
/// off the base ref); the comment/dispatch callers pass `None`.
/// `pub` for the integration tests (same pattern as
/// `create_task_from_comment_core`).
pub fn create_task_params(
    app: &App,
    project_id: &str,
    name: &str,
    workspace: Option<&str>,
    branch: Option<&str>,
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

    // Project-root tasks run in the live checkout — never mint or switch
    // branches there (the checkout is the user's; GitSetup::None leaves it
    // untouched).
    let (git, workspace_target) = match workspace {
        Some("project-root") => (GitSetup::None, WorkspaceTarget::ProjectRoot),
        None | Some("new-worktree") => {
            let git = match branch {
                Some(b) if !b.trim().is_empty() => GitSetup::UseBranch {
                    branch_name: b.trim().to_string(),
                },
                _ => {
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
                    GitSetup::CreateBranch {
                        branch_name,
                        from_branch,
                        push_branch: group.push_on_create,
                    }
                }
            };
            (git, WorkspaceTarget::NewWorktree)
        }
        Some(other) => return Err(format!("invalid workspace target: {other}")),
    };

    Ok(CreateTaskParams {
        id: None,
        project_id: project_id.to_string(),
        task_config,
        git,
        workspace: workspace_target,
        automation_run_id: None,
    })
}

/// Branch list for the create-task dialog's start-source picker (reference
/// `git branch -a` list in the dialog).
#[tauri::command]
pub async fn list_project_branches(
    app: State<'_, Arc<App>>,
    project_id: String,
) -> Result<Vec<fartcode_core::git::BranchRef>, String> {
    let app = app.inner().clone();
    // Off the IPC thread: `git branch -a` is a subprocess.
    off_main_thread(move || list_project_branches_blocking(&app, &project_id)).await
}

/// `list_project_branches`'s body, off the IPC thread.
pub fn list_project_branches_blocking(
    app: &App,
    project_id: &str,
) -> Result<Vec<fartcode_core::git::BranchRef>, String> {
    app.task_creation
        .list_branches(project_id)
        .map_err(|e| e.to_string())
}

/// Idempotent re-provision (E2-04 provision): materializes the task's
/// worktree when it's missing — heals tasks created before the command
/// provisioned at create time.
#[tauri::command]
pub async fn provision_task(app: State<'_, Arc<App>>, task_id: String) -> Result<(), String> {
    let app = app.inner().clone();
    // Off the IPC thread: cold provision runs `git fetch` (un-timed) plus
    // worktree list/prune/add; even the idempotent reuse path shells out.
    off_main_thread(move || provision_task_blocking(&app, &task_id)).await
}

/// `provision_task`'s body, off the IPC thread.
pub fn provision_task_blocking(app: &App, task_id: &str) -> Result<(), String> {
    // E12-10: a BYOI task's workspace is a machine, not a worktree — the
    // project's provision script makes it, and core's provision is a no-op
    // for that row kind.
    crate::byoi_tasks::provision(app, task_id).map_err(|e| e.to_string())?;
    app.task_creation
        .provision(task_id)
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

/// 7a "a archive instead": sets `archived_at` — the card leaves the board
/// but the row, worktree, and branch all survive (delete is the only
/// destructive path). Emits `task:archived` so stores refetch.
#[tauri::command]
pub fn task_archive(app: State<'_, Arc<App>>, task_id: String) -> Result<TaskDto, String> {
    app.tasks
        .archive(&task_id)
        .map(|t| TaskDto::from(&t))
        .map_err(|e| e.to_string())
}

/// ⌘K restore: clears `archived_at`, the task returns to the board.
/// Emits `task:restored` so stores refetch.
#[tauri::command]
pub fn task_restore(app: State<'_, Arc<App>>, task_id: String) -> Result<TaskDto, String> {
    app.tasks
        .restore(&task_id)
        .map(|t| TaskDto::from(&t))
        .map_err(|e| e.to_string())
}

/// E2-09: deletes the task with full teardown (running sessions reaped,
/// view state dropped, worktree removed when unused). Options follow the
/// reference `DeleteTaskOptions` defaults.
#[tauri::command]
pub async fn delete_task(
    app: State<'_, Arc<App>>,
    terminals: State<'_, Arc<crate::terminals::TerminalManager>>,
    acp: State<'_, Arc<crate::acp_runtime::AcpRuntime>>,
    project_id: String,
    task_id: String,
    delete_worktree: Option<bool>,
    delete_branch: Option<bool>,
) -> Result<(), String> {
    let app = app.inner().clone();
    let terminals = terminals.inner().clone();
    let acp = acp.inner().clone();
    // Off the IPC thread: teardown spin-waits up to 5s PER session leaf,
    // kills tmux sessions, removes the worktree and prunes — tens of
    // seconds in the worst case, and the exact freeze users reported.
    off_main_thread(move || {
        delete_task_blocking(
            &app,
            &terminals,
            &acp,
            &project_id,
            &task_id,
            delete_worktree,
            delete_branch,
        )
    })
    .await
}

/// `delete_task`'s body, off the IPC thread. The order of side effects is
/// load-bearing: ACP stop → row deletion + worktree removal → terminal
/// teardown.
pub fn delete_task_blocking<R: tauri::Runtime>(
    app: &App,
    terminals: &TerminalManager<R>,
    acp: &crate::acp_runtime::AcpRuntime,
    project_id: &str,
    task_id: &str,
    delete_worktree: Option<bool>,
    delete_branch: Option<bool>,
) -> Result<(), String> {
    // E2-11-5 acceptance 3: stop the task's ACP sessions BEFORE the task
    // row deletion cascades the conversation rows away (best-effort, like
    // the PTY reap).
    acp.stop_task(task_id);
    // E12-10: destroy the BYOI machine BEFORE the rows that name it are
    // gone. Never fails — a machine that refuses to die must not make the
    // task undeletable (it warns, and leaks).
    crate::byoi_tasks::terminate(app, task_id);
    let options = DeleteTaskOptions {
        delete_worktree: delete_worktree.unwrap_or(true),
        delete_branch: delete_branch.unwrap_or(false),
    };
    // #135: the delete hands the sweep the card ids it FK-unlinked,
    // captured inside its own transaction (pass-2 finding 1: any capture
    // outside the tx races concurrent link writes) and delivered at
    // row-commit time, before the seconds-wide worktree teardown (pass-3
    // fix). The hook never fires on Err — fail-closed (#66) holds.
    app.deletion
        .delete_task(project_id, task_id, &options, |ids| {
            crate::step_engine::on_task_deleted(app, ids)
        })
        .map_err(|e| e.to_string())?;
    // E2-12: deleting a task closes its interactive terminals; ADR-0025:
    // and sweeps its tmux sessions (surviving orphans from crashes too).
    terminals.close_task(project_id, task_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fartcode_core::events::{EventBus, InternalEvent};
    use fartcode_core::issues::columns::{ColumnKind, ColumnStore, NewColumn, OnEnter, OnSettle};
    use fartcode_core::issues::NewIssue;

    #[test]
    fn first_line_title_strips_model_wrappers() {
        // gemma3:270m wraps titles in markdown bold despite instructions.
        assert_eq!(
            first_line_title(b"**Reconnect Buffer Queue**\n"),
            "Reconnect Buffer Queue"
        );
        // claude occasionally quotes and adds a period.
        assert_eq!(
            first_line_title(b"\"Fix save button.\"\n"),
            "Fix save button"
        );
        // Leading blank lines / spinner-adjacent whitespace skipped.
        assert_eq!(
            first_line_title(b"\n  \n  Fix settings save\n"),
            "Fix settings save"
        );
        // Empty output stays empty (caller treats as no-title).
        assert_eq!(first_line_title(b""), "");
        // 80-char cap.
        assert_eq!(first_line_title("x".repeat(200).as_bytes()).len(), 80);
    }

    fn app() -> Arc<App> {
        App::init(Some(":memory:")).expect("app init")
    }

    /// MockRuntime terminal manager — same pattern as the
    /// `commands::projects` tests (a Wry manager panics off the main
    /// thread, so tests drive `delete_task_blocking` directly).
    fn manager() -> Arc<crate::terminals::TerminalManager<tauri::test::MockRuntime>> {
        Arc::new(crate::terminals::TerminalManager::new(
            tauri::test::mock_app().handle().clone(),
        ))
    }

    /// A no-op ACP runtime (ported from `commands::projects` tests):
    /// task deletion only calls `stop_task`, and the fixture task owns
    /// no conversations, so the adapter resolver never fires.
    fn acp_runtime(app: &Arc<App>) -> Arc<crate::acp_runtime::AcpRuntime> {
        struct NoEvents;
        impl fartcode_acp::session::SessionEvents for NoEvents {
            fn update(&self, _: &str, _: &fartcode_acp::client::SessionUpdateEvent) {}
            fn transcript_changed(&self, _: &str, _: &fartcode_acp::LiveModels) {}
            fn permission_requested(
                &self,
                _: &str,
                _: &fartcode_acp::session::PermissionRequestedEvent,
            ) {
            }
        }
        crate::acp_runtime::AcpRuntime::new(
            app.conversations.clone(),
            app.tasks.clone(),
            app.db.clone(),
            app.provider_accounts.clone(),
            Arc::new(NoEvents),
            Arc::new(|provider: &str| {
                Err(fartcode_acp::Error::InvalidState(format!(
                    "no adapter in tests: {provider}"
                )))
            }),
        )
    }

    /// Board fixture mirroring `step_engine::tests`: project `p1` with
    /// default columns, a `project-root` workspace, a raw task row, and
    /// a "Plan" queue column for parks.
    fn seed_fixture(app: &App, task_id: &str) {
        {
            let conn = app.db.conn().lock().unwrap();
            conn.execute_batch(
                "INSERT INTO projects (id, name, path) VALUES ('p1', 'proj', '/tmp/proj');",
            )
            .unwrap();
            fartcode_core::issues::columns::seed_default_columns(&conn, "p1").unwrap();
            // `project-root` kind: TaskStore::delete requires a non-null
            // workspace_id, and this kind skips every worktree branch.
            conn.execute(
                "INSERT INTO workspaces (id, kind, path)
                 VALUES ('w1', 'project-root', '/tmp/proj')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (id, project_id, name, status, workspace_id)
                 VALUES (?1, 'p1', 't', 'in_progress', 'w1')",
                [task_id],
            )
            .unwrap();
        }
        ColumnStore::new(app.db.clone())
            .create(NewColumn {
                project_id: "p1".into(),
                name: "Plan".into(),
                kind: ColumnKind::AgentStep,
                counts_as_done: false,
                is_landing: false,
                on_enter: Some(OnEnter::Queue),
                on_settle: Some(OnSettle::Hold),
                advance_to: None,
                step_prompt: None,
                step_provider: None,
                step_model: None,
                step_effort: None,
                step_tools: None,
            })
            .unwrap();
    }

    /// A card parked on the "Plan" queue column and linked to `task_id`.
    fn parked_card(app: &App, title: &str, task_id: &str) -> fartcode_core::issues::Issue {
        let issue = app
            .issues
            .create(NewIssue {
                project_id: "p1".into(),
                title: title.into(),
                body: None,
                acceptance: Vec::new(),
                lane: None,
                provider: None,
                model: None,
                prd_path: None,
                prd_section: None,
                external_ref: None,
                dossier_path: None,
            })
            .unwrap();
        let queue = ColumnStore::new(app.db.clone())
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.name == "Plan")
            .expect("seed_fixture created the Plan queue column");
        crate::step_engine::enter_column_from_command(app, &issue.id, &queue.id, None).unwrap();
        app.issues
            .set_linked_task(&issue.id, Some(task_id))
            .unwrap();
        issue
    }

    fn parked_linked_issue(app: &App, task_id: &str) -> fartcode_core::issues::Issue {
        seed_fixture(app, task_id);
        parked_card(app, "card", task_id)
    }

    /// Drains the bus into just the `StepQueueCleared` issue ids.
    fn cleared_events(rx: &mut tokio::sync::broadcast::Receiver<InternalEvent>) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let InternalEvent::StepQueueCleared { issue_id, .. } = ev {
                out.push(issue_id);
            }
        }
        out
    }

    /// #135 wiring: deleting a task sweeps the linked card's park with
    /// the cleared event (the sweep itself is unit-proven in
    /// `step_engine::tests`; the #66 lesson is that call sites are where
    /// these bugs live). The card survives with the link FK-nulled.
    #[test]
    fn delete_task_unparks_the_linked_card() {
        let app = app();
        let issue = parked_linked_issue(&app, "t1");
        assert!(app.steps.peek_park(&issue.id).is_some());
        let mut rx = app.event_bus.subscribe();
        let terminals = manager();

        delete_task_blocking(
            &app,
            &terminals,
            &acp_runtime(&app),
            "p1",
            "t1",
            Some(false),
            Some(false),
        )
        .expect("delete_task");

        assert!(app.steps.peek_park(&issue.id).is_none(), "park swept");
        assert_eq!(cleared_events(&mut rx), vec![issue.id.clone()]);
        let card = app.issues.get(&issue.id).unwrap().expect("card survives");
        assert!(card.linked_task_id.is_none(), "FK cleared the link");
    }

    /// #135 AC3 (fail-closed, #66): a delete that errors before the row
    /// is gone leaves park, registry, and link untouched — the sweep
    /// must only run after a successful row delete.
    #[test]
    fn failed_delete_leaves_the_park_untouched() {
        let app = app();
        let issue = parked_linked_issue(&app, "t1");
        app.db
            .conn()
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER block_task_delete BEFORE DELETE ON tasks\n                 BEGIN SELECT RAISE(ABORT, 'blocked by test'); END;",
            )
            .unwrap();
        let mut rx = app.event_bus.subscribe();
        let terminals = manager();

        let err = delete_task_blocking(
            &app,
            &terminals,
            &acp_runtime(&app),
            "p1",
            "t1",
            Some(false),
            Some(false),
        )
        .expect_err("trigger blocks the row delete");

        assert!(err.contains("blocked"), "{err}");
        assert!(
            app.steps.peek_park(&issue.id).is_some(),
            "park survives the failed delete"
        );
        assert!(
            cleared_events(&mut rx).is_empty(),
            "no spurious cleared event"
        );
        assert_eq!(
            app.issues
                .get(&issue.id)
                .unwrap()
                .unwrap()
                .linked_task_id
                .as_deref(),
            Some("t1")
        );
    }

    /// A card LAUNCHED (not parked) into the seeded run-mode "Quick"
    /// column, linked to `task_id` — its trace is registry-only.
    fn launched_card(app: &App, title: &str, task_id: &str) -> fartcode_core::issues::Issue {
        let issue = app
            .issues
            .create(NewIssue {
                project_id: "p1".into(),
                title: title.into(),
                body: None,
                acceptance: Vec::new(),
                lane: None,
                provider: None,
                model: None,
                prd_path: None,
                prd_section: None,
                external_ref: None,
                dossier_path: None,
            })
            .unwrap();
        app.issues
            .set_linked_task(&issue.id, Some(task_id))
            .unwrap();
        let quick = ColumnStore::new(app.db.clone())
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.name == "Quick")
            .expect("seeded Quick column");
        crate::step_engine::enter_column_from_command(app, &issue.id, &quick.id, None).unwrap();
        issue
    }

    /// Adversarial finding 4: AC3's "registry untouched" half. A
    /// launched-but-unparked card's trace is registry-only — the park
    /// proxy cannot see it, so assert the registry directly across a
    /// delete that fails before the row is gone.
    #[test]
    fn failed_delete_leaves_the_registry_untouched() {
        let app = app();
        seed_fixture(&app, "t1");
        let issue = launched_card(&app, "card", "t1");
        assert!(
            app.steps.peek_park(&issue.id).is_none(),
            "launched, not parked"
        );
        assert!(app.steps.has_state(&issue.id), "registry entry exists");
        app.db
            .conn()
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER block_task_delete BEFORE DELETE ON tasks
                 BEGIN SELECT RAISE(ABORT, 'blocked by test'); END;",
            )
            .unwrap();
        let terminals = manager();

        delete_task_blocking(
            &app,
            &terminals,
            &acp_runtime(&app),
            "p1",
            "t1",
            Some(false),
            Some(false),
        )
        .expect_err("trigger blocks the row delete");

        assert!(
            app.steps.has_state(&issue.id),
            "registry survives the failed delete"
        );
    }

    /// Adversarial pass 2, finding 1: the sweep hook receives the
    /// FK-unlinked ids from INSIDE `delete_task`, right after the rows
    /// commit — not seconds later behind worktree teardown, where a
    /// confirm-born launch could be destroyed by a stale sweep.
    #[test]
    fn delete_task_hands_unlinked_ids_to_the_sweep_hook() {
        let app = app();
        seed_fixture(&app, "t1");
        let a = parked_card(&app, "card a", "t1");
        let b = parked_card(&app, "card b", "t1");
        let seen = std::sync::Mutex::new(Vec::<String>::new());

        app.deletion
            .delete_task(
                "p1",
                "t1",
                &DeleteTaskOptions {
                    delete_worktree: false,
                    delete_branch: false,
                },
                |ids| seen.lock().unwrap().extend(ids.iter().cloned()),
            )
            .expect("delete_task");

        let mut got = seen.into_inner().unwrap();
        got.sort();
        let mut want = vec![a.id.clone(), b.id.clone()];
        want.sort();
        assert_eq!(got, want);
    }

    /// Adversarial pass 2, finding 2: the idempotent early return
    /// (double-delete) succeeds WITHOUT invoking the sweep hook — a
    /// vanished task has already had its links FK-nulled.
    #[test]
    fn the_sweep_hook_stays_silent_for_a_missing_task() {
        let app = app();
        seed_fixture(&app, "t1");
        let called = std::sync::Mutex::new(false);

        app.deletion
            .delete_task(
                "p1",
                "no-such-task",
                &DeleteTaskOptions {
                    delete_worktree: false,
                    delete_branch: false,
                },
                |_ids| *called.lock().unwrap() = true,
            )
            .expect("missing task is a quiet no-op");

        assert!(!*called.lock().unwrap(), "hook must not fire");
    }

    /// Adversarial finding 3 (plural pin): every card linked to the task
    /// is swept, not just the first — two parks, two cleared events.
    #[test]
    fn delete_task_sweeps_every_linked_card() {
        let app = app();
        seed_fixture(&app, "t1");
        let a = parked_card(&app, "card a", "t1");
        let b = parked_card(&app, "card b", "t1");
        let mut rx = app.event_bus.subscribe();
        let terminals = manager();

        delete_task_blocking(
            &app,
            &terminals,
            &acp_runtime(&app),
            "p1",
            "t1",
            Some(false),
            Some(false),
        )
        .expect("delete_task");

        assert!(app.steps.peek_park(&a.id).is_none(), "first park swept");
        assert!(app.steps.peek_park(&b.id).is_none(), "second park swept");
        let mut cleared = cleared_events(&mut rx);
        cleared.sort();
        let mut want = vec![a.id.clone(), b.id.clone()];
        want.sort();
        assert_eq!(cleared, want);
    }
}
