//! Session cell — one ACP conversation's live state (ticket E2-11-3,
//! reduced transcript wired in E2-11-4).
//!
//! Ports the reference `packages/runtime/src/acp-agents/session/cell.ts`
//! (with its `machine/machine.ts` phase logic folded in). One cell owns one
//! conversation's lifecycle (starting → ready → working/cancelling →
//! closed), the prompt queue, pending permission requests, the transcript
//! parser (reducer + live models), and the raw-traffic debug log.
//!
//! Deviations from the reference, deliberately:
//! - No 250ms quiescence timer: updates are only accepted while a turn is
//!   in flight or during replay (v1 adapters stream agent output inside
//!   turns).
//! - No background-agent counting (`AgentsChanged`): the agent slice is
//!   driven by the reducer's baseline events only.
//! - The queued-prompt continuation runs inside the sending task's loop
//!   instead of a machine effect + callback round-trip (same final state).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use agent_client_protocol_schema::v1::{
    ContentBlock, PermissionOption, SessionId, SessionUpdate, StopReason, ToolCallUpdate,
};
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::oneshot;

use crate::client::{AcpClient, SessionUpdateEvent};
use crate::error::Error;
use crate::handlers::{PermissionAnswerer, PermissionDecision, PermissionRequest};
use crate::session::events::{LiveModels, PermissionRequestedEvent, SessionEvents};
use crate::transcript::models::{
    DoneTurnReason, ErrorTurnReason, TranscriptTurn, TranscriptTurnOutcome,
};
use crate::transcript::normalize::{MessageRole, NormalizedEvent};
use crate::transcript::raw_log::{iso_now, now_ms, RawAcpEvent, RawAcpLog, RawAcpLogMeta};
use crate::transcript::TranscriptParser;

/// Lifecycle phase of one session (reference `SessionPhase`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    /// `session/new` / `session/load` sent; waiting for ready/replay end.
    Starting,
    /// Idle between turns; prompts run immediately.
    Ready,
    /// A prompt turn is in flight.
    Working,
    /// Cancel requested for the in-flight turn.
    Cancelling,
    /// The adapter is gone; every operation is rejected.
    Closed,
}

impl Lifecycle {
    pub fn as_str(&self) -> &'static str {
        match self {
            Lifecycle::Starting => "starting",
            Lifecycle::Ready => "ready",
            Lifecycle::Working => "working",
            Lifecycle::Cancelling => "cancelling",
            Lifecycle::Closed => "closed",
        }
    }
}

/// How a turn ended (reference `TranscriptTurnOutcome`, flattened for the
/// control plane; mapped to the transcript outcome at settle time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    /// The adapter answered with a stop reason (`end_turn`, `max_tokens`, …).
    Stopped { reason: String },
    /// Stop reason `cancelled` after a local cancel request.
    Cancelled,
    /// The prompt request itself failed (transport/agent error).
    Errored,
}

/// A prompt waiting for a free turn slot (reference `QueuedPrompt`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedPrompt {
    pub id: String,
    pub text: String,
    pub hidden_context: Option<String>,
}

/// A permission request surfaced by the adapter, awaiting a user decision.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPermission {
    /// Our correlation id — pass to [`SessionCell::resolve_permission`].
    pub request_id: String,
    /// Session the adapter asked on.
    pub session_id: SessionId,
    /// The tool call the permission gates (may be partial).
    pub tool_call: ToolCallUpdate,
    /// The options the user may pick from.
    pub options: Vec<PermissionOption>,
}

/// Prompt-composer draft (reference `PromptDraft`, rev-guarded).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptDraft {
    pub text: String,
    pub rev: u64,
}

/// Snapshot of the cell for UI/state consumers (reference `SessionState`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub lifecycle: Lifecycle,
    pub active_turn_id: Option<String>,
    pub pending_permissions: Vec<PendingPermission>,
    pub last_stop_reason: Option<String>,
    pub queued_prompts: Vec<QueuedPrompt>,
    /// True while a turn is in flight (working or cancelling).
    pub is_generating: bool,
    /// A prompt sent now would run immediately (not queue).
    pub can_submit: bool,
    /// There is something to cancel.
    pub can_cancel: bool,
}

/// Committed history + the in-flight turn, reduced (reference
/// `AcpChatHistory`).
#[derive(Debug, Clone)]
pub struct ChatHistory {
    pub committed: Vec<TranscriptTurn>,
    pub active: Option<TranscriptTurn>,
}

/// One page of committed history, newest-last (reference `HistoryPage`).
#[derive(Debug, Clone)]
pub struct HistoryPage {
    pub turns: Vec<TranscriptTurn>,
    /// Seq to pass as `before` for the next (older) page; `None` when done.
    pub next_cursor: Option<u64>,
}

struct PermissionSlot {
    tx: oneshot::Sender<PermissionDecision>,
}

struct Inner {
    lifecycle: Lifecycle,
    active_turn_id: Option<String>,
    queued: VecDeque<QueuedPrompt>,
    pending: Vec<PendingPermission>,
    slots: HashMap<String, PermissionSlot>,
    replaying: bool,
    last_stop_reason: Option<String>,
    next_turn_index: u64,
    draft: Option<PromptDraft>,
    draft_rev: u64,
    parser: TranscriptParser,
}

/// Owns one ACP conversation: state machine, prompt queue, permission
/// broker, transcript parser, raw log (tickets E2-11-3 / E2-11-4).
pub struct SessionCell {
    conversation_id: String,
    acp_session_id: SessionId,
    client: Arc<AcpClient>,
    events: Option<Arc<dyn SessionEvents>>,
    raw_log: Mutex<RawAcpLog>,
    inner: Mutex<Inner>,
}

impl SessionCell {
    /// Builds a cell in the [`Lifecycle::Starting`] phase. The manager calls
    /// [`SessionCell::mark_ready`] (new session) or
    /// [`SessionCell::begin_replay`] / [`SessionCell::end_replay`] (loaded
    /// session) once the adapter answered.
    pub fn new(
        conversation_id: String,
        provider_id: &str,
        acp_session_id: SessionId,
        client: Arc<AcpClient>,
        events: Option<Arc<dyn SessionEvents>>,
    ) -> Self {
        let meta = RawAcpLogMeta {
            conversation_id: conversation_id.clone(),
            provider_id: provider_id.to_string(),
            acp_session_id: acp_session_id.0.to_string(),
            created_at: iso_now(),
        };
        let parser = TranscriptParser::new(conversation_id.clone());
        Self {
            raw_log: Mutex::new(RawAcpLog::new(meta)),
            conversation_id,
            acp_session_id,
            client,
            events,
            inner: Mutex::new(Inner {
                lifecycle: Lifecycle::Starting,
                active_turn_id: None,
                queued: VecDeque::new(),
                pending: Vec::new(),
                slots: HashMap::new(),
                replaying: false,
                last_stop_reason: None,
                next_turn_index: 0,
                draft: None,
                draft_rev: 0,
                parser,
            }),
        }
    }

    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    pub fn acp_session_id(&self) -> &SessionId {
        &self.acp_session_id
    }

    /// The client driving this cell (the manager uses it for shutdown).
    pub fn client(&self) -> &Arc<AcpClient> {
        &self.client
    }

    // -- lifecycle ---------------------------------------------------------

    /// New-session path: starting → ready.
    pub fn mark_ready(&self) {
        {
            let mut inner = self.inner.lock();
            if inner.lifecycle != Lifecycle::Starting {
                tracing::warn!(
                    conversation = %self.conversation_id,
                    phase = inner.lifecycle.as_str(),
                    "SessionCell: mark_ready outside starting"
                );
                return;
            }
            inner.lifecycle = Lifecycle::Ready;
        }
        self.fire_transcript_changed();
    }

    /// Loaded-session path: accept replayed history until `end_replay`.
    pub fn begin_replay(&self) {
        {
            let mut inner = self.inner.lock();
            if inner.lifecycle != Lifecycle::Starting {
                tracing::warn!(
                    conversation = %self.conversation_id,
                    phase = inner.lifecycle.as_str(),
                    "SessionCell: begin_replay outside starting"
                );
                return;
            }
            inner.replaying = true;
            inner.parser.begin_replay();
        }
        self.fire_transcript_changed();
    }

    /// Ends the replay window and moves to [`Lifecycle::Ready`]. The
    /// replayed history settles as committed turns (no stop reason exists
    /// during replay — reference `endReplay`).
    pub fn end_replay(&self) {
        {
            let mut inner = self.inner.lock();
            inner.replaying = false;
            inner.parser.end_replay(now_ms());
            if inner.lifecycle == Lifecycle::Starting {
                inner.lifecycle = Lifecycle::Ready;
            }
        }
        self.fire_transcript_changed();
    }

    /// Process-death path: settle every pending permission as cancelled and
    /// close the cell. Idempotent.
    pub fn dispose(&self) {
        let drained = {
            let mut inner = self.inner.lock();
            inner.lifecycle = Lifecycle::Closed;
            drain_pending(&mut inner)
        };
        for (slot, _) in drained {
            let _ = slot.tx.send(PermissionDecision::Cancelled);
        }
        self.fire_transcript_changed();
    }

    // -- prompts -----------------------------------------------------------

    /// Sends a prompt now when idle, else queues it (reference `prompt()`).
    /// Returns `queued` — `false` means the turn ran to completion in this
    /// call (including any queued follow-ups that drained behind it).
    pub async fn prompt(&self, text: &str, hidden_context: Option<&str>) -> Result<bool, Error> {
        let started = {
            let mut inner = self.inner.lock();
            if inner.lifecycle == Lifecycle::Closed {
                return Err(Error::InvalidState("session is closed".into()));
            }
            can_begin_turn(&inner).then(|| begin_turn(&mut inner, text))
        };
        match started {
            Some(_turn_id) => {
                self.run_turns(text.to_string(), hidden_context.map(str::to_string))
                    .await?;
                Ok(false)
            }
            None => {
                self.queue_prompt(text, hidden_context)?;
                Ok(true)
            }
        }
    }

    /// Appends to the queue (reference `queuePrompt`). Valid in every phase
    /// except [`Lifecycle::Closed`].
    pub fn queue_prompt(
        &self,
        text: &str,
        hidden_context: Option<&str>,
    ) -> Result<QueuedPrompt, Error> {
        let prompt = QueuedPrompt {
            id: uuid::Uuid::new_v4().to_string(),
            text: text.to_string(),
            hidden_context: hidden_context.map(str::to_string),
        };
        {
            let mut inner = self.inner.lock();
            if inner.lifecycle == Lifecycle::Closed {
                return Err(Error::InvalidState("session is closed".into()));
            }
            inner.queued.push_back(prompt.clone());
        }
        self.fire_transcript_changed();
        Ok(prompt)
    }

    /// Dispatches the queue head when a turn slot is free (reference
    /// `SessionReady`/`TurnEnded` dequeue effects). Spawns the sending task
    /// and returns `true` when something was dispatched; `false` when the
    /// queue is empty or busy (the in-flight turn loop will drain it).
    pub fn drain_queue(self: &Arc<Self>) -> bool {
        let next = {
            let mut inner = self.inner.lock();
            if can_begin_turn(&inner) {
                let prompt = inner.queued.pop_front();
                if let Some(p) = &prompt {
                    begin_turn(&mut inner, &p.text);
                }
                prompt
            } else {
                None
            }
        };
        let Some(prompt) = next else {
            return false;
        };
        let this = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(error) = this.run_turns(prompt.text, prompt.hidden_context).await {
                tracing::warn!(
                    conversation = %this.conversation_id,
                    error = %error,
                    "SessionCell: queued prompt failed"
                );
            }
        });
        true
    }

    /// Replaces a queued prompt's text. No-op for unknown ids (reference).
    pub fn edit_queued_prompt(
        &self,
        id: &str,
        text: &str,
        hidden_context: Option<&str>,
    ) -> Result<(), Error> {
        {
            let mut inner = self.inner.lock();
            if inner.lifecycle == Lifecycle::Closed {
                return Err(Error::InvalidState("session is closed".into()));
            }
            for prompt in inner.queued.iter_mut() {
                if prompt.id == id {
                    prompt.text = text.to_string();
                    prompt.hidden_context = hidden_context.map(str::to_string);
                }
            }
        }
        self.fire_transcript_changed();
        Ok(())
    }

    /// Removes a queued prompt. No-op for unknown ids (reference).
    pub fn remove_queued_prompt(&self, id: &str) -> Result<(), Error> {
        {
            let mut inner = self.inner.lock();
            if inner.lifecycle == Lifecycle::Closed {
                return Err(Error::InvalidState("session is closed".into()));
            }
            inner.queued.retain(|p| p.id != id);
        }
        self.fire_transcript_changed();
        Ok(())
    }

    /// Reorders the queue; `ids` must be exactly the current queue's ids
    /// (reference `ReorderQueue` validation).
    pub fn reorder_queue(&self, ids: &[String]) -> Result<(), Error> {
        {
            let mut inner = self.inner.lock();
            if inner.lifecycle == Lifecycle::Closed {
                return Err(Error::InvalidState("session is closed".into()));
            }
            if ids.len() != inner.queued.len()
                || ids
                    .iter()
                    .any(|id| !inner.queued.iter().any(|p| &p.id == id))
            {
                return Err(Error::InvalidState(
                    "queue reorder ids must match the queued prompts".into(),
                ));
            }
            let by_id: HashMap<String, QueuedPrompt> =
                inner.queued.drain(..).map(|p| (p.id.clone(), p)).collect();
            for id in ids {
                if let Some(p) = by_id.get(id) {
                    inner.queued.push_back(p.clone());
                }
            }
        }
        self.fire_transcript_changed();
        Ok(())
    }

    /// Cancels the in-flight turn (`session/cancel`). No-op when nothing is
    /// running (reference `Cancel` decision).
    pub async fn cancel(&self) -> Result<(), Error> {
        {
            let mut inner = self.inner.lock();
            if inner.lifecycle != Lifecycle::Working {
                return Ok(());
            }
            inner.lifecycle = Lifecycle::Cancelling;
            let drained = drain_pending(&mut inner);
            for (slot, _) in drained {
                let _ = slot.tx.send(PermissionDecision::Cancelled);
            }
        }
        self.fire_transcript_changed();
        self.client.cancel(&self.acp_session_id).await
    }

    // -- permissions -------------------------------------------------------

    /// Answers a surfaced permission request. Errors when `request_id` is
    /// not pending (reference `resolvePermission`).
    pub fn resolve_permission(&self, request_id: &str, option_id: &str) -> Result<(), Error> {
        let (slot, session_id) = {
            let mut inner = self.inner.lock();
            let Some(pending) = inner.pending.iter().find(|p| p.request_id == request_id) else {
                return Err(Error::InvalidState(format!(
                    "no pending permission request with id '{request_id}'"
                )));
            };
            let session_id = pending.session_id.clone();
            inner.pending.retain(|p| p.request_id != request_id);
            let slot = inner.slots.remove(request_id).ok_or_else(|| {
                Error::InvalidState(format!("no resolver for request id '{request_id}'"))
            })?;
            (slot, session_id)
        };
        self.raw_log.lock().record(RawAcpEvent::PermissionResolved {
            session_id: session_id.0.to_string(),
            request_id: request_id.to_string(),
            option_id: option_id.to_string(),
        });
        let _ = slot
            .tx
            .send(PermissionDecision::Selected(option_id.to_string()));
        self.fire_transcript_changed();
        Ok(())
    }

    /// Permission-broker entry: called from the client's permission resolver
    /// with the surfaced request and its transport answerer. Registers the
    /// request as pending and answers the adapter once
    /// [`SessionCell::resolve_permission`] (or cancel/dispose) settles it.
    pub fn permission_requested(
        &self,
        request: PermissionRequest,
        answerer: Arc<PermissionAnswerer>,
    ) {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        let pending = PendingPermission {
            request_id: request_id.clone(),
            session_id: request.request.session_id.clone(),
            tool_call: request.request.tool_call.clone(),
            options: request.request.options.clone(),
        };
        {
            let mut inner = self.inner.lock();
            inner.pending.push(pending.clone());
            inner
                .slots
                .insert(request_id.clone(), PermissionSlot { tx });
        }
        self.raw_log.lock().record(RawAcpEvent::PermissionRequest {
            session_id: pending.session_id.0.to_string(),
            request: serde_json::to_value(&request.request).unwrap_or_default(),
        });
        if let Some(events) = &self.events {
            events.permission_requested(
                &self.conversation_id,
                &PermissionRequestedEvent {
                    request_id: request_id.clone(),
                    pending: pending.clone(),
                },
            );
        }
        self.fire_transcript_changed();
        tokio::spawn(async move {
            // An abandoned receiver means the cell was dropped without
            // draining — the spec requires the cancelled outcome then.
            let decision = rx.await.unwrap_or(PermissionDecision::Cancelled);
            let _ = answerer.answer(decision).await;
        });
    }

    // -- modes / config ----------------------------------------------------

    /// Switches the session mode. The adapter validates the id (the mode
    /// live model tracks `current_mode_update` through the parser).
    pub async fn set_mode(&self, mode_id: &str) -> Result<(), Error> {
        self.client.set_mode(&self.acp_session_id, mode_id).await?;
        Ok(())
    }

    /// Sets a session config option (model, effort, …). The adapter validates
    /// the config id.
    pub async fn set_config_option(
        &self,
        config_id: &str,
        value: agent_client_protocol_schema::v1::SessionConfigOptionValue,
    ) -> Result<(), Error> {
        self.client
            .set_config_option(&self.acp_session_id, config_id, value)
            .await?;
        Ok(())
    }

    // -- draft -------------------------------------------------------------

    /// Rev-guarded composer draft (reference `setPromptDraft`): stale revs
    /// are ignored, `None` clears.
    pub fn set_prompt_draft(&self, rev: u64, text: Option<&str>) -> Result<(), Error> {
        {
            let mut inner = self.inner.lock();
            if rev <= inner.draft_rev {
                return Ok(());
            }
            inner.draft_rev = rev;
            inner.draft = text.map(|t| PromptDraft {
                text: t.to_string(),
                rev,
            });
        }
        self.fire_transcript_changed();
        Ok(())
    }

    pub fn prompt_draft(&self) -> Option<PromptDraft> {
        self.inner.lock().draft.clone()
    }

    // -- observation -------------------------------------------------------

    pub fn session_state(&self) -> SessionState {
        session_state_of(&self.inner.lock())
    }

    pub fn lifecycle(&self) -> Lifecycle {
        self.inner.lock().lifecycle
    }

    /// Full live-model snapshot for `acp:transcript` emission (the UI
    /// rehydrates from this; reference live-model pull).
    pub fn live_models(&self) -> LiveModels {
        let inner = self.inner.lock();
        LiveModels {
            session_state: session_state_of(&inner),
            committed: inner.parser.history().to_vec(),
            active_turn: inner.parser.active_turn().cloned(),
            config: inner.parser.config().clone(),
            usage: inner.parser.usage().cloned(),
            title: inner.parser.title().map(str::to_string),
            agents: inner.parser.agents().to_vec(),
            plan: inner.parser.plan().cloned(),
            draft: inner.draft.clone(),
            queued_prompts: inner.queued.iter().cloned().collect(),
            // Agent-managed terminals arrive with the Phase-4 terminal
            // capability (client advertises none until then).
            terminals: Vec::new(),
        }
    }

    pub fn history(&self) -> ChatHistory {
        let inner = self.inner.lock();
        ChatHistory {
            committed: inner.parser.history().to_vec(),
            active: inner.parser.active_turn().cloned(),
        }
    }

    /// Committed history page, newest-last (reference `getHistory`, limit
    /// defaults to 50 at the call site).
    pub fn history_page(&self, before: Option<u64>, limit: usize) -> HistoryPage {
        let inner = self.inner.lock();
        let turns: Vec<&TranscriptTurn> = inner
            .parser
            .history()
            .iter()
            .filter(|t| before.is_none_or(|b| t.seq < b))
            .collect();
        let start = turns.len().saturating_sub(limit);
        let page: Vec<TranscriptTurn> = turns[start..].iter().map(|t| (*t).clone()).collect();
        let next_cursor = if page.len() == limit {
            page.first().map(|t| t.seq)
        } else {
            None
        };
        HistoryPage {
            turns: page,
            next_cursor,
        }
    }

    /// Raw ACP traffic log for debugging (reference `exportRawLog`).
    pub fn export_raw_log(&self) -> String {
        self.raw_log.lock().export_json()
    }

    // -- transport plumbing ------------------------------------------------

    /// Records one `session/update` (routed by the manager or collected by
    /// the in-flight prompt): folds it into the transcript parser, appends
    /// it to the raw log, and fires `acp:update` + `acp:transcript`.
    pub fn record_update(&self, event: SessionUpdateEvent) {
        // Hidden-context blocks echoed back (live) or replayed (session/load)
        // as user chunks stay out of the visible transcript; the raw log
        // below still records them for debugging.
        if !is_hidden_context_echo(&event.update) {
            let mut inner = self.inner.lock();
            inner.parser.push(&event.update, now_ms());
        }
        self.raw_log.lock().record(RawAcpEvent::SessionUpdate {
            session_id: event.session_id.0.to_string(),
            update: serde_json::to_value(&event.update).unwrap_or_default(),
        });
        if let Some(events) = &self.events {
            events.update(&self.conversation_id, &event);
        }
        self.fire_transcript_changed();
    }

    // -- internals ---------------------------------------------------------

    /// Runs a turn for `text` (bookkeeping already done via [`begin_turn`])
    /// and then drains queued prompts until the queue empties or the cell
    /// closes (reference sendPrompt loop via machine effects).
    async fn run_turns(
        &self,
        mut text: String,
        mut hidden_context: Option<String>,
    ) -> Result<(), Error> {
        loop {
            let mut blocks = Vec::new();
            if !text.is_empty() {
                blocks.push(text_block(&text)?);
            }
            if let Some(hidden) = &hidden_context {
                blocks.push(text_block(&format!("{HIDDEN_CONTEXT_SENTINEL}\n{hidden}"))?);
            }
            self.raw_log.lock().record(RawAcpEvent::Prompt {
                session_id: self.acp_session_id.0.to_string(),
                content: serde_json::to_value(&blocks).unwrap_or_default(),
            });
            let result = self.client.prompt(&self.acp_session_id, blocks).await;
            match result {
                Ok(turn_result) => {
                    for update in turn_result.updates {
                        self.record_update(update);
                    }
                    let cancelling = self.inner.lock().lifecycle == Lifecycle::Cancelling;
                    let reason = stop_reason_str(&turn_result.response.stop_reason);
                    self.raw_log.lock().record(RawAcpEvent::PromptResult {
                        session_id: self.acp_session_id.0.to_string(),
                        stop_reason: Some(reason.to_string()),
                    });
                    let outcome = if cancelling && reason == "cancelled" {
                        TurnOutcome::Cancelled
                    } else {
                        TurnOutcome::Stopped {
                            reason: reason.to_string(),
                        }
                    };
                    self.settle_turn(outcome);
                }
                Err(error) => {
                    self.raw_log.lock().record(RawAcpEvent::PromptResult {
                        session_id: self.acp_session_id.0.to_string(),
                        stop_reason: None,
                    });
                    self.settle_turn(TurnOutcome::Errored);
                    return Err(error);
                }
            }

            // Continuation: atomically claim the next queued prompt (pop +
            // begin_turn under one lock so a racing drain/prompt cannot win
            // the slot twice).
            let next = {
                let mut inner = self.inner.lock();
                if can_begin_turn(&inner) {
                    if let Some(prompt) = inner.queued.pop_front() {
                        begin_turn(&mut inner, &prompt.text);
                        Some(prompt)
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            match next {
                Some(next_prompt) => {
                    text = next_prompt.text;
                    hidden_context = next_prompt.hidden_context;
                }
                None => return Ok(()),
            }
        }
    }

    /// Commits the active turn and returns to [`Lifecycle::Ready`]
    /// (reference `TurnEnded`).
    fn settle_turn(&self, outcome: TurnOutcome) {
        {
            let mut inner = self.inner.lock();
            inner.last_stop_reason = match &outcome {
                TurnOutcome::Stopped { reason } => Some(reason.clone()),
                TurnOutcome::Cancelled => Some("cancelled".into()),
                TurnOutcome::Errored => None,
            };
            inner
                .parser
                .settle_turn(transcript_outcome(&outcome), now_ms());
            inner.active_turn_id = None;
            if matches!(inner.lifecycle, Lifecycle::Working | Lifecycle::Cancelling) {
                inner.lifecycle = Lifecycle::Ready;
            }
        }
        self.fire_transcript_changed();
    }

    fn fire_transcript_changed(&self) {
        let Some(events) = &self.events else {
            return;
        };
        let models = self.live_models();
        events.transcript_changed(&self.conversation_id, &models);
    }
}

/// Turn slots open when the cell is idle and not replaying (caller holds the
/// inner lock).
fn can_begin_turn(inner: &Inner) -> bool {
    inner.lifecycle == Lifecycle::Ready && inner.active_turn_id.is_none() && !inner.replaying
}

/// Creates the turn bookkeeping for a prompt about to send (caller holds the
/// inner lock — this is the atomicity point: exactly one caller wins a turn
/// slot). Injects the synthetic user message into the transcript first
/// (reference `cell.ts` pushEvent of the user message at prompt time).
fn begin_turn(inner: &mut Inner, prompt_text: &str) -> String {
    let seq = inner.next_turn_index;
    inner.next_turn_index += 1;
    if !prompt_text.is_empty() {
        inner.parser.push_event(
            NormalizedEvent::Message {
                role: MessageRole::User,
                message_id: Some(format!("{}-{seq}-user", inner.parser.conversation_id())),
                text: prompt_text.to_string(),
            },
            now_ms(),
        );
    }
    let turn_id = inner
        .parser
        .active_turn()
        .map(|t| t.id.clone())
        .unwrap_or_default();
    inner.active_turn_id = Some(turn_id.clone());
    inner.lifecycle = Lifecycle::Working;
    turn_id
}

/// Removes every pending permission and returns their resolver slots.
fn drain_pending(inner: &mut Inner) -> Vec<(PermissionSlot, String)> {
    let pending = std::mem::take(&mut inner.pending);
    pending
        .into_iter()
        .filter_map(|p| {
            inner
                .slots
                .remove(&p.request_id)
                .map(|slot| (slot, p.request_id))
        })
        .collect()
}

fn session_state_of(inner: &Inner) -> SessionState {
    let working = matches!(inner.lifecycle, Lifecycle::Working | Lifecycle::Cancelling);
    SessionState {
        lifecycle: inner.lifecycle,
        active_turn_id: inner.active_turn_id.clone(),
        pending_permissions: inner.pending.clone(),
        last_stop_reason: inner.last_stop_reason.clone(),
        queued_prompts: inner.queued.iter().cloned().collect(),
        is_generating: working,
        can_submit: inner.lifecycle == Lifecycle::Ready
            && inner.active_turn_id.is_none()
            && !inner.replaying,
        can_cancel: working,
    }
}

/// Control-plane outcome → transcript outcome (reference
/// `outcomeFromStopReason` + the settleTurn mapping).
fn transcript_outcome(outcome: &TurnOutcome) -> TranscriptTurnOutcome {
    match outcome {
        TurnOutcome::Stopped { reason } => TranscriptTurnOutcome::Done {
            reason: parse_stop_reason(reason),
        },
        TurnOutcome::Cancelled => TranscriptTurnOutcome::Cancelled { reason: None },
        TurnOutcome::Errored => TranscriptTurnOutcome::Error {
            reason: Some(ErrorTurnReason::PromptFailed),
        },
    }
}

fn parse_stop_reason(reason: &str) -> Option<DoneTurnReason> {
    match reason {
        "end_turn" => Some(DoneTurnReason::EndTurn),
        "max_tokens" => Some(DoneTurnReason::MaxTokens),
        "max_turn_requests" => Some(DoneTurnReason::MaxTurnRequests),
        "refusal" => Some(DoneTurnReason::Refusal),
        "cancelled" => Some(DoneTurnReason::Cancelled),
        _ => None,
    }
}

/// Prefixed to hidden-context prompt blocks so copies that come back as
/// `user_message_chunk` (live echo or `session/load` replay) can be
/// recognized and suppressed from the transcript.
pub const HIDDEN_CONTEXT_SENTINEL: &str = "[fartCode:hidden-context]";

/// True for a user chunk that is an echoed/replayed hidden-context block.
// ponytail: only catches the chunk carrying the sentinel; an adapter that
// splits one text block across chunks would leak the tail — none do today.
fn is_hidden_context_echo(update: &SessionUpdate) -> bool {
    match update {
        SessionUpdate::UserMessageChunk(chunk) => match &chunk.content {
            ContentBlock::Text(t) => t.text.starts_with(HIDDEN_CONTEXT_SENTINEL),
            _ => false,
        },
        _ => false,
    }
}

fn text_block(text: &str) -> Result<ContentBlock, Error> {
    serde_json::from_value(serde_json::json!({ "type": "text", "text": text }))
        .map_err(|e| Error::Protocol(format!("text content block: {e}")))
}

#[cfg(test)]
mod hidden_context_tests {
    use super::*;

    fn user_chunk(text: &str) -> SessionUpdate {
        serde_json::from_value(serde_json::json!({
            "sessionUpdate": "user_message_chunk",
            "content": { "type": "text", "text": text },
        }))
        .unwrap()
    }

    #[test]
    fn suppresses_only_sentinel_user_chunks() {
        let hidden = format!("{HIDDEN_CONTEXT_SENTINEL}\nYou are the PM.");
        assert!(is_hidden_context_echo(&user_chunk(&hidden)));
        assert!(!is_hidden_context_echo(&user_chunk(
            "implement the feature"
        )));
        // Agent chunks are never suppressed, even with the sentinel.
        let agent: SessionUpdate = serde_json::from_value(serde_json::json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": hidden },
        }))
        .unwrap();
        assert!(!is_hidden_context_echo(&agent));
    }
}

fn stop_reason_str(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::MaxTurnRequests => "max_turn_requests",
        StopReason::Refusal => "refusal",
        StopReason::Cancelled => "cancelled",
        _ => "other",
    }
}
