//! Session lifecycle for ACP conversations (tickets E2-11-3 / E2-11-4).
//!
//! - [`cell::SessionCell`] — one conversation: state machine, prompt queue,
//!   permission broker, transcript parser, raw log.
//! - [`manager::SessionManager`] — cross-session lifecycle keyed by
//!   conversation id; session-id persistence seam; update/permission
//!   routing.
//! - [`events`] — the live-model event seams (`acp:update` /
//!   `acp:transcript` / `acp:permission_request`) consumed by the app
//!   layer.

pub mod cell;
pub mod events;
pub mod manager;

pub use cell::{
    ChatHistory, HistoryPage, Lifecycle, PendingPermission, PromptDraft, QueuedPrompt, SessionCell,
    SessionState, TurnOutcome,
};
pub use events::{
    AgentTerminalExit, AgentTerminalState, LiveModels, PermissionRequestedEvent, SessionEvents,
};
pub use manager::{SessionIdStore, SessionManager, StartInput, StartOutcome};
