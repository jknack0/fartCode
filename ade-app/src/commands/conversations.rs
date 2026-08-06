//! Conversation + ACP commands (ticket E2-11-5).
//!
//! - `create_conversation` — the post-terminal-only-refactor creation path:
//!   the conversation's runtime type is decided SERVER-SIDE from the
//!   provider decision (ACP-capable provider → `'acp'`, else `'pty'`); the
//!   renderer never picks it.
//! - `list_conversations` — DTOs carry the resolved `runtime` field
//!   (`"tui"` | `"acp"`) so the frontend store can route ⌘Enter.
//! - `acp_start` / `acp_send_prompt` / `acp_cancel` /
//!   `acp_resolve_permission` / `acp_stop` / `acp_history` — thin wrappers
//!   over [`crate::acp_runtime::AcpRuntime`] (reference command surface,
//!   reference error mapping to `String`).

use ade_core::conversations::{
    resolve_session_path, ConversationScopeDto, ConversationStore, ConversationTypeDto,
    CreateConversationParams, SessionPath,
};
use serde::Serialize;
use std::sync::Arc;
use tauri::State;

use crate::acp_runtime::{AcpRuntime, AcpSendPromptResponse, AcpStartOutcome};
use crate::app::App;

/// One conversation for the frontend (camelCase like every other DTO).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDto {
    pub id: String,
    pub project_id: String,
    pub task_id: Option<String>,
    /// `"task" | "project"` (E17-04: the PM chat is project-scoped).
    pub scope: String,
    pub title: String,
    pub provider: Option<String>,
    pub r#type: String,
    pub session_id: Option<String>,
    /// The provider decision's output: `"acp"` runs structured chat,
    /// `"tui"` keeps the terminal path.
    pub runtime: &'static str,
}

fn conversation_dto(conv: &ade_core::conversations::model::Conversation) -> ConversationDto {
    ConversationDto {
        id: conv.id.clone(),
        project_id: conv.project_id.clone(),
        task_id: conv.task_id.clone(),
        scope: conv.scope.as_str().to_string(),
        title: conv.title.clone(),
        provider: conv.provider.clone(),
        r#type: conv.config.r#type.as_str().to_string(),
        session_id: conv.session_id.clone(),
        runtime: match resolve_session_path(conv) {
            SessionPath::Acp => "acp",
            SessionPath::Pty => "tui",
        },
    }
}

/// Creates a conversation under a task. The runtime type follows the
/// provider decision: ACP-capable provider → ACP conversation, anything
/// else (or missing provider) → PTY.
#[tauri::command]
pub fn create_conversation(
    app: State<'_, Arc<App>>,
    project_id: String,
    task_id: String,
    provider: Option<String>,
    title: Option<String>,
) -> Result<ConversationDto, String> {
    let acp_capable = provider
        .as_deref()
        .and_then(ade_providers::get)
        .is_some_and(|p| p.capabilities.acp);
    let r#type = if acp_capable {
        ConversationTypeDto::Acp
    } else {
        ConversationTypeDto::Pty
    };
    let conv = app
        .conversations
        .create(CreateConversationParams {
            id: None,
            project_id,
            task_id: Some(task_id),
            scope: Some(ConversationScopeDto::Task),
            provider,
            title: title.unwrap_or_else(|| "New conversation".to_string()),
            auto_approve: None,
            model: None,
            initial_prompt: None,
            initial_queue: None,
            is_initial_conversation: false,
            r#type: Some(r#type),
        })
        .map_err(|e| e.to_string())?;
    Ok(conversation_dto(&conv))
}

/// Lists a task's conversations with their resolved runtime field.
#[tauri::command]
pub fn list_conversations(
    app: State<'_, Arc<App>>,
    task_id: String,
) -> Result<Vec<ConversationDto>, String> {
    app.conversations
        .list_by_task(&task_id)
        .map(|convs| convs.iter().map(conversation_dto).collect())
        .map_err(|e| e.to_string())
}

// -- E17-04 project (PM chat) conversations -----------------------------------

/// Lists the project's project-scoped conversations (scope = 'project',
/// task_id NULL) — the PM chat panel's restore path.
#[tauri::command]
pub fn list_project_conversations(
    app: State<'_, Arc<App>>,
    project_id: String,
) -> Result<Vec<ConversationDto>, String> {
    app.conversations
        .list_by_project(&project_id)
        .map(|convs| {
            convs
                .iter()
                .filter(|c| c.scope == ade_core::conversations::model::ConversationScope::Project)
                .map(conversation_dto)
                .collect()
        })
        .map_err(|e| e.to_string())
}

/// Returns the project's ACP conversation, creating it on first use (E17-04:
/// exactly one persistent PM conversation per project). The provider must be
/// ACP-capable — the PM chat has no TUI fallback.
#[tauri::command]
pub fn get_or_create_project_conversation(
    app: State<'_, Arc<App>>,
    project_id: String,
    provider: String,
) -> Result<ConversationDto, String> {
    let acp_capable = ade_providers::get(&provider).is_some_and(|p| p.capabilities.acp);
    if !acp_capable {
        return Err(format!(
            "PM chat requires an ACP-capable provider; {provider:?} is not"
        ));
    }
    let existing = app
        .conversations
        .list_by_project(&project_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|c| {
            c.scope == ade_core::conversations::model::ConversationScope::Project
                && c.config.r#type == ade_core::conversations::model::ConversationType::Acp
        });
    if let Some(conv) = existing {
        return Ok(conversation_dto(&conv));
    }
    let conv = app
        .conversations
        .create(CreateConversationParams {
            id: None,
            project_id,
            task_id: None,
            scope: Some(ConversationScopeDto::Project),
            provider: Some(provider),
            title: "Project chat".to_string(),
            auto_approve: None,
            model: None,
            initial_prompt: None,
            initial_queue: None,
            is_initial_conversation: false,
            r#type: Some(ConversationTypeDto::Acp),
        })
        .map_err(|e| e.to_string())?;
    Ok(conversation_dto(&conv))
}

// -- ACP command surface ------------------------------------------------------

/// Starts (or resumes) the conversation's ACP session. Rejected for
/// non-ACP conversations — the TUI path is untouched by design.
#[tauri::command]
pub async fn acp_start(
    runtime: State<'_, Arc<AcpRuntime>>,
    conversation_id: String,
) -> Result<AcpStartOutcome, String> {
    runtime
        .start(&conversation_id)
        .await
        .map_err(|e| e.to_string())
}

/// Sends a prompt (⌘Enter): runs immediately when idle, queues otherwise.
#[tauri::command]
pub async fn acp_send_prompt(
    runtime: State<'_, Arc<AcpRuntime>>,
    conversation_id: String,
    text: String,
    hidden_context: Option<String>,
) -> Result<AcpSendPromptResponse, String> {
    runtime
        .send_prompt(&conversation_id, &text, hidden_context.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// Cancels the in-flight turn.
#[tauri::command]
pub async fn acp_cancel(
    runtime: State<'_, Arc<AcpRuntime>>,
    conversation_id: String,
) -> Result<(), String> {
    runtime
        .cancel(&conversation_id)
        .await
        .map_err(|e| e.to_string())
}

/// Resolves a surfaced permission request (option id from the prompt card).
#[tauri::command]
pub fn acp_resolve_permission(
    runtime: State<'_, Arc<AcpRuntime>>,
    conversation_id: String,
    request_id: String,
    option_id: String,
) -> Result<(), String> {
    runtime
        .resolve_permission(&conversation_id, &request_id, &option_id)
        .map_err(|e| e.to_string())
}

/// Stops the conversation's session (adapter teardown).
#[tauri::command]
pub fn acp_stop(
    runtime: State<'_, Arc<AcpRuntime>>,
    conversation_id: String,
) -> Result<(), String> {
    runtime.stop(&conversation_id).map_err(|e| e.to_string())
}

/// The full live-model snapshot: committed + active turns, config, usage,
/// title, agents, plan, draft, queued prompts (#33's renderer hydrates
/// from this). `null` when the conversation is not running.
#[tauri::command]
pub fn acp_history(
    runtime: State<'_, Arc<AcpRuntime>>,
    conversation_id: String,
) -> Result<Option<ade_acp::LiveModels>, String> {
    Ok(runtime.live_models(&conversation_id))
}
