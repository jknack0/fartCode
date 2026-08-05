//! Session manager — cross-session lifecycle for ACP conversations
//! (ticket E2-11-3).
//!
//! Ports the reference `packages/runtime/src/acp-agents/runtime/
//! session-manager.ts`, scoped to what Phase 2 consumes before the live
//! models land (E2-11-4) and the Tauri wiring (#32):
//!
//! - Cells keyed by conversation id, routes keyed by ACP session id.
//! - `start` = `session/load` (resume) when a persisted session id exists
//!   and the agent supports it, falling back to `session/new` — with the
//!   session id persisted through [`SessionIdStore`] on both paths so a
//!   restart resumes the same session.
//! - Permission resolver that routes surfaced requests into the cell.
//!
//! Deliberate deviations from the reference: no connection pool (one
//! `AcpClient` per conversation, handed in by the caller — the process-host
//! seam from E2-11-2), no live-model hosts / session summaries (E2-11-4),
//! no agent terminals (Phase 4).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak};

use agent_client_protocol_schema::v1::{SessionId, SessionUpdate};
use parking_lot::Mutex;

use crate::client::AcpClient;
use crate::error::Error;
use crate::handlers::{PermissionAnswerer, PermissionRequest};
use crate::session::cell::{
    Callbacks, ChatHistory, HistoryPage, Lifecycle, QueuedPrompt, SessionCell, SessionState,
    UpdateSink,
};

/// Persistence seam for ACP session ids (`conversations.session_id`).
///
/// Implemented over `ade_core::conversations::ConversationStore` at the app
/// layer (#32); kept trait-shaped here so `ade-acp` stays free of the
/// ade-core dependency (ADR-0027).
pub trait SessionIdStore: Send + Sync {
    /// Persists the provider session id for a conversation. Errors are
    /// logged by the manager, never fatal (launcher precedent, E2-06).
    fn set_session_id(&self, conversation_id: &str, session_id: &str) -> Result<(), String>;
}

/// Input for [`SessionManager::start`].
#[derive(Clone)]
pub struct StartInput {
    /// Conversation this session belongs to.
    pub conversation_id: String,
    /// Workspace root the adapter operates in (fs/* scoping + session cwd).
    pub cwd: PathBuf,
    /// Persisted provider session id to resume (`session/load`), if any.
    pub session_id: Option<SessionId>,
    /// The agent supports `session/load` (from `initialize` capabilities).
    pub supports_load_session: bool,
    /// Prompts queued for delivery once the session is ready, in order.
    pub initial_queue: Vec<QueuedPrompt>,
    /// Live update feed (raw for now; the reducer subscribes in E2-11-4).
    pub update_sink: Option<UpdateSink>,
    /// The client driving this session (spawned by the caller — test rig or
    /// the E2-11-2 process host).
    pub client: Arc<AcpClient>,
}

/// Result of [`SessionManager::start`].
#[derive(Debug, Clone)]
pub struct StartOutcome {
    /// The provider session id (fresh or resumed).
    pub session_id: SessionId,
    /// `true` when the session was resumed via `session/load`.
    pub resumed: bool,
}

struct Record {
    cell: Arc<SessionCell>,
}

/// Owns every running ACP conversation, keyed by conversation id
/// (reference `SessionManager`).
pub struct SessionManager {
    cells: Mutex<HashMap<String, Record>>,
    /// acp session id → conversation id (reference `routes`, minus the
    /// process-key level — one client per conversation here).
    routes: Mutex<HashMap<String, String>>,
    store: Option<Arc<dyn SessionIdStore>>,
}

impl SessionManager {
    pub fn new(store: Option<Arc<dyn SessionIdStore>>) -> Self {
        Self {
            cells: Mutex::new(HashMap::new()),
            routes: Mutex::new(HashMap::new()),
            store,
        }
    }

    // -- lifecycle ---------------------------------------------------------

    /// Starts (or resumes) the session for `input.conversation_id`.
    ///
    /// Idempotent: a second start for a running conversation returns the
    /// existing session id (reference behavior). Resume path:
    /// `session/load` with replay capture; a failed load falls back to
    /// `session/new` (reference: "loadSession failed, starting a new
    /// session"). The session id is persisted on both paths.
    pub async fn start(self: &Arc<Self>, input: StartInput) -> Result<StartOutcome, Error> {
        if let Some(record) = self.cells.lock().get(&input.conversation_id) {
            return Ok(StartOutcome {
                session_id: record.cell.acp_session_id().clone(),
                resumed: false,
            });
        }

        // Resume path.
        if let Some(session_id) = &input.session_id {
            if input.supports_load_session {
                match self.start_loaded(&input, session_id).await {
                    Ok(outcome) => return Ok(outcome),
                    Err(error) => {
                        tracing::warn!(
                            conversation = %input.conversation_id,
                            error = %error,
                            "SessionManager: load_session failed, starting a new session"
                        );
                    }
                }
            }
        }

        // Fresh path.
        let session_id = input.client.new_session(&input.cwd).await?;
        let cell = self.register_cell(&input, session_id.clone());
        cell.mark_ready();
        self.persist_session_id(&input.conversation_id, &session_id);
        self.dispatch_initial_queue(&cell, input.initial_queue)
            .await;
        Ok(StartOutcome {
            session_id,
            resumed: false,
        })
    }

    /// Resume path: register the cell in replay mode, run `session/load`,
    /// capture the replayed history, then hand the initial queue over.
    async fn start_loaded(
        self: &Arc<Self>,
        input: &StartInput,
        session_id: &SessionId,
    ) -> Result<StartOutcome, Error> {
        let cell = self.register_cell(input, session_id.clone());
        cell.begin_replay();
        match input.client.load_session(session_id, &input.cwd).await {
            Ok((_response, updates)) => {
                for update in updates {
                    cell.record_update(update);
                }
                cell.end_replay();
                self.persist_session_id(&input.conversation_id, session_id);
                self.dispatch_initial_queue(&cell, input.initial_queue.clone())
                    .await;
                Ok(StartOutcome {
                    session_id: session_id.clone(),
                    resumed: true,
                })
            }
            Err(error) => {
                self.remove_record(&input.conversation_id);
                Err(error)
            }
        }
    }

    /// Stops a conversation's session: settles pending permissions, drops
    /// the record, and shuts the adapter down in the background
    /// (reference `stop` — missing conversations are a silent ok).
    pub fn stop(&self, conversation_id: &str) -> Result<(), Error> {
        let Some(record) = self.cells.lock().remove(conversation_id) else {
            return Ok(());
        };
        self.unregister_route(record.cell.acp_session_id());
        record.cell.dispose();
        let client = Arc::clone(record.cell.client());
        tokio::spawn(async move {
            client.shutdown().await;
        });
        Ok(())
    }

    // -- prompts -----------------------------------------------------------

    /// Sends a prompt now when idle, else queues it (reference `prompt`).
    pub async fn prompt(
        &self,
        conversation_id: &str,
        text: &str,
        hidden_context: Option<&str>,
    ) -> Result<bool, Error> {
        let cell = self.cell(conversation_id)?;
        cell.prompt(text, hidden_context).await
    }

    /// Appends to the queue (reference `queuePrompt`).
    pub fn queue_prompt(
        &self,
        conversation_id: &str,
        text: &str,
        hidden_context: Option<&str>,
    ) -> Result<QueuedPrompt, Error> {
        let cell = self.cell(conversation_id)?;
        cell.queue_prompt(text, hidden_context)
    }

    /// Replaces a queued prompt's text (reference `editQueuedPrompt`).
    pub fn edit_queued_prompt(
        &self,
        conversation_id: &str,
        id: &str,
        text: &str,
        hidden_context: Option<&str>,
    ) -> Result<(), Error> {
        let cell = self.cell(conversation_id)?;
        cell.edit_queued_prompt(id, text, hidden_context)
    }

    /// Removes a queued prompt (reference `removeQueuedPrompt`).
    pub fn remove_queued_prompt(&self, conversation_id: &str, id: &str) -> Result<(), Error> {
        let cell = self.cell(conversation_id)?;
        cell.remove_queued_prompt(id)
    }

    /// Reorders the queue (reference `reorderQueue`).
    pub fn reorder_queue(&self, conversation_id: &str, ids: &[String]) -> Result<(), Error> {
        let cell = self.cell(conversation_id)?;
        cell.reorder_queue(ids)
    }

    /// Cancels the in-flight turn; no-op when nothing is running
    /// (reference `cancel`).
    pub async fn cancel(&self, conversation_id: &str) -> Result<(), Error> {
        let Some(cell) = self.cell_opt(conversation_id) else {
            return Ok(());
        };
        cell.cancel().await
    }

    // -- permissions / modes / config ---------------------------------------

    /// Answers a surfaced permission request (reference `resolvePermission`).
    pub fn resolve_permission(
        &self,
        conversation_id: &str,
        request_id: &str,
        option_id: &str,
    ) -> Result<(), Error> {
        let cell = self.cell(conversation_id)?;
        cell.resolve_permission(request_id, option_id)
    }

    /// Switches the session mode (reference `setMode`).
    pub async fn set_mode(&self, conversation_id: &str, mode_id: &str) -> Result<(), Error> {
        let cell = self.cell(conversation_id)?;
        cell.set_mode(mode_id).await
    }

    /// Sets a session config option (reference `setConfigOption`).
    pub async fn set_config_option(
        &self,
        conversation_id: &str,
        config_id: &str,
        value: agent_client_protocol_schema::v1::SessionConfigOptionValue,
    ) -> Result<(), Error> {
        let cell = self.cell(conversation_id)?;
        cell.set_config_option(config_id, value).await
    }

    /// Rev-guarded composer draft (reference `setPromptDraft`).
    pub fn set_prompt_draft(
        &self,
        conversation_id: &str,
        rev: u64,
        text: Option<&str>,
    ) -> Result<(), Error> {
        let cell = self.cell(conversation_id)?;
        cell.set_prompt_draft(rev, text)
    }

    // -- observation -------------------------------------------------------

    pub fn is_running(&self, conversation_id: &str) -> bool {
        self.cells.lock().contains_key(conversation_id)
    }

    pub fn get_chat_history(&self, conversation_id: &str) -> ChatHistory {
        match self.cell_opt(conversation_id) {
            Some(cell) => cell.history(),
            None => ChatHistory {
                committed: Vec::new(),
                active: None,
                replay: Vec::new(),
            },
        }
    }

    /// Committed history page, newest-last (reference `getHistory`; callers
    /// default `limit` to 50).
    pub fn get_history(
        &self,
        conversation_id: &str,
        before: Option<u64>,
        limit: usize,
    ) -> HistoryPage {
        match self.cell_opt(conversation_id) {
            Some(cell) => cell.history_page(before, limit),
            None => HistoryPage {
                turns: Vec::new(),
                next_cursor: None,
            },
        }
    }

    /// The prompt-composer draft (reference draft sync surface).
    pub fn get_prompt_draft(
        &self,
        conversation_id: &str,
    ) -> Option<crate::session::cell::PromptDraft> {
        self.cell_opt(conversation_id)?.prompt_draft()
    }

    pub fn get_session_state(&self, conversation_id: &str) -> SessionState {
        match self.cell_opt(conversation_id) {
            Some(cell) => cell.session_state(),
            None => SessionState {
                lifecycle: Lifecycle::Closed,
                active_turn_id: None,
                pending_permissions: Vec::new(),
                last_stop_reason: None,
                queued_prompts: Vec::new(),
                is_generating: false,
                can_submit: false,
                can_cancel: false,
            },
        }
    }

    // -- routing -----------------------------------------------------------

    /// Routes a host-delivered `session/update` to its conversation
    /// (reference `onSessionUpdate`, minus the raw-log/live-model steps).
    /// Unknown sessions are warned and dropped.
    pub fn route_update(&self, session_id: &SessionId, update: SessionUpdate) {
        let Some(conversation_id) = self.routes.lock().get(session_id.0.as_ref()).cloned() else {
            tracing::warn!(session = %session_id, "SessionManager: update for unknown session");
            return;
        };
        let Some(cell) = self.cell_opt(&conversation_id) else {
            return;
        };
        if cell.acp_session_id() != session_id {
            tracing::warn!(
                conversation = %conversation_id,
                "SessionManager: session id drift; dropping update"
            );
            return;
        }
        cell.record_update(crate::client::SessionUpdateEvent {
            session_id: session_id.clone(),
            update,
        });
    }

    /// Permission resolver for `AcpClient::spawn` / `from_transport`:
    /// routes surfaced requests into the conversation's cell (answering
    /// `cancelled` when the conversation is already gone).
    pub fn permission_resolver(
        manager: Weak<SessionManager>,
        conversation_id: String,
    ) -> impl Fn(PermissionRequest, Arc<PermissionAnswerer>) + Send + Sync + 'static {
        move |request, answerer| {
            let cell = manager.upgrade().and_then(|m| m.cell_opt(&conversation_id));
            match cell {
                Some(cell) => cell.permission_requested(request, answerer),
                None => {
                    tokio::spawn(async move {
                        let _ = answerer
                            .answer(crate::handlers::PermissionDecision::Cancelled)
                            .await;
                    });
                }
            }
        }
    }

    // -- internals ---------------------------------------------------------

    fn register_cell(&self, input: &StartInput, session_id: SessionId) -> Arc<SessionCell> {
        let cell = Arc::new(SessionCell::new(
            input.conversation_id.clone(),
            session_id.clone(),
            Arc::clone(&input.client),
            Callbacks::default(),
            input.update_sink.clone(),
        ));
        self.cells.lock().insert(
            input.conversation_id.clone(),
            Record {
                cell: Arc::clone(&cell),
            },
        );
        self.routes.lock().insert(
            session_id.0.as_ref().to_string(),
            input.conversation_id.clone(),
        );
        cell
    }

    fn remove_record(&self, conversation_id: &str) {
        if let Some(record) = self.cells.lock().remove(conversation_id) {
            self.unregister_route(record.cell.acp_session_id());
        }
    }

    fn unregister_route(&self, session_id: &SessionId) {
        self.routes.lock().remove(session_id.0.as_ref());
    }

    fn persist_session_id(&self, conversation_id: &str, session_id: &SessionId) {
        let Some(store) = &self.store else {
            return;
        };
        if let Err(error) = store.set_session_id(conversation_id, session_id.0.as_ref()) {
            tracing::warn!(
                conversation = %conversation_id,
                error = %error,
                "SessionManager: failed to persist session id"
            );
        }
    }

    /// Queues the initial prompts and dispatches the head once the cell is
    /// ready; the cell's turn loop drains the rest in order (reference
    /// `queueInitialPrompts` + the `SessionReady` dequeue effect).
    async fn dispatch_initial_queue(&self, cell: &Arc<SessionCell>, queue: Vec<QueuedPrompt>) {
        for prompt in queue {
            if let Err(error) = cell.queue_prompt(&prompt.text, prompt.hidden_context.as_deref()) {
                tracing::warn!(
                    conversation = %cell.conversation_id(),
                    error = %error,
                    "SessionManager: failed to queue initial prompt"
                );
            }
        }
        cell.drain_queue();
    }

    fn cell(&self, conversation_id: &str) -> Result<Arc<SessionCell>, Error> {
        self.cell_opt(conversation_id)
            .ok_or_else(|| Error::ConversationNotFound(conversation_id.to_string()))
    }

    fn cell_opt(&self, conversation_id: &str) -> Option<Arc<SessionCell>> {
        self.cells
            .lock()
            .get(conversation_id)
            .map(|r| Arc::clone(&r.cell))
    }
}
