//! Conversation domain types (ticket E2-05).
//!
//! Ports `shared/core/conversations/conversations.ts` +
//! `conversation-config.ts` + `main/core/conversations/utils.ts`
//! (`mapConversationRowToConversation`). The versioned `conversations.config`
//! JSON uses the reference's INLINE version scheme (`{"version":"1", ...}`),
//! not the `db::versioned_json` wrapper — parsing is loss-tolerant and never
//! panics (corrupt/missing config falls back to a default pty config).

use serde::{Deserialize, Serialize};

pub const MAX_CONVERSATION_TITLE_LENGTH: usize = 100;

/// `'pty'` (terminal/PTY path) | `'acp'` (Agent Client Protocol, Phase 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationType {
    Pty,
    Acp,
}

impl ConversationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConversationType::Pty => "pty",
            ConversationType::Acp => "acp",
        }
    }

    pub fn parse(s: &str) -> ConversationType {
        match s {
            "acp" => ConversationType::Acp,
            _ => ConversationType::Pty,
        }
    }
}

/// `'task'` (default) | `'project'`. Phase 0 creates task-scoped only;
/// project-scoped conversations (`task_id = NULL`) arrive with Phase 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationScope {
    Task,
    Project,
}

impl ConversationScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConversationScope::Task => "task",
            ConversationScope::Project => "project",
        }
    }

    pub fn parse(s: &str) -> ConversationScope {
        match s {
            "project" => ConversationScope::Project,
            _ => ConversationScope::Task,
        }
    }
}

/// Agent lifecycle status on the conversation row (reference `AgentStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Idle,
    Working,
    AwaitingInput,
    Error,
    Completed,
}

impl AgentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentStatus::Idle => "idle",
            AgentStatus::Working => "working",
            AgentStatus::AwaitingInput => "awaiting-input",
            AgentStatus::Error => "error",
            AgentStatus::Completed => "completed",
        }
    }

    /// Tolerant: unknown values parse to `None` (agent status is best-effort
    /// telemetry on the row, never load-bearing).
    pub fn parse(s: &str) -> Option<AgentStatus> {
        match s {
            "idle" => Some(AgentStatus::Idle),
            "working" => Some(AgentStatus::Working),
            "awaiting-input" => Some(AgentStatus::AwaitingInput),
            "error" => Some(AgentStatus::Error),
            "completed" => Some(AgentStatus::Completed),
            _ => None,
        }
    }
}

/// A queued prompt for ACP first spawn (reference `InitialQueuePrompt`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitialQueuePrompt {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden_context: Option<String>,
}

/// Versioned v1 conversation config (reference `conversation-config.ts`):
/// `{"version":"1","type":"pty"|"acp", autoApprove?, initialPrompt?, model?,
/// initialQueue?}`. Delta-shaped — absent fields are omitted, never null.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationConfig {
    pub r#type: ConversationType,
    pub auto_approve: Option<bool>,
    /// Delivered once on first spawn (gated on `session_id` being null).
    pub initial_prompt: Option<String>,
    /// Empty string or absent = the CLI's default model.
    pub model: Option<String>,
    /// ACP only (Phase 2). `initialPrompt` remains readable for legacy rows.
    pub initial_queue: Option<Vec<InitialQueuePrompt>>,
}

impl Default for ConversationConfig {
    fn default() -> Self {
        Self {
            r#type: ConversationType::Pty,
            auto_approve: None,
            initial_prompt: None,
            model: None,
            initial_queue: None,
        }
    }
}

impl ConversationConfig {
    pub fn to_json(&self) -> serde_json::Value {
        let mut obj = serde_json::Map::new();
        obj.insert("version".into(), serde_json::json!("1"));
        obj.insert("type".into(), serde_json::json!(self.r#type.as_str()));
        if let Some(aa) = self.auto_approve {
            obj.insert("autoApprove".into(), serde_json::json!(aa));
        }
        if let Some(p) = &self.initial_prompt {
            obj.insert("initialPrompt".into(), serde_json::json!(p));
        }
        if let Some(m) = &self.model {
            obj.insert("model".into(), serde_json::json!(m));
        }
        if let Some(q) = &self.initial_queue {
            obj.insert("initialQueue".into(), serde_json::json!(q));
        }
        serde_json::Value::Object(obj)
    }

    /// Loss-tolerant parse: corrupt / missing / unknown-version configs fall
    /// back to a default pty config (never panics).
    pub fn from_json(value: Option<&serde_json::Value>) -> ConversationConfig {
        let Some(value) = value.filter(|v| v.is_object()) else {
            return ConversationConfig::default();
        };
        let r#type = value
            .get("type")
            .and_then(|t| t.as_str())
            .map(ConversationType::parse)
            .unwrap_or(ConversationType::Pty);
        let auto_approve = value.get("autoApprove").and_then(|a| a.as_bool());
        let initial_prompt = value
            .get("initialPrompt")
            .and_then(|p| p.as_str())
            .map(String::from);
        let model = value
            .get("model")
            .and_then(|m| m.as_str())
            .map(String::from);
        let initial_queue = value
            .get("initialQueue")
            .and_then(|q| q.as_array())
            .map(|q| {
                q.iter()
                    .filter_map(|p| {
                        let text = p.get("text").and_then(|t| t.as_str())?;
                        Some(InitialQueuePrompt {
                            text: text.to_string(),
                            hidden_context: p
                                .get("hiddenContext")
                                .and_then(|h| h.as_str())
                                .map(String::from),
                        })
                    })
                    .collect()
            })
            .filter(|q: &Vec<InitialQueuePrompt>| !q.is_empty());
        ConversationConfig {
            r#type,
            auto_approve,
            initial_prompt,
            model,
            initial_queue,
        }
    }
}

/// The durable conversation (reference `Conversation`).
#[derive(Debug, Clone)]
pub struct Conversation {
    pub id: String,
    pub project_id: String,
    /// `None` for project-scoped conversations (Phase 1).
    pub task_id: Option<String>,
    pub scope: ConversationScope,
    pub title: String,
    pub provider: Option<String>,
    pub config: ConversationConfig,
    /// The agent-facing session identifier. Null = never spawned. Set to the
    /// conversation id before the first PTY spawn; later may be overwritten
    /// by the agent's native id. For ACP: set to the id returned by
    /// newSession/loadSession.
    pub session_id: Option<String>,
    pub is_initial_conversation: Option<bool>,
    pub agent_status: Option<AgentStatus>,
    pub agent_status_seen: bool,
    pub r#type: ConversationType,
    pub last_interacted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Conversation {
    /// `initialQueueFromRow` (reference utils.ts): ACP conversations that
    /// have never spawned carry their queued prompts — from `initialQueue`,
    /// or the legacy `initialPrompt` as a single prompt.
    pub fn initial_queue(&self) -> Option<Vec<InitialQueuePrompt>> {
        if self.session_id.is_some() || self.config.r#type != ConversationType::Acp {
            return None;
        }
        if let Some(queue) = &self.config.initial_queue {
            if !queue.is_empty() {
                return Some(queue.clone());
            }
        }
        let legacy = self
            .config
            .initial_prompt
            .as_ref()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty());
        legacy.map(|p| {
            vec![InitialQueuePrompt {
                text: p.to_string(),
                hidden_context: None,
            }]
        })
    }
}

pub(crate) const CONVERSATION_COLUMNS: &str = "id, project_id, task_id, scope, title, provider, \
     config, session_id, is_initial_conversation, agent_status, agent_status_seen, type, \
     last_interacted_at, created_at, updated_at";

pub(crate) fn conversation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Conversation> {
    let config_json: Option<String> = row.get(6)?;
    let config_value = config_json
        .as_deref()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok());
    let r#type_raw: Option<String> = row.get(11)?;
    let agent_status_raw: Option<String> = row.get(9)?;
    Ok(Conversation {
        id: row.get(0)?,
        project_id: row.get(1)?,
        task_id: row.get(2)?,
        scope: ConversationScope::parse(&row.get::<_, String>(3)?),
        title: row.get(4)?,
        provider: row.get(5)?,
        config: ConversationConfig::from_json(config_value.as_ref()),
        session_id: row.get(7)?,
        is_initial_conversation: row.get(8)?,
        agent_status: agent_status_raw.as_deref().and_then(AgentStatus::parse),
        agent_status_seen: row.get::<_, i64>(10)? != 0,
        r#type: r#type_raw
            .as_deref()
            .map(ConversationType::parse)
            .unwrap_or_else(|| ConversationConfig::from_json(config_value.as_ref()).r#type),
        last_interacted_at: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}
