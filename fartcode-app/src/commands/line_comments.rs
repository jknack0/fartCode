//! Line-comment commands (E4-10, #50; ARCHITECTURE.md §14): CRUD over the
//! domain store plus `create_task_from_comment` — the "Create Task" flow
//! that turns a review comment into a provisioned task whose initial prompt
//! is the §14 template.
//!
//! **UI thread (#80):** `create_task_from_comment` is the heaviest command
//! in this module — `git rev-parse --abbrev-ref HEAD` for the branch line,
//! a full `create_with_provision` (fetch / branch / `worktree add` / push),
//! then a PTY spawn per auto-run lifecycle script. A non-async
//! `#[tauri::command]` runs that inline on the IPC thread, which on macOS
//! is the main thread, so the NSRunLoop stalls and the window stops
//! repainting. It is therefore `async` **and** hands the work to
//! `spawn_blocking` (merely `async` would block a tokio worker instead).
//! The wire contract — argument names, serialized result, error strings,
//! side-effect ordering — is unchanged. The CRUD commands stay synchronous:
//! each is a single indexed DB statement.

use std::sync::Arc;

use fartcode_core::git::GitOps;
use fartcode_core::line_comments::{
    build_comment_prompt, CommentPromptContext, LineComment, SourceSide,
};
use fartcode_core::tasks::operations::{InitialConversationConfig, TaskConfigParams};
use tauri::State;

use crate::app::App;
use crate::commands::lifecycle::run_auto_lifecycle_scripts;
use crate::commands::tasks::create_task_params;
use crate::terminals::TerminalManager;

/// Request body for [`add_line_comment`] (frontend sends one object).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddLineCommentRequest {
    pub task_id: String,
    pub file_path: String,
    pub line_number: i64,
    pub line_end: Option<i64>,
    pub source_side: String,
    pub line_content: Option<String>,
    pub content: String,
}

/// "Add Note" / agent-tool surface: persists a comment anchored to a diff
/// line range and emits `comment:created`.
#[tauri::command]
pub fn add_line_comment(
    app: State<'_, Arc<App>>,
    request: AddLineCommentRequest,
) -> Result<LineComment, String> {
    let side = SourceSide::parse(&request.source_side).map_err(String::from)?;
    app.line_comments
        .add(fartcode_core::line_comments::AddLineCommentOptions {
            task_id: request.task_id,
            file_path: request.file_path,
            line_number: request.line_number,
            line_end: request.line_end,
            source_side: side,
            line_content: request.line_content,
            content: request.content,
            created_by: None,
        })
        .map_err(String::from)
}

/// Agent-tool surface (E4-11, #51): validated against the task's workspace
/// (path containment, file existence, in-range anchor) and attributed to the
/// provider as `created_by = agent:<provider>`. See decisions/0035 for the
/// agent-call mechanism.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAddLineCommentRequest {
    pub task_id: String,
    pub file_path: String,
    pub line_start: i64,
    pub line_end: Option<i64>,
    pub source_side: String,
    pub content: String,
    /// Provider id making the call (`claude`, …) — lands in attribution.
    pub provider: String,
}

#[tauri::command]
pub fn agent_add_line_comment(
    app: State<'_, Arc<App>>,
    request: AgentAddLineCommentRequest,
) -> Result<LineComment, String> {
    let side = SourceSide::parse(&request.source_side).map_err(String::from)?;
    app.line_comments
        .add_agent_comment(
            fartcode_core::line_comments::AddLineCommentOptions {
                task_id: request.task_id,
                file_path: request.file_path,
                line_number: request.line_start,
                line_end: request.line_end,
                source_side: side,
                line_content: None,
                content: request.content,
                created_by: None,
            },
            &request.provider,
        )
        .map_err(String::from)
}

/// Comments for a task (optionally narrowed to one file) — the diff gutter
/// rehydrates from here on tab open / restart.
#[tauri::command]
pub fn list_line_comments(
    app: State<'_, Arc<App>>,
    task_id: String,
    file_path: Option<String>,
) -> Result<Vec<LineComment>, String> {
    app.line_comments
        .list_for_task(&task_id, file_path.as_deref())
        .map_err(String::from)
}

/// Manual resolution (§14 decision: linked-task completion does NOT
/// auto-resolve). Emits `comment:resolved`.
#[tauri::command]
pub fn resolve_line_comment(
    app: State<'_, Arc<App>>,
    comment_id: String,
) -> Result<LineComment, String> {
    app.line_comments.resolve(&comment_id).map_err(String::from)
}

#[tauri::command]
pub fn delete_line_comment(app: State<'_, Arc<App>>, comment_id: String) -> Result<(), String> {
    app.line_comments.delete(&comment_id).map_err(String::from)
}

/// Result of the "Create Task" flow: the provisioned task + the §14 prompt
/// the UI pastes into the freshly-spawned agent terminal.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskFromCommentResult {
    pub task: fartcode_core::tasks::TaskDto,
    pub prompt: String,
}

/// "Create Task" (§14): creates a provisioned task linked bidirectionally
/// to the comment, whose initial conversation carries the §14 prompt.
/// `selected_code` / `enclosing_function` arrive from the diff selection —
/// the comment row only snapshots the first selected line.
#[tauri::command]
pub async fn create_task_from_comment(
    app: State<'_, Arc<App>>,
    terminals: State<'_, Arc<TerminalManager>>,
    project_id: String,
    name: String,
    comment_id: String,
    selected_code: String,
    enclosing_function: Option<String>,
) -> Result<CreateTaskFromCommentResult, String> {
    // `State` borrows cannot cross into the blocking closure; both managed
    // values are `Arc`s, so clone the handles and move those.
    create_task_from_comment_off_thread(
        app.inner().clone(),
        terminals.inner().clone(),
        project_id,
        name,
        comment_id,
        selected_code,
        enclosing_function,
    )
    .await
}

/// [`create_task_from_comment`] with the Tauri `State` already unwrapped
/// and generic over the runtime, so tests can drive it with
/// `tauri::test::MockRuntime`. Runs [`create_task_from_comment_core`]
/// verbatim on the blocking pool and maps a join failure to the command's
/// `String` error type.
pub(crate) async fn create_task_from_comment_off_thread<R: tauri::Runtime>(
    app: Arc<App>,
    terminals: Arc<TerminalManager<R>>,
    project_id: String,
    name: String,
    comment_id: String,
    selected_code: String,
    enclosing_function: Option<String>,
) -> Result<CreateTaskFromCommentResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        create_task_from_comment_core(
            &app,
            &terminals,
            &project_id,
            &name,
            &comment_id,
            &selected_code,
            enclosing_function.as_deref(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Command core (testable without a Tauri `State`): prompt construction +
/// provisioned task creation + bidirectional comment link.
pub fn create_task_from_comment_core<R: tauri::Runtime>(
    app: &App,
    terminals: &TerminalManager<R>,
    project_id: &str,
    name: &str,
    comment_id: &str,
    selected_code: &str,
    enclosing_function: Option<&str>,
) -> Result<CreateTaskFromCommentResult, String> {
    let comment = app
        .line_comments
        .get(comment_id)
        .map_err(String::from)?
        .ok_or_else(|| format!("line comment not found: {comment_id}"))?;

    // Branch context for the prompt: the current branch of the reviewed
    // task's workspace (best-effort — omitted when unresolvable).
    let branch = reviewed_workspace_branch(app, &comment.task_id);

    let line_end = comment.line_end.unwrap_or(comment.line_number);
    let prompt = build_comment_prompt(&CommentPromptContext {
        file_path: &comment.file_path,
        branch: branch.as_deref(),
        enclosing_function,
        line_start: comment.line_number,
        line_end,
        selected_code,
        comment: &comment.content,
    });

    let mut config = InitialConversationConfig::new(
        uuid::Uuid::new_v4().to_string(),
        "claude",
        format!("Comment: {}", truncate_for_title(&comment.content)),
    );
    config.initial_prompt = Some(prompt.clone());

    let params = create_task_params(
        app,
        project_id,
        name,
        TaskConfigParams {
            name: name.to_string(),
            initial_status: None,
            linked_issue: None,
            initial_conversation: Some(config),
        },
    )?;
    let success = app
        .task_creation
        .create_with_provision(params)
        .map_err(String::from)?;

    // Bidirectional link (§14): comment.linked_task_id + tasks.source_comment_id.
    app.line_comments
        .link_task(comment_id, &success.task.id)
        .map_err(String::from)?;

    // E1-06: auto-run setup/run scripts on task creation when enabled.
    run_auto_lifecycle_scripts(terminals, app, &success.task.id);

    Ok(CreateTaskFromCommentResult {
        task: fartcode_core::tasks::TaskDto::from(&success.task),
        prompt,
    })
}

/// Current branch of the task's materialized workspace (the §14 BRANCH
/// line). `None` when the task has no local workspace path or git can't
/// answer — the prompt omits the line in that case.
fn reviewed_workspace_branch(app: &App, task_id: &str) -> Option<String> {
    let path: String = {
        let conn = app.db.conn().lock().ok()?;
        conn.query_row(
            "SELECT w.path FROM tasks t
               JOIN workspaces w ON w.id = t.workspace_id
              WHERE t.id = ?1",
            [task_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
    }?;
    fartcode_git::CliGit
        .current_branch(std::path::Path::new(&path))
        .ok()
        .flatten()
}

fn truncate_for_title(s: &str) -> String {
    let first_line = s.lines().next().unwrap_or("").trim();
    if first_line.chars().count() <= 40 {
        first_line.to_string()
    } else {
        let cut: String = first_line.chars().take(40).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fartcode_core::projects::ProjectStore;
    use fartcode_core::settings::SettingsStore;
    use std::path::Path;
    use std::process::Command;
    use std::sync::Mutex;
    use std::time::Duration;

    /// Every await here is bounded — an unbounded wait would wedge the
    /// suite instead of failing it.
    const TEST_TIMEOUT: Duration = Duration::from_secs(60);

    /// Drives a command future on a **single-threaded** runtime — the shape
    /// of the IPC thread, so a body that fails to leave the thread is
    /// observable.
    fn block_on_bounded<F: std::future::Future>(fut: F) -> F::Output {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move {
            tokio::time::timeout(TEST_TIMEOUT, fut)
                .await
                .expect("command future timed out")
        })
    }

    fn terminal_manager() -> Arc<TerminalManager<tauri::test::MockRuntime>> {
        Arc::new(TerminalManager::new(
            tauri::test::mock_app().handle().clone(),
        ))
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=t@t", "-c", "user.name=t"])
            .args(args)
            .output()
            .expect("git spawns");
        assert!(out.status.success(), "git {args:?} failed");
    }

    fn repo_fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init", "-b", "main"]);
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src/auth.rs"), "fn validate() {}\n").unwrap();
        git(tmp.path(), &["add", "."]);
        git(tmp.path(), &["commit", "-m", "init"]);
        tmp
    }

    /// Project over a fixture repo with the worktree pool redirected into a
    /// tempdir, plus a reviewed task + workspace + one review comment.
    /// Mirrors `tests/line_comments_integration.rs` so the async path is
    /// asserted against the same fixture the sync path was.
    struct Fixture {
        app: Arc<App>,
        project_id: String,
        comment_id: String,
        _repo: tempfile::TempDir,
        _worktrees: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let repo = repo_fixture();
        let worktrees = tempfile::tempdir().unwrap();
        let app = App::init(Some(":memory:")).expect("app init");
        let project = app
            .projects
            .create_local(repo.path(), false)
            .expect("create project");
        let local: fartcode_core::settings::LocalProjectGroup =
            serde_json::from_value(app.settings.get_json("localProject").unwrap()).unwrap();
        app.settings
            .set_json(
                "localProject",
                serde_json::json!({
                    "defaultProjectsDirectory": local.default_projects_directory,
                    "defaultWorktreeDirectory": worktrees.path().to_string_lossy(),
                    "writeAgentConfigToGitIgnore": local.write_agent_config_to_git_ignore,
                }),
            )
            .expect("set localProject");
        {
            let conn = app.db.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO tasks (id, project_id, name, status)
                 VALUES ('task-reviewed', ?1, 'reviewed', 'in_progress')",
                [&project.id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO workspaces (id, type, kind, location, path)
                 VALUES ('ws-reviewed', 'local', 'worktree', 'local', ?1)",
                [repo.path().to_string_lossy().as_ref()],
            )
            .unwrap();
            conn.execute(
                "UPDATE tasks SET workspace_id = 'ws-reviewed' WHERE id = 'task-reviewed'",
                [],
            )
            .unwrap();
        }
        let comment = app
            .line_comments
            .add(fartcode_core::line_comments::AddLineCommentOptions {
                task_id: "task-reviewed".into(),
                file_path: "src/auth.rs".into(),
                line_number: 1,
                line_end: Some(1),
                source_side: SourceSide::After,
                line_content: Some("fn validate() {}".into()),
                content: "This should return Result".into(),
                created_by: None,
            })
            .expect("add comment");
        Fixture {
            app,
            project_id: project.id,
            comment_id: comment.id,
            _repo: repo,
            _worktrees: worktrees,
        }
    }

    #[test]
    fn off_thread_create_task_from_comment_matches_the_sync_contract() {
        let f = fixture();
        let result = block_on_bounded(create_task_from_comment_off_thread(
            f.app.clone(),
            terminal_manager(),
            f.project_id.clone(),
            "fix-validate".into(),
            f.comment_id.clone(),
            "fn validate() {}".into(),
            Some("fn validate()".into()),
        ))
        .expect("create task from comment");

        // §14 prompt template, byte for byte the same fields as before.
        assert!(result
            .prompt
            .starts_with("You are reviewing code in a git diff.\n"));
        assert!(result.prompt.contains("FILE: src/auth.rs"));
        assert!(result.prompt.contains("BRANCH: main"));
        assert!(result.prompt.contains("ENCLOSING FUNCTION: fn validate()"));
        assert!(result
            .prompt
            .contains("SELECTED CODE (line 1):\nfn validate() {}"));
        assert!(result
            .prompt
            .contains("COMMENT FROM REVIEWER:\nThis should return Result"));

        // Bidirectional link (§14) — comment → task AND task → comment.
        let comment = f.app.line_comments.get(&f.comment_id).unwrap().unwrap();
        assert_eq!(
            comment.linked_task_id.as_deref(),
            Some(result.task.id.as_str())
        );
        let source: Option<String> = {
            let conn = f.app.db.conn().lock().unwrap();
            conn.query_row(
                "SELECT source_comment_id FROM tasks WHERE id = ?1",
                [&result.task.id],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(source.as_deref(), Some(f.comment_id.as_str()));

        // Still provisioned (the worktree is materialized, not deferred).
        assert!(result.task.workspace_id.is_some());
        let ws_path: String = {
            let conn = f.app.db.conn().lock().unwrap();
            conn.query_row(
                "SELECT path FROM workspaces WHERE id = ?1",
                [result.task.workspace_id.as_deref().unwrap()],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert!(Path::new(&ws_path).exists(), "worktree missing: {ws_path}");

        // The initial conversation still carries the prompt.
        let config: serde_json::Value = {
            let conn = f.app.db.conn().lock().unwrap();
            let raw: String = conn
                .query_row(
                    "SELECT config FROM conversations WHERE task_id = ?1
                      AND is_initial_conversation = 1",
                    [&result.task.id],
                    |row| row.get(0),
                )
                .unwrap();
            serde_json::from_str(&raw).unwrap()
        };
        assert_eq!(config["version"], "1");
        assert_eq!(config["initialPrompt"], result.prompt);
    }

    #[test]
    fn off_thread_create_task_from_comment_preserves_the_error_string() {
        let f = fixture();
        let sync_err = create_task_from_comment_core(
            &f.app,
            &terminal_manager(),
            &f.project_id,
            "x",
            "lc_missing",
            "code",
            None,
        )
        .expect_err("missing comment fails");

        let async_err = block_on_bounded(create_task_from_comment_off_thread(
            f.app.clone(),
            terminal_manager(),
            f.project_id.clone(),
            "x".into(),
            "lc_missing".into(),
            "code".into(),
            None,
        ))
        .expect_err("missing comment fails");

        assert_eq!(async_err, sync_err);
        assert_eq!(async_err, "line comment not found: lc_missing");
    }

    /// The #80 property: on a single-threaded runtime a co-scheduled task
    /// can only run while the awaited command is pending, so `probe` before
    /// `command` proves the body left the calling thread. The error path is
    /// used deliberately — it exercises the same `spawn_blocking` hop
    /// without provisioning a worktree.
    #[test]
    fn create_task_from_comment_yields_the_calling_thread() {
        let f = fixture();
        let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let probe_order = order.clone();
        let command_order = order.clone();
        let app = f.app.clone();
        let project_id = f.project_id.clone();
        block_on_bounded(async move {
            tokio::spawn(async move { probe_order.lock().unwrap().push("probe") });
            let err = create_task_from_comment_off_thread(
                app,
                terminal_manager(),
                project_id,
                "x".into(),
                "lc_missing".into(),
                "code".into(),
                None,
            )
            .await;
            command_order.lock().unwrap().push("command");
            err.expect_err("missing comment fails");
        });

        assert_eq!(
            order.lock().unwrap().as_slice(),
            ["probe", "command"],
            "create_task_from_comment blocked its caller instead of yielding"
        );
    }
}
