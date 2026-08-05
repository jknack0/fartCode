//! Session cell — one ACP conversation's live state (ticket E2-11-3).
//!
//! Ports the reference `packages/runtime/src/acp-agents/session/cell.ts`
//! (with its `machine/machine.ts` phase logic folded in). One cell owns one
//! conversation's lifecycle (starting → ready → working/cancelling →
//! closed), the prompt queue, pending permission requests, and the committed
//! turn history. The full transcript-reducer/live-model split arrives with
//! E2-11-4; until then each [`Turn`] keeps its raw `session/update` stream.
//!
//! Deviations from the reference, deliberately:
//! - No 250ms quiescence timer: updates are only accepted while a turn is in
//!   flight or during replay (v1 adapters stream agent output inside turns).
//! - No background-agent counting (`AgentsChanged`): subagent updates arrive
//!   with the transcript reducer.
//! - The queued-prompt continuation runs inside the sending task's loop
//!   instead of a machine effect + callback round-trip (same final state).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use agent_client_protocol_schema::v1::{
    ContentBlock, PermissionOption, SessionId, StopReason, ToolCallUpdate,
};
use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::client::{AcpClient, SessionUpdateEvent};
use crate::error::Error;
use crate::handlers::{PermissionAnswerer, PermissionDecision, PermissionRequest};

/// Lifecycle phase of one session (reference `SessionPhase`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// One prompt turn.
#[derive(Debug, Clone)]
pub struct Turn {
    /// Turn index within the conversation (0-based, creation order).
    pub seq: u64,
    /// `turn-<seq>`.
    pub id: String,
    /// The user prompt text that started the turn.
    pub prompt_text: String,
    /// `None` while the turn is in flight.
    pub outcome: Option<TurnOutcome>,
    /// `session/update` events received during the turn, arrival order.
    pub updates: Vec<SessionUpdateEvent>,
}

/// How a turn ended (reference `TranscriptTurnOutcome`, flattened).
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
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedPrompt {
    pub id: String,
    pub text: String,
    pub hidden_context: Option<String>,
}

/// A permission request surfaced by the adapter, awaiting a user decision.
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone, PartialEq)]
pub struct PromptDraft {
    pub text: String,
    pub rev: u64,
}

/// Snapshot of the cell for UI/state consumers (reference `SessionState`).
#[derive(Debug, Clone)]
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

/// Committed history + the in-flight turn (reference `AcpChatHistory`).
#[derive(Debug, Clone)]
pub struct ChatHistory {
    pub committed: Vec<Turn>,
    pub active: Option<Turn>,
    /// History replayed by `session/load` (before the first live turn).
    pub replay: Vec<SessionUpdateEvent>,
}

/// One page of committed history, newest-last (reference `HistoryPage`).
#[derive(Debug, Clone)]
pub struct HistoryPage {
    pub turns: Vec<Turn>,
    /// Seq to pass as `before` for the next (older) page; `None` when done.
    pub next_cursor: Option<u64>,
}

/// State-change notifications (reference `SessionCellCallbacks`, shrunken to
/// what has consumers this ticket — transcript/draft callbacks return with
/// the live models in E2-11-4).
#[derive(Default, Clone)]
pub struct Callbacks {
    pub on_state_changed: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// Live feed of routed `session/update` events (raw for now; reduced in
/// E2-11-4). Fired once per event, replay included.
pub type UpdateSink = Arc<dyn Fn(&SessionUpdateEvent) + Send + Sync>;

struct PermissionSlot {
    tx: oneshot::Sender<PermissionDecision>,
}

struct Inner {
    lifecycle: Lifecycle,
    active_turn_id: Option<String>,
    queued: VecDeque<QueuedPrompt>,
    pending: Vec<PendingPermission>,
    slots: HashMap<String, PermissionSlot>,
    committed: Vec<Turn>,
    active: Option<Turn>,
    replay: Vec<SessionUpdateEvent>,
    replaying: bool,
    last_stop_reason: Option<String>,
    next_turn_index: u64,
    draft: Option<PromptDraft>,
    draft_rev: u64,
}

/// Owns one ACP conversation: state machine, prompt queue, turn quiescence,
/// permission broker (ticket E2-11-3).
pub struct SessionCell {
    conversation_id: String,
    acp_session_id: SessionId,
    client: Arc<AcpClient>,
    callbacks: Callbacks,
    update_sink: Option<UpdateSink>,
    inner: Mutex<Inner>,
}

impl SessionCell {
    /// Builds a cell in the [`Lifecycle::Starting`] phase. The manager calls
    /// [`SessionCell::mark_ready`] (new session) or [`SessionCell::begin_replay`]
    /// / [`SessionCell::end_replay`] (loaded session) once the adapter answered.
    pub fn new(
        conversation_id: String,
        acp_session_id: SessionId,
        client: Arc<AcpClient>,
        callbacks: Callbacks,
        update_sink: Option<UpdateSink>,
    ) -> Self {
        Self {
            conversation_id,
            acp_session_id,
            client,
            callbacks,
            update_sink,
            inner: Mutex::new(Inner {
                lifecycle: Lifecycle::Starting,
                active_turn_id: None,
                queued: VecDeque::new(),
                pending: Vec::new(),
                slots: HashMap::new(),
                committed: Vec::new(),
                active: None,
                replay: Vec::new(),
                replaying: false,
                last_stop_reason: None,
                next_turn_index: 0,
                draft: None,
                draft_rev: 0,
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
        self.fire_state_changed();
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
        }
        self.fire_state_changed();
    }

    /// Ends the replay window and moves to [`Lifecycle::Ready`].
    pub fn end_replay(&self) {
        {
            let mut inner = self.inner.lock();
            inner.replaying = false;
            if inner.lifecycle == Lifecycle::Starting {
                inner.lifecycle = Lifecycle::Ready;
            }
        }
        self.fire_state_changed();
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
        self.fire_state_changed();
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
            Some(_turn) => {
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
        self.fire_state_changed();
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
        self.fire_state_changed();
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
        self.fire_state_changed();
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
        self.fire_state_changed();
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
        self.fire_state_changed();
        self.client.cancel(&self.acp_session_id).await
    }

    // -- permissions -------------------------------------------------------

    /// Answers a surfaced permission request. Errors when `request_id` is
    /// not pending (reference `resolvePermission`).
    pub fn resolve_permission(&self, request_id: &str, option_id: &str) -> Result<(), Error> {
        let slot = {
            let mut inner = self.inner.lock();
            if !inner.pending.iter().any(|p| p.request_id == request_id) {
                return Err(Error::InvalidState(format!(
                    "no pending permission request with id '{request_id}'"
                )));
            }
            inner.pending.retain(|p| p.request_id != request_id);
            inner.slots.remove(request_id).ok_or_else(|| {
                Error::InvalidState(format!("no resolver for request id '{request_id}'"))
            })?
        };
        let _ = slot
            .tx
            .send(PermissionDecision::Selected(option_id.to_string()));
        self.fire_state_changed();
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
        {
            let mut inner = self.inner.lock();
            inner.pending.push(PendingPermission {
                request_id: request_id.clone(),
                session_id: request.request.session_id.clone(),
                tool_call: request.request.tool_call.clone(),
                options: request.request.options.clone(),
            });
            inner.slots.insert(request_id, PermissionSlot { tx });
        }
        self.fire_state_changed();
        tokio::spawn(async move {
            // An abandoned receiver means the cell was dropped without
            // draining — the spec requires the cancelled outcome then.
            let decision = rx.await.unwrap_or(PermissionDecision::Cancelled);
            let _ = answerer.answer(decision).await;
        });
    }

    // -- modes / config ----------------------------------------------------

    /// Switches the session mode. The adapter validates the id (mode/config
    /// live-model tracking arrives with E2-11-4).
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
        self.fire_state_changed();
        Ok(())
    }

    pub fn prompt_draft(&self) -> Option<PromptDraft> {
        self.inner.lock().draft.clone()
    }

    // -- observation -------------------------------------------------------

    pub fn session_state(&self) -> SessionState {
        let inner = self.inner.lock();
        let working = matches!(inner.lifecycle, Lifecycle::Working | Lifecycle::Cancelling);
        SessionState {
            lifecycle: inner.lifecycle,
            active_turn_id: inner.active_turn_id.clone(),
            pending_permissions: inner.pending.clone(),
            last_stop_reason: inner.last_stop_reason.clone(),
            queued_prompts: inner.queued.iter().cloned().collect(),
            is_generating: working,
            can_submit: inner.lifecycle == Lifecycle::Ready && inner.active.is_none(),
            can_cancel: working,
        }
    }

    pub fn lifecycle(&self) -> Lifecycle {
        self.inner.lock().lifecycle
    }

    pub fn history(&self) -> ChatHistory {
        let inner = self.inner.lock();
        ChatHistory {
            committed: inner.committed.clone(),
            active: inner.active.clone(),
            replay: inner.replay.clone(),
        }
    }

    /// Committed history page, newest-last (reference `getHistory`, limit
    /// defaults to 50 at the call site).
    pub fn history_page(&self, before: Option<u64>, limit: usize) -> HistoryPage {
        let inner = self.inner.lock();
        let turns: Vec<&Turn> = inner
            .committed
            .iter()
            .filter(|t| before.is_none_or(|b| t.seq < b))
            .collect();
        let start = turns.len().saturating_sub(limit);
        let page: Vec<Turn> = turns[start..].iter().map(|t| (*t).clone()).collect();
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

    // -- transport plumbing ------------------------------------------------

    /// Records one `session/update` (routed by the manager or collected by
    /// the in-flight prompt). Replay-window events land in the replay
    /// history; in-turn events on the active turn; anything else is dropped
    /// with a warning (reference "dropping transcript update outside active
    /// turn").
    pub fn record_update(&self, event: SessionUpdateEvent) {
        {
            let mut inner = self.inner.lock();
            if inner.replaying {
                inner.replay.push(event.clone());
            } else if let Some(turn) = inner.active.as_mut() {
                turn.updates.push(event.clone());
            } else {
                tracing::warn!(
                    conversation = %self.conversation_id,
                    phase = inner.lifecycle.as_str(),
                    "SessionCell: dropping update outside active turn"
                );
                return;
            }
        }
        if let Some(sink) = &self.update_sink {
            sink(&event);
        }
        self.fire_state_changed();
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
                blocks.push(text_block(hidden)?);
            }
            let result = self.client.prompt(&self.acp_session_id, blocks).await;
            match result {
                Ok(turn_result) => {
                    for update in turn_result.updates {
                        self.record_update(update);
                    }
                    let cancelling = self.inner.lock().lifecycle == Lifecycle::Cancelling;
                    let reason = stop_reason_str(&turn_result.response.stop_reason);
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
            if let Some(mut turn) = inner.active.take() {
                turn.outcome = Some(outcome);
                inner.committed.push(turn);
            }
            inner.active_turn_id = None;
            if matches!(inner.lifecycle, Lifecycle::Working | Lifecycle::Cancelling) {
                inner.lifecycle = Lifecycle::Ready;
            }
        }
        self.fire_state_changed();
    }

    fn fire_state_changed(&self) {
        if let Some(cb) = &self.callbacks.on_state_changed {
            cb();
        }
    }
}

/// Turn slots open when the cell is idle and not replaying (caller holds the
/// inner lock).
fn can_begin_turn(inner: &Inner) -> bool {
    inner.lifecycle == Lifecycle::Ready && inner.active.is_none() && !inner.replaying
}

/// Creates the turn bookkeeping for a prompt about to send (caller holds the
/// inner lock — this is the atomicity point: exactly one caller wins a turn
/// slot).
fn begin_turn(inner: &mut Inner, prompt_text: &str) -> Turn {
    let seq = inner.next_turn_index;
    inner.next_turn_index += 1;
    let turn = Turn {
        seq,
        id: format!("turn-{seq}"),
        prompt_text: prompt_text.to_string(),
        outcome: None,
        updates: Vec::new(),
    };
    inner.active_turn_id = Some(turn.id.clone());
    inner.active = Some(turn.clone());
    inner.lifecycle = Lifecycle::Working;
    turn
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

fn text_block(text: &str) -> Result<ContentBlock, Error> {
    serde_json::from_value(serde_json::json!({ "type": "text", "text": text }))
        .map_err(|e| Error::Protocol(format!("text content block: {e}")))
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
