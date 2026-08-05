//! ACP runtime service for the Tauri shell (ticket E2-11-5).
//!
//! Wires the E2-11 stack together inside the app process (ADR-0030):
//!
//! - [`AcpRuntime`] owns the [`SessionManager`] and one `AcpClient` per
//!   running conversation. `start` resolves the conversation's worktree,
//!   the adapter binary, and the server-side provider env (keyring —
//!   the renderer never supplies env), then spawns the adapter as a child
//!   process and hands the client to the manager.
//! - [`DbSessionIdStore`] implements the manager's persistence seam over
//!   `ade_core::conversations::ConversationStore` (ADR-0027).
//! - The provider decision hook (`resolve_session_path`) gates `start`:
//!   only ACP-type conversations with an ACP-capable provider run here;
//!   everything else stays on the TUI/PTY path, byte-identical.
//!
//! Adapter binary resolution is injected ([`AdapterResolver`]) because the
//! per-provider adapter descriptors arrive with E3; the default looks for
//! `<provider-id>-acp` on PATH.
//!
//! Errors speak `ade_acp::Error` (the ACP layer's type); ade-core errors
//! are mapped through [`core_err`] so the two sibling crates never share a
//! dependency edge.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ade_acp::session::{SessionEvents, SessionIdStore, SessionManager, StartInput};
use ade_acp::{AcpClient, Error, QueuedPrompt};
use ade_core::conversations::{resolve_session_path, ConversationStore, SessionPath};
use ade_core::db::Db;
use ade_core::provider_accounts::ProviderAccountStore;
use ade_core::pty::launcher::find_on_path;
use ade_core::tasks::TaskStore;
use agent_client_protocol_schema::v1::SessionId;
use parking_lot::Mutex;
use rusqlite::OptionalExtension;
use serde::Serialize;

/// Resolves a provider id to its ACP adapter binary. Injected so tests can
/// point at the fake adapter and E3 can plug in real descriptors later.
pub type AdapterResolver = Arc<dyn Fn(&str) -> Result<PathBuf, Error> + Send + Sync>;

/// Real-world adapter binary + install package per provider (the npm
/// binary names do NOT follow a `<id>-acp` pattern — claude's adapter is
/// `claude-agent-acp` from `@agentclientprotocol/claude-agent-acp`).
/// Long-term home is the provider descriptor's adapter metadata (E3 plugin
/// machinery, Phase 2); until then this table is the single source. The
/// `<id>-acp` fallback is kept for providers whose adapters do follow it.
fn adapter_binary(provider_id: &str) -> (String, Option<&'static str>) {
    match provider_id {
        "claude" => (
            "claude-agent-acp".to_string(),
            Some("@agentclientprotocol/claude-agent-acp"),
        ),
        _ => (format!("{provider_id}-acp"), None),
    }
}

/// Default resolver: real adapter binary name on PATH, with an actionable
/// install hint when the package is known.
pub fn default_adapter_resolver(provider_id: &str) -> Result<PathBuf, Error> {
    let (name, package) = adapter_binary(provider_id);
    find_on_path(&name).ok_or_else(|| {
        let hint = package
            .map(|p| format!(" — install with: npm i -g {p}"))
            .unwrap_or_default();
        Error::InvalidState(format!(
            "ACP adapter binary not found on PATH: {name}{hint}"
        ))
    })
}

/// ade-core error → ade-acp error (the runtime's native error type).
fn core_err(e: ade_core::Error) -> Error {
    match e {
        ade_core::Error::ConversationNotFound(id) => Error::ConversationNotFound(id),
        other => Error::InvalidState(other.to_string()),
    }
}

/// Persistence seam: manager → `conversations.session_id` (ADR-0027).
/// Errors are logged by the manager, never fatal.
pub struct DbSessionIdStore {
    store: Arc<dyn ConversationStore>,
}

impl DbSessionIdStore {
    pub fn new(store: Arc<dyn ConversationStore>) -> Self {
        Self { store }
    }
}

impl SessionIdStore for DbSessionIdStore {
    fn set_session_id(&self, conversation_id: &str, session_id: &str) -> Result<(), String> {
        self.store
            .set_session_id(conversation_id, session_id)
            .map_err(|e| e.to_string())
    }
}

/// DTO for `acp_start` results.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpStartOutcome {
    pub session_id: String,
    pub resumed: bool,
}

/// DTO for `acp_send_prompt` results (reference `sendPromptResponse`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpSendPromptResponse {
    pub queued: bool,
}

/// Owns every running ACP conversation in the app process.
pub struct AcpRuntime {
    manager: Arc<SessionManager>,
    conversations: Arc<dyn ConversationStore>,
    tasks: Arc<dyn TaskStore>,
    db: Arc<dyn Db>,
    provider_accounts: Arc<ProviderAccountStore>,
    events: Arc<dyn SessionEvents>,
    adapter_resolver: AdapterResolver,
    /// Clients stay alive for the manager's shutdown path (stop() drops
    /// the cell record and shuts the adapter down via its client).
    clients: Mutex<HashMap<String, Arc<AcpClient>>>,
}

impl AcpRuntime {
    pub fn new(
        conversations: Arc<dyn ConversationStore>,
        tasks: Arc<dyn TaskStore>,
        db: Arc<dyn Db>,
        provider_accounts: Arc<ProviderAccountStore>,
        events: Arc<dyn SessionEvents>,
        adapter_resolver: AdapterResolver,
    ) -> Arc<Self> {
        let store = Arc::new(DbSessionIdStore::new(conversations.clone()));
        Arc::new(Self {
            manager: Arc::new(SessionManager::new(Some(store))),
            conversations,
            tasks,
            db,
            provider_accounts,
            events,
            adapter_resolver,
            clients: Mutex::new(HashMap::new()),
        })
    }

    /// Starts (or resumes) the ACP session for a conversation.
    ///
    /// Reference `startSession`: idempotent — a running conversation
    /// returns its existing session id. The provider decision hook must
    /// resolve to [`SessionPath::Acp`]; anything else is rejected so the
    /// TUI path stays untouched.
    pub async fn start(&self, conversation_id: &str) -> Result<AcpStartOutcome, Error> {
        if self.manager.is_running(conversation_id) {
            // Reference `startSession` idempotency: a running conversation
            // returns its existing (persisted) session id.
            let session_id = self
                .conversations
                .get(conversation_id)
                .map_err(core_err)?
                .and_then(|c| c.session_id)
                .unwrap_or_default();
            return Ok(AcpStartOutcome {
                session_id,
                resumed: false,
            });
        }

        let conv = self
            .conversations
            .get(conversation_id)
            .map_err(core_err)?
            .ok_or_else(|| Error::ConversationNotFound(conversation_id.to_string()))?;
        if resolve_session_path(&conv) != SessionPath::Acp {
            return Err(Error::InvalidState(format!(
                "conversation {conversation_id} is not on the ACP path"
            )));
        }
        let provider_id = conv
            .provider
            .clone()
            .ok_or_else(|| Error::InvalidState("conversation has no provider".into()))?;

        let cwd = self.resolve_cwd(conversation_id)?;
        let adapter = (self.adapter_resolver)(&provider_id)?;
        let env = self.provider_env(&provider_id);
        // Reference behavior (plugins claude/index.ts): point the adapter's
        // Claude Agent SDK at the host-installed claude binary instead of
        // the SDK's auto-downloaded native binary.
        let mut env = env;
        if provider_id == "claude" && !env.iter().any(|(k, _)| k == "CLAUDE_CODE_EXECUTABLE") {
            if let Some(cli) = find_on_path("claude") {
                env.push((
                    "CLAUDE_CODE_EXECUTABLE".into(),
                    cli.to_string_lossy().into_owned(),
                ));
            }
        }

        let manager = Arc::clone(&self.manager);
        let resolver = SessionManager::permission_resolver(
            Arc::downgrade(&manager),
            conversation_id.to_string(),
        );
        let client = Arc::new(
            AcpClient::spawn(&adapter, &[], &env, Some(&cwd), Some(cwd.clone()), resolver).await?,
        );
        let init = client.initialize().await?;
        let supports_load_session = init.response.agent_capabilities.load_session;
        let loop_client = Arc::clone(&client);
        tokio::spawn(async move { loop_client.run_event_loop().await });
        self.clients
            .lock()
            .insert(conversation_id.to_string(), Arc::clone(&client));

        let initial_queue: Vec<QueuedPrompt> = conv
            .config
            .initial_queue
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|p| QueuedPrompt {
                id: uuid::Uuid::new_v4().to_string(),
                text: p.text,
                hidden_context: p.hidden_context,
            })
            .collect();

        let outcome = self
            .manager
            .start(StartInput {
                conversation_id: conversation_id.to_string(),
                provider_id,
                cwd,
                session_id: conv.session_id.as_deref().map(SessionId::new),
                supports_load_session,
                initial_queue,
                events: Some(self.events.clone()),
                client,
            })
            .await?;

        Ok(AcpStartOutcome {
            session_id: outcome.session_id.0.to_string(),
            resumed: outcome.resumed,
        })
    }

    /// Sends a prompt now when idle, else queues it.
    pub async fn send_prompt(
        &self,
        conversation_id: &str,
        text: &str,
        hidden_context: Option<&str>,
    ) -> Result<AcpSendPromptResponse, Error> {
        let queued = self
            .manager
            .prompt(conversation_id, text, hidden_context)
            .await?;
        Ok(AcpSendPromptResponse { queued })
    }

    /// Cancels the in-flight turn (no-op when idle).
    pub async fn cancel(&self, conversation_id: &str) -> Result<(), Error> {
        self.manager.cancel(conversation_id).await
    }

    /// Answers a surfaced permission request.
    pub fn resolve_permission(
        &self,
        conversation_id: &str,
        request_id: &str,
        option_id: &str,
    ) -> Result<(), Error> {
        self.manager
            .resolve_permission(conversation_id, request_id, option_id)
    }

    /// Stops one conversation's session (silent when not running).
    pub fn stop(&self, conversation_id: &str) -> Result<(), Error> {
        self.clients.lock().remove(conversation_id);
        self.manager.stop(conversation_id)
    }

    /// Task-deletion teardown: stop every ACP session of the task
    /// (reference `stopSession` per conversation; best-effort like the
    /// PTY reap in E2-09).
    pub fn stop_task(&self, task_id: &str) {
        let Ok(conversations) = self.conversations.list_by_task(task_id) else {
            return;
        };
        for conv in conversations {
            if let Err(e) = self.stop(&conv.id) {
                tracing::warn!(conversation = %conv.id, error = %e, "ACP stop failed during task teardown");
            }
        }
    }

    pub fn is_running(&self, conversation_id: &str) -> bool {
        self.manager.is_running(conversation_id)
    }

    /// The full live-model snapshot (history + config + usage + …).
    pub fn live_models(&self, conversation_id: &str) -> Option<ade_acp::LiveModels> {
        self.manager.live_models(conversation_id)
    }

    // -- internals ---------------------------------------------------------

    /// The adapter session cwd: the task's worktree path when one exists,
    /// else the project root (chat works anywhere; mirrors the rehydrate
    /// path lookup).
    fn resolve_cwd(&self, conversation_id: &str) -> Result<PathBuf, Error> {
        let conv = self
            .conversations
            .get(conversation_id)
            .map_err(core_err)?
            .ok_or_else(|| Error::ConversationNotFound(conversation_id.to_string()))?;
        let Some(task_id) = &conv.task_id else {
            return Err(Error::InvalidState(
                "project-scoped conversations have no workspace yet".into(),
            ));
        };
        let task = self
            .tasks
            .get(task_id)
            .map_err(core_err)?
            .ok_or_else(|| Error::InvalidState(format!("task {task_id} not found")))?;
        if let Some(workspace_id) = &task.workspace_id {
            let path: Option<String> = self
                .db
                .conn()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .query_row(
                    "SELECT path FROM workspaces WHERE id = ?1",
                    [workspace_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| Error::InvalidState(e.to_string()))?;
            if let Some(path) = path {
                if Path::new(&path).is_dir() {
                    return Ok(PathBuf::from(path));
                }
                tracing::warn!(workspace = %workspace_id, "worktree missing on disk; falling back to project root");
            }
        }
        let project_path: String = self
            .db
            .conn()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row(
                "SELECT path FROM projects WHERE id = ?1",
                [&task.project_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| Error::InvalidState(e.to_string()))?
            .ok_or_else(|| Error::InvalidState(format!("project {} not found", task.project_id)))?;
        Ok(PathBuf::from(project_path))
    }

    /// Server-side provider env: keyring credentials for the default
    /// account (E3-07); when the provider has no account, fall back to the
    /// descriptor's env vars from the process env (the TUI launcher
    /// convention) — a missing var is skipped, never invented. The renderer
    /// contributes nothing.
    fn provider_env(&self, provider_id: &str) -> Vec<(String, String)> {
        if let Ok(env) = self.provider_accounts.resolve_env(provider_id) {
            if !env.is_empty() {
                return env;
            }
        }
        let Some(provider) = ade_providers::get(provider_id) else {
            return Vec::new();
        };
        provider
            .env_vars
            .iter()
            .filter_map(|key| std::env::var(key).ok().map(|value| (key.clone(), value)))
            .collect()
    }
}
