//! Conversation session supervisor (ticket E2-05).
//!
//! Ports `main/core/conversations/` — CRUD + hydrate/dehydrate + session-id
//! persistence + resume resolution. The actual PTY / ACP session machinery
//! (E2-06 / Phase 2) hooks in where the reference calls
//! `task.conversations.startSession` / `detachSession`.

pub mod model;

use std::sync::Arc;

use rusqlite::OptionalExtension;

use crate::db::Db;
use crate::Error;
use model::{
    conversation_from_row, Conversation, ConversationConfig, ConversationScope, ConversationType,
    InitialQueuePrompt, CONVERSATION_COLUMNS,
};

pub use model::{
    AgentStatus as ConversationAgentStatus, ConversationConfig as ConversationConfigDto,
    ConversationScope as ConversationScopeDto, ConversationType as ConversationTypeDto,
    MAX_CONVERSATION_TITLE_LENGTH,
};

/// Providers that require the agent's NATIVE session id to resume
/// (reference `PROVIDER_SESSION_ID_REQUIRED_FOR_RESUME` — must match exactly).
pub const PROVIDER_SESSION_ID_REQUIRED_FOR_RESUME: [&str; 7] = [
    "amp",
    "codex",
    "commandcode",
    "droid",
    "goose",
    "oh-my-pi",
    "pi",
];

/// Deterministic PTY session id: `<projectId>:<scopeId>:<leafId>` (reference
/// `shared/core/pty/ptySessionId.ts`). One active PTY per leaf entity.
pub fn make_pty_session_id(project_id: &str, scope_id: &str, leaf_id: &str) -> String {
    format!("{project_id}:{scope_id}:{leaf_id}")
}

/// Inverse of `make_pty_session_id`; `None` unless exactly 3 non-empty parts.
pub fn parse_pty_session_id(id: &str) -> Option<(String, String, String)> {
    let parts: Vec<&str> = id.split(':').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    Some((
        parts[0].to_string(),
        parts[1].to_string(),
        parts[2].to_string(),
    ))
}

/// Resume resolution (reference `resolve-agent-session-command.ts` — ported
/// exactly). Returns `(sessionId, isResuming)`.
///
/// - Providers in the 7-set, when resuming: use `conversation.session_id`
///   iff set and `!= conversation.id`; otherwise fall back to the
///   conversation id with `isResuming` cleared (unless the caller explicitly
///   allows the fallback with `require_provider_session_id == false`).
/// - Every other provider: `(conversation.id, isResuming)` untouched.
pub fn resolve_agent_session_command_args(
    provider_id: &str,
    conversation_id: &str,
    conversation_session_id: Option<&str>,
    is_resuming: bool,
    require_provider_session_id: Option<bool>,
) -> (String, bool) {
    if PROVIDER_SESSION_ID_REQUIRED_FOR_RESUME.contains(&provider_id) && is_resuming {
        if let Some(native) = conversation_session_id.filter(|s| *s != conversation_id) {
            return (native.to_string(), true);
        }
        if require_provider_session_id == Some(false) {
            return (conversation_id.to_string(), is_resuming);
        }
        return (conversation_id.to_string(), false);
    }
    (conversation_id.to_string(), is_resuming)
}

/// How a conversation's agent session is delivered (ticket E2-11-3
/// "provider decision hook").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPath {
    /// Structured chat over ACP (E2-11): needs an ACP-type conversation AND
    /// a provider with `capabilities.acp`.
    Acp,
    /// Terminal/TUI path (E2-06 launcher) — the default for everything else,
    /// untouched by the ACP work.
    Pty,
}

/// Supervisor decision: which path delivers this conversation's agent
/// session. ACP requires BOTH the conversation's config type (`'acp'`) and
/// the provider registry's `capabilities.acp`; any miss falls back to the
/// TUI/PTY path so non-ACP providers behave exactly as before E2-11.
pub fn resolve_session_path(conversation: &Conversation) -> SessionPath {
    let acp_capable = conversation
        .provider
        .as_deref()
        .and_then(ade_providers::get)
        .is_some_and(|p| p.capabilities.acp);
    if conversation.config.r#type == ConversationType::Acp && acp_capable {
        SessionPath::Acp
    } else {
        SessionPath::Pty
    }
}

/// Typed create params (reference `CreateConversationParams`).
#[derive(Debug, Clone)]
pub struct CreateConversationParams {
    pub id: Option<String>,
    pub project_id: String,
    /// `None` = project-scoped — Phase 0 rejects with a typed error.
    pub task_id: Option<String>,
    pub scope: Option<ConversationScope>,
    pub provider: Option<String>,
    pub title: String,
    pub auto_approve: Option<bool>,
    pub model: Option<String>,
    pub initial_prompt: Option<String>,
    pub initial_queue: Option<Vec<InitialQueuePrompt>>,
    pub is_initial_conversation: bool,
    pub r#type: Option<ConversationType>,
}

/// Outcome of `hydrate`: the conversation + its resume state.
#[derive(Debug, Clone)]
pub struct HydrateResult {
    pub conversation: Conversation,
    pub is_first_spawn: bool,
    pub is_resuming: bool,
}

/// Conversation store surface (ARCHITECTURE.md §7 service wiring).
pub trait ConversationStore: Send + Sync {
    /// Creates a conversation row. PTY conversations get
    /// `session_id = conversation.id` at creation (the reference's
    /// "set to conversation.id before first spawn"); ACP stays null until
    /// the session establishes a provider id. Emits `conversation:created`.
    fn create(&self, params: CreateConversationParams) -> Result<Conversation, Error>;

    fn get(&self, id: &str) -> Result<Option<Conversation>, Error>;
    fn list_by_task(&self, task_id: &str) -> Result<Vec<Conversation>, Error>;
    fn list_by_project(&self, project_id: &str) -> Result<Vec<Conversation>, Error>;

    /// Reference `deleteConversation`: deletes the row scoped to
    /// (project, task, conversation) + emits `conversation:deleted`. Session
    /// teardown (PTY stop / tmux kill) is E2-06's hook.
    fn delete(&self, project_id: &str, task_id: &str, conversation_id: &str) -> Result<(), Error>;

    /// Reference `renameConversation`: trimmed, clamped to 100 chars.
    fn rename(&self, id: &str, new_title: &str) -> Result<Conversation, Error>;

    /// Reference `touchConversation`.
    fn touch(&self, id: &str, last_interacted_at: &str) -> Result<(), Error>;

    /// Reference `setSessionId`: single guarded UPDATE — no read-then-write.
    /// Errors: `empty-session-id` (Error::EmptySessionId) and
    /// `conversation-not-found` (Error::ConversationNotFound).
    fn set_session_id(&self, id: &str, session_id: &str) -> Result<(), Error>;

    /// Reference `markConversationSeen`: `agent_status_seen = 1`.
    fn mark_seen(&self, id: &str) -> Result<(), Error>;

    /// Agent status tracking on the row (agent events surface in E2-06).
    fn update_agent_status(&self, id: &str, status: ConversationAgentStatus) -> Result<(), Error>;

    /// Reference `hydrateConversation`: restores a conversation after a
    /// restart with the correct resume state. For PTY: writes
    /// `session_id = conversation.id` on the FIRST spawn (idempotency guard
    /// — a second hydrate sees it set and reports `is_resuming = true`).
    fn hydrate(
        &self,
        project_id: &str,
        task_id: &str,
        conversation_id: &str,
    ) -> Result<HydrateResult, Error>;

    /// Reference `dehydrateConversation`: ACP → no-op; PTY → detach the
    /// session (E2-06 hook; Phase 0 validates + no-ops).
    fn dehydrate(
        &self,
        project_id: &str,
        task_id: &str,
        conversation_id: &str,
    ) -> Result<(), Error>;
}

/// SQLite-backed conversation store.
pub struct DbConversationStore {
    db: Arc<dyn Db>,
    event_bus: Arc<dyn crate::events::EventBus>,
}

impl DbConversationStore {
    pub fn new(db: Arc<dyn Db>, event_bus: Arc<dyn crate::events::EventBus>) -> Self {
        Self { db, event_bus }
    }

    fn with_conn<T>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
        f(&conn)
    }
}

impl ConversationStore for DbConversationStore {
    fn create(&self, params: CreateConversationParams) -> Result<Conversation, Error> {
        // E17-04 (#58): project scope is live (PM chat) — a project-scoped
        // conversation must have task_id NULL; task scope requires one.
        let scope = params.scope.unwrap_or(ConversationScope::Task);
        let task_id = match scope {
            ConversationScope::Task => Some(params.task_id.ok_or_else(|| {
                Error::InvalidTaskInput("task-scoped conversations require a task_id".into())
            })?),
            ConversationScope::Project => {
                if params.task_id.is_some() {
                    return Err(Error::InvalidTaskInput(
                        "project-scoped conversations must not carry a task_id".into(),
                    ));
                }
                None
            }
        };
        let id = params
            .id
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let title = params.title.trim().to_string();
        if title.is_empty() {
            return Err(Error::InvalidTaskInput(
                "conversation title is required".into(),
            ));
        }
        let conversation_type = params.r#type.unwrap_or(ConversationType::Pty);
        let config = ConversationConfig {
            r#type: conversation_type,
            auto_approve: params.auto_approve,
            initial_prompt: params.initial_prompt,
            model: params.model,
            initial_queue: params.initial_queue,
        };
        // PTY conversations carry session_id = conversation.id from creation
        // (reference createConversation); ACP stays null until the session
        // establishes a provider id.
        let session_id = match conversation_type {
            ConversationType::Pty => Some(id.clone()),
            ConversationType::Acp => None,
        };

        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO conversations
                     (id, project_id, task_id, scope, title, provider, config, session_id,
                      is_initial_conversation, type, created_at, updated_at, last_interacted_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'),
                         datetime('now'), datetime('now'))",
                rusqlite::params![
                    id,
                    params.project_id,
                    task_id,
                    scope.as_str(),
                    title,
                    params.provider,
                    config.to_json().to_string(),
                    session_id,
                    params.is_initial_conversation as i64,
                    conversation_type.as_str(),
                ],
            )?;
            Ok(())
        })?;

        let conversation = self
            .get(&id)?
            .ok_or_else(|| Error::Internal("created conversation not found".into()))?;
        self.event_bus
            .send(crate::events::InternalEvent::ConversationCreated {
                id: conversation.id.clone(),
                task_id: task_id.unwrap_or_default(),
                provider: conversation.provider.clone().unwrap_or_default(),
                title: conversation.title.clone(),
            });
        Ok(conversation)
    }

    fn get(&self, id: &str) -> Result<Option<Conversation>, Error> {
        self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    &format!("SELECT {CONVERSATION_COLUMNS} FROM conversations WHERE id = ?1"),
                    [id],
                    conversation_from_row,
                )
                .optional()?)
        })
    }

    fn list_by_task(&self, task_id: &str) -> Result<Vec<Conversation>, Error> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {CONVERSATION_COLUMNS} FROM conversations WHERE task_id = ?1 \
                 ORDER BY created_at ASC"
            ))?;
            let rows = stmt.query_map([task_id], conversation_from_row)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    fn list_by_project(&self, project_id: &str) -> Result<Vec<Conversation>, Error> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {CONVERSATION_COLUMNS} FROM conversations WHERE project_id = ?1 \
                 ORDER BY created_at ASC"
            ))?;
            let rows = stmt.query_map([project_id], conversation_from_row)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    fn delete(&self, project_id: &str, task_id: &str, conversation_id: &str) -> Result<(), Error> {
        let deleted = self.with_conn(|conn| {
            Ok(conn.execute(
                "DELETE FROM conversations WHERE id = ?1 AND project_id = ?2 AND task_id = ?3",
                rusqlite::params![conversation_id, project_id, task_id],
            )?)
        })?;
        // Reference deleteConversation: fire-and-forget — a raced delete is
        // a no-op, not an error; the event fires only when a row went away.
        if deleted > 0 {
            // Session teardown (PTY stop / tmux kill) is E2-06's hook.
            self.event_bus
                .send(crate::events::InternalEvent::ConversationDeleted {
                    id: conversation_id.into(),
                });
        }
        Ok(())
    }

    fn rename(&self, id: &str, new_title: &str) -> Result<Conversation, Error> {
        let title = new_title.trim();
        if title.is_empty() {
            return Err(Error::InvalidTaskInput(
                "conversation title is required".into(),
            ));
        }
        // Character-clamped, not byte-sliced — a byte slice can split a
        // multi-byte UTF-8 char and panic.
        let title: String = title.chars().take(MAX_CONVERSATION_TITLE_LENGTH).collect();
        let updated = self.with_conn(|conn| {
            Ok(conn.execute(
                "UPDATE conversations SET title = ?1, updated_at = datetime('now') WHERE id = ?2",
                rusqlite::params![title, id],
            )?)
        })?;
        if updated == 0 {
            return Err(Error::ConversationNotFound(id.into()));
        }
        self.event_bus
            .send(crate::events::InternalEvent::ConversationRenamed {
                id: id.to_string(),
                title: title.clone(),
            });
        self.get(id)?
            .ok_or_else(|| Error::ConversationNotFound(id.into()))
    }

    fn touch(&self, id: &str, last_interacted_at: &str) -> Result<(), Error> {
        // Reference touchConversation: fire-and-forget no-op when gone.
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE conversations SET last_interacted_at = ?1, updated_at = datetime('now') \
                 WHERE id = ?2",
                rusqlite::params![last_interacted_at, id],
            )?;
            Ok(())
        })
    }

    fn set_session_id(&self, id: &str, session_id: &str) -> Result<(), Error> {
        let trimmed = session_id.trim();
        if trimmed.is_empty() {
            return Err(Error::EmptySessionId);
        }
        let updated = self.with_conn(|conn| {
            Ok(conn.execute(
                "UPDATE conversations SET session_id = ?1, updated_at = datetime('now') \
                 WHERE id = ?2",
                rusqlite::params![trimmed, id],
            )?)
        })?;
        if updated == 0 {
            return Err(Error::ConversationNotFound(id.into()));
        }
        Ok(())
    }

    fn mark_seen(&self, id: &str) -> Result<(), Error> {
        // Reference markConversationSeen: fire-and-forget no-op when the
        // conversation is gone.
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE conversations SET agent_status_seen = 1 WHERE id = ?1",
                [id],
            )?;
            Ok(())
        })
    }

    fn update_agent_status(&self, id: &str, status: ConversationAgentStatus) -> Result<(), Error> {
        let updated = self.with_conn(|conn| {
            Ok(conn.execute(
                "UPDATE conversations SET agent_status = ?1, agent_status_seen = 0, \
                 updated_at = datetime('now') WHERE id = ?2",
                rusqlite::params![status.as_str(), id],
            )?)
        })?;
        if updated == 0 {
            return Err(Error::ConversationNotFound(id.into()));
        }
        Ok(())
    }

    fn hydrate(
        &self,
        project_id: &str,
        task_id: &str,
        conversation_id: &str,
    ) -> Result<HydrateResult, Error> {
        let conversation = self
            .get(conversation_id)?
            .ok_or_else(|| Error::ConversationNotFound(conversation_id.into()))?;
        if conversation.project_id != project_id || conversation.task_id.as_deref() != Some(task_id)
        {
            return Err(Error::ConversationNotFound(conversation_id.into()));
        }

        let (is_first_spawn, is_resuming) = match conversation.r#type {
            ConversationType::Acp => {
                // ACP sessions start lazily; resume state comes from the
                // provider session id (populated by the ACP client, Phase 2).
                (
                    conversation.session_id.is_none(),
                    conversation.session_id.is_some(),
                )
            }
            ConversationType::Pty => {
                // Atomic first-spawn guard (reference hydrateConversation):
                // only a NULL session_id gets the conversation id written —
                // the affected-row count decides who won the race, no
                // read-then-write, so a concurrent double-hydrate can't
                // double-fire.
                let claimed = self.with_conn(|conn| {
                    Ok(conn.execute(
                        "UPDATE conversations SET session_id = ?1, updated_at = datetime('now') \
                         WHERE id = ?2 AND session_id IS NULL",
                        rusqlite::params![conversation_id, conversation_id],
                    )?)
                })?;
                (claimed == 1, claimed != 1)
            }
        };
        let conversation = self
            .get(conversation_id)?
            .ok_or_else(|| Error::ConversationNotFound(conversation_id.into()))?;
        Ok(HydrateResult {
            conversation,
            is_first_spawn,
            is_resuming,
        })
    }

    fn dehydrate(
        &self,
        project_id: &str,
        task_id: &str,
        conversation_id: &str,
    ) -> Result<(), Error> {
        let conversation = self
            .get(conversation_id)?
            .ok_or_else(|| Error::ConversationNotFound(conversation_id.into()))?;
        if conversation.project_id != project_id || conversation.task_id.as_deref() != Some(task_id)
        {
            return Err(Error::ConversationNotFound(conversation_id.into()));
        }
        if conversation.r#type == ConversationType::Acp {
            return Ok(()); // ACP has no active PTY session to detach.
        }
        // PTY session detach (task.conversations.detachSession) is E2-06's
        // hook — Phase 0 validates ownership and returns.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pty_session_id_round_trips() {
        let id = make_pty_session_id("proj-1", "task-2", "conv-3");
        assert_eq!(id, "proj-1:task-2:conv-3");
        assert_eq!(
            parse_pty_session_id(&id),
            Some(("proj-1".into(), "task-2".into(), "conv-3".into()))
        );
        assert_eq!(parse_pty_session_id("a:b"), None);
        assert_eq!(parse_pty_session_id("a:b:c:d"), None);
        assert_eq!(parse_pty_session_id("a::c"), None);
    }

    #[test]
    fn resume_set_contains_exactly_the_seven_providers() {
        assert_eq!(
            PROVIDER_SESSION_ID_REQUIRED_FOR_RESUME,
            [
                "amp",
                "codex",
                "commandcode",
                "droid",
                "goose",
                "oh-my-pi",
                "pi"
            ]
        );
    }

    #[test]
    fn resume_native_session_id_wins_when_set_and_different() {
        // codex (in the 7-set) resuming with a native id → native + resuming.
        let (sid, resuming) =
            resolve_agent_session_command_args("codex", "conv-1", Some("native-abc"), true, None);
        assert_eq!((sid.as_str(), resuming), ("native-abc", true));
    }

    #[test]
    fn resume_falls_back_when_native_is_conversation_id_or_missing() {
        // Native id == conversation id → fresh (isResuming cleared).
        let (sid, resuming) =
            resolve_agent_session_command_args("codex", "conv-1", Some("conv-1"), true, None);
        assert_eq!((sid.as_str(), resuming), ("conv-1", false));

        // No native id → fresh.
        let (sid, resuming) =
            resolve_agent_session_command_args("codex", "conv-1", None, true, None);
        assert_eq!((sid.as_str(), resuming), ("conv-1", false));
    }

    #[test]
    fn resume_require_provider_session_id_false_keeps_is_resuming() {
        // The native id wins when usable (reference checks it first); the
        // requireProviderSessionId=false escape applies only when the native
        // id is missing or equals the conversation id.
        let (sid, resuming) = resolve_agent_session_command_args(
            "codex",
            "conv-1",
            Some("native-abc"),
            true,
            Some(false),
        );
        assert_eq!((sid.as_str(), resuming), ("native-abc", true));

        let (sid, resuming) =
            resolve_agent_session_command_args("codex", "conv-1", None, true, Some(false));
        assert_eq!((sid.as_str(), resuming), ("conv-1", true));

        let (sid, resuming) = resolve_agent_session_command_args(
            "codex",
            "conv-1",
            Some("conv-1"),
            true,
            Some(false),
        );
        assert_eq!((sid.as_str(), resuming), ("conv-1", true));
    }

    #[test]
    fn resume_passthrough_for_non_seven_providers() {
        // claude is not in the set: session id untouched, resume flag kept.
        let (sid, resuming) =
            resolve_agent_session_command_args("claude", "conv-1", Some("native-abc"), true, None);
        assert_eq!((sid.as_str(), resuming), ("conv-1", true));

        // pi not resuming → passthrough.
        let (sid, resuming) =
            resolve_agent_session_command_args("pi", "conv-1", Some("native-abc"), false, None);
        assert_eq!((sid.as_str(), resuming), ("conv-1", false));
    }

    #[test]
    fn config_json_is_delta_shaped_and_round_trips() {
        let cfg = ConversationConfig {
            r#type: ConversationType::Pty,
            auto_approve: Some(true),
            initial_prompt: Some("do it".into()),
            model: Some("sonnet".into()),
            initial_queue: None,
        };
        let json = cfg.to_json();
        assert_eq!(json["version"], "1");
        assert_eq!(json["type"], "pty");
        assert_eq!(json["autoApprove"], true);
        assert!(json.get("initialQueue").is_none());

        let parsed = ConversationConfig::from_json(Some(&json));
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn config_from_json_never_panics() {
        let parsed = ConversationConfig::from_json(None);
        assert_eq!(parsed, ConversationConfig::default());
        let parsed = ConversationConfig::from_json(Some(&serde_json::json!({"bogus": 1})));
        assert_eq!(parsed, ConversationConfig::default());
        let parsed = ConversationConfig::from_json(Some(&serde_json::json!({"type": "acp"})));
        assert_eq!(parsed.r#type, ConversationType::Acp);
    }
}
