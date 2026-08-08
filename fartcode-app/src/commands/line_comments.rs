//! Line-comment commands (E4-10, #50; ARCHITECTURE.md §14): CRUD over the
//! domain store plus `create_task_from_comment` — the "Create Task" flow
//! that turns a review comment into a provisioned task whose initial prompt
//! is the §14 template.

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
pub fn create_task_from_comment(
    app: State<'_, Arc<App>>,
    terminals: State<'_, Arc<TerminalManager>>,
    project_id: String,
    name: String,
    comment_id: String,
    selected_code: String,
    enclosing_function: Option<String>,
) -> Result<CreateTaskFromCommentResult, String> {
    create_task_from_comment_core(
        &app,
        &terminals,
        &project_id,
        &name,
        &comment_id,
        &selected_code,
        enclosing_function.as_deref(),
    )
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
