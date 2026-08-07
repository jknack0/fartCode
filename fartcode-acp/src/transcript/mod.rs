//! Transcript reducer + live models for ACP sessions (ticket E2-11-4).
//!
//! Rust port of the reference `packages/core/src/acp/reducer`:
//!
//! - [`normalize`] — stateless `SessionUpdate` → [`normalize::NormalizedEvent`]
//!   decoder.
//! - [`reducer::reduce`] — pure `(ParserState, ReducerInput) → ParserState`
//!   fold over transcript/config/usage/title/agents/plan slices.
//! - [`parser::TranscriptParser`] — the stateful per-conversation wrapper
//!   (push / settle_turn / begin_replay / end_replay / replay).
//! - [`raw_log::RawAcpLog`] — append-only raw-traffic debug artifact.
//!
//! Id synthesis ([`ids`]) keeps the reference's string formats so render
//! identity survives a future migration.

pub mod config_derive;
pub mod fold;
pub mod ids;
pub mod models;
pub mod normalize;
pub mod parser;
pub mod raw_log;
pub mod reducer;

pub use models::{
    AgentState, AgentStatus, ConfigChoice, PlanEntry, PlanEntryInput, PlanEntryPriority,
    PlanEntryStatus, PlanState, Role, SelectGroup, SessionCommand, SessionConfigState,
    SessionUsage, ThinkingStatus, ToolCallItem, ToolGroup, ToolItemKind, ToolNode, ToolStatus,
    TranscriptItem, TranscriptMessage, TranscriptThinking, TranscriptTurn, TranscriptTurnOutcome,
    TurnInitiator, UsageCost, SESSION_PLAN_ID,
};
pub use normalize::{
    decode_session_update, MessageRole, NormalizedDiff, NormalizedEvent, NormalizedToolStatus,
};
pub use parser::{ReplayEntry, ReplayResult, TranscriptParser};
pub use raw_log::{iso_now, now_ms, RawAcpEvent, RawAcpLog, RawAcpLogMeta};
pub use reducer::{reduce, ParserState, ReducerInput, SegmentState, TranscriptSlice};
