//! Conversation commands (E2-08: list, create, delete).

use ade_core::conversations::{
    model::Conversation, ConversationScopeDto, ConversationStore, ConversationTypeDto,
    CreateConversationParams,
};
use serde::Serialize;

use crate::app::App;
use std::sync::Arc;
use tauri::State;

/// DTO for the frontend conversation list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDto {
    pub id: String,
    pub project_id: String,
    pub task_id: Option<String>,
    pub provider: Option<String>,
    pub title: String,
    pub agent_status: Option<String>,
    pub session_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<&Conversation> for ConversationDto {
    fn from(c: &Conversation) -> Self {
        Self {
            id: c.id.clone(),
            project_id: c.project_id.clone(),
            task_id: c.task_id.clone(),
            provider: c.provider.clone(),
            title: c.title.clone(),
            agent_status: c.agent_status.as_ref().map(|s| format!("{s:?}")),
            session_id: c.session_id.clone(),
            created_at: c.created_at.clone(),
            updated_at: c.updated_at.clone(),
        }
    }
}

#[tauri::command]
pub fn list_conversations(
    app: State<'_, Arc<App>>,
    task_id: String,
) -> Result<Vec<ConversationDto>, String> {
    app.conversations
        .list_by_task(&task_id)
        .map(|cs| cs.iter().map(ConversationDto::from).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_conversation(
    app: State<'_, Arc<App>>,
    project_id: String,
    task_id: String,
    provider: Option<String>,
    title: String,
    model: Option<String>,
    initial_prompt: Option<String>,
) -> Result<ConversationDto, String> {
    let params = CreateConversationParams {
        id: None,
        project_id,
        task_id: Some(task_id),
        scope: Some(ConversationScopeDto::Task),
        provider,
        title,
        auto_approve: None,
        model,
        initial_prompt,
        initial_queue: None,
        is_initial_conversation: false,
        r#type: Some(ConversationTypeDto::Pty),
    };
    app.conversations
        .create(params)
        .map(|c| ConversationDto::from(&c))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_conversation(
    app: State<'_, Arc<App>>,
    project_id: String,
    task_id: String,
    conversation_id: String,
) -> Result<(), String> {
    app.conversations
        .delete(&project_id, &task_id, &conversation_id)
        .map_err(|e| e.to_string())
}
