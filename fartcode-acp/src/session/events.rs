//! Live-model snapshot + event seams for ACP sessions (ticket E2-11-4).
//!
//! The cell fires three event kinds through [`SessionEvents`] (the app
//! layer implements it over Tauri events — `acp:update`, `acp:transcript`,
//! `acp:permission_request`, keyed by conversation id):
//!
//! - [`SessionEvents::update`] — every routed raw `session/update`.
//! - [`SessionEvents::transcript_changed`] — after any state change, with
//!   the full [`LiveModels`] snapshot (the UI rehydrates from it).
//! - [`SessionEvents::permission_requested`] — when the adapter surfaces a
//!   permission request.
//!
//! The terminal live models (`terminals` + `terminalOutput` live log in the
//! reference) are modeled by [`AgentTerminalState`] but stay empty in
//! Phase 2: agent-managed terminals require the `terminal` client
//! capability, which lands in Phase 4 (the handlers answer `terminal/*`
//! with MethodNotFound until then).

use serde::Serialize;

use crate::client::SessionUpdateEvent;
use crate::session::cell::{PendingPermission, PromptDraft, QueuedPrompt, SessionState};
use crate::transcript::models::{
    AgentState, PlanState, SessionConfigState, SessionUsage, TranscriptTurn,
};

/// Everything the UI subscribes to for one conversation (the
/// `acp:transcript` payload). A full snapshot — cheap to rehydrate from,
/// mirrors the reference's "transcript changed → pull the slices" pattern.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveModels {
    /// Session lifecycle, affordances, pending permissions, last stop
    /// reason.
    pub session_state: SessionState,
    /// Committed turns, oldest first.
    pub committed: Vec<TranscriptTurn>,
    /// The in-flight turn, when one is running.
    pub active_turn: Option<TranscriptTurn>,
    /// Model/effort/mode selectors + advertised commands.
    pub config: SessionConfigState,
    /// Context-window usage.
    pub usage: Option<SessionUsage>,
    /// Session title from `session_info_update`.
    pub title: Option<String>,
    /// Spawned (sub)agents.
    pub agents: Vec<AgentState>,
    /// The session plan.
    pub plan: Option<PlanState>,
    /// Composer draft (rev-guarded).
    pub draft: Option<PromptDraft>,
    /// Prompts waiting for a free turn slot.
    pub queued_prompts: Vec<QueuedPrompt>,
    /// Agent-managed terminals — always empty until the Phase-4 terminal
    /// capability lands (see module docs).
    pub terminals: Vec<AgentTerminalState>,
}

/// Serializable snapshot of an agent-managed terminal (reference
/// `terminalStateSchema`). Unpopulated in Phase 2.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTerminalState {
    pub terminal_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub output: String,
    pub truncated: bool,
    pub exit_status: Option<AgentTerminalExit>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTerminalExit {
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
}

/// Payload of `acp:permission_request` (what the UI renders as the
/// permission prompt).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequestedEvent {
    /// Our correlation id — pass to the resolve command.
    pub request_id: String,
    /// The pending request (tool call snapshot + options).
    pub pending: PendingPermission,
}

/// Event sink for one conversation's ACP session. Implemented at the app
/// layer over the Tauri emitter (ticket E2-11-5 wires it); `fartcode-acp` stays
/// transport-agnostic (ADR-0027 layering).
pub trait SessionEvents: Send + Sync {
    /// One raw `session/update` was routed to the conversation
    /// (`acp:update`). Replay-window updates included.
    fn update(&self, conversation_id: &str, event: &SessionUpdateEvent);

    /// The reduced transcript or any live model changed (`acp:transcript`).
    fn transcript_changed(&self, conversation_id: &str, models: &LiveModels);

    /// The adapter surfaced a permission request
    /// (`acp:permission_request`).
    fn permission_requested(&self, conversation_id: &str, event: &PermissionRequestedEvent);
}
