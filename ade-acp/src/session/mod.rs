//! Session lifecycle for ACP conversations (ticket E2-11-3).
//!
//! - [`cell::SessionCell`] — one conversation: state machine, prompt queue,
//!   turn quiescence, permission broker.
//! - [`manager::SessionManager`] — cross-session lifecycle keyed by
//!   conversation id; session-id persistence seam; update/permission
//!   routing.

pub mod cell;
pub mod manager;

pub use cell::{
    Callbacks, ChatHistory, HistoryPage, Lifecycle, PendingPermission, PromptDraft, QueuedPrompt,
    SessionCell, SessionState, Turn, TurnOutcome, UpdateSink,
};
pub use manager::{SessionIdStore, SessionManager, StartInput, StartOutcome};
