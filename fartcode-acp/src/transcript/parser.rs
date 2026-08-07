//! Stateful transcript parser (ticket E2-11-4).
//!
//! Port of the reference `reducer/parser.ts` (`AcpTranscriptParser`): one
//! [`TranscriptParser`] per conversation folds raw `session/update` chunks
//! into all slices — transcript (committed turns + active turn), config,
//! usage, title, agents, plan.
//!
//! Live streaming: `push` per update, `settle_turn` when the prompt
//! resolves. Bounded replay: `begin_replay` → `push` per entry →
//! `end_replay` (the trailing active turn is closed at EOF — there is no
//! stop reason during replay).

use agent_client_protocol_schema::v1::SessionUpdate;

use crate::transcript::models::{
    AgentState, PlanState, SessionConfigState, SessionUsage, TranscriptTurn, TranscriptTurnOutcome,
};
use crate::transcript::normalize::NormalizedEvent;
use crate::transcript::reducer::{ParserState, ReducerInput};

/// One replay entry: a raw update with an optional timestamp.
pub struct ReplayEntry {
    pub update: SessionUpdate,
    /// Epoch ms; defaults to the running counter when absent.
    pub at: Option<u64>,
}

/// Result of a bounded replay.
pub struct ReplayResult {
    pub committed: Vec<TranscriptTurn>,
    pub active: Option<TranscriptTurn>,
    pub config: SessionConfigState,
    pub usage: Option<SessionUsage>,
    pub title: Option<String>,
    pub agents: Vec<AgentState>,
    pub plan: Option<PlanState>,
}

/// Folds a raw ACP update stream into reduced transcript slices
/// (reference `AcpTranscriptParser`).
pub struct TranscriptParser {
    conversation_id: String,
    state: ParserState,
}

impl TranscriptParser {
    pub fn new(conversation_id: impl Into<String>) -> Self {
        Self {
            conversation_id: conversation_id.into(),
            state: ParserState::new(),
        }
    }

    /// The conversation this parser reduces for (used for synthetic
    /// message ids at turn open).
    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    /// Feed one raw ACP `SessionUpdate`. Routes to the appropriate slice;
    /// may open or close a turn.
    pub fn push(&mut self, update: &SessionUpdate, at: u64) {
        self.state = crate::transcript::reducer::reduce(
            &self.state,
            ReducerInput::Update { update, at },
            &self.conversation_id,
        );
    }

    /// Feed an already-decoded event (used for synthetic events the cell
    /// produces itself, e.g. the opening user message).
    pub fn push_event(&mut self, event: NormalizedEvent, at: u64) {
        self.state = crate::transcript::reducer::reduce(
            &self.state,
            ReducerInput::Event { event, at },
            &self.conversation_id,
        );
    }

    /// Explicitly close the active transcript turn (prompt resolved; the
    /// stop reason belongs to the session state machine, not the
    /// transcript). No-op when there is no active turn.
    pub fn end_turn(&mut self, at: u64) {
        self.state = crate::transcript::reducer::reduce(
            &self.state,
            ReducerInput::TurnEnd { at, outcome: None },
            &self.conversation_id,
        );
    }

    /// Close the active turn with a durable outcome.
    pub fn settle_turn(&mut self, outcome: TranscriptTurnOutcome, at: u64) {
        self.state = crate::transcript::reducer::reduce(
            &self.state,
            ReducerInput::TurnEnd {
                at,
                outcome: Some(outcome),
            },
            &self.conversation_id,
        );
    }

    pub fn begin_replay(&mut self) {
        self.state = crate::transcript::reducer::reduce(
            &self.state,
            ReducerInput::ReplayStart,
            &self.conversation_id,
        );
    }

    pub fn end_replay(&mut self, at: u64) {
        self.state = crate::transcript::reducer::reduce(
            &self.state,
            ReducerInput::ReplayEnd { at },
            &self.conversation_id,
        );
    }

    /// Reset all slices to their initial state.
    pub fn reset(&mut self) {
        self.state = ParserState::new();
    }

    // -- observation -----------------------------------------------------

    /// All finalized, committed turns in chronological order.
    pub fn history(&self) -> &[TranscriptTurn] {
        &self.state.transcript.committed
    }

    /// The in-flight turn, or `None` when the session is idle.
    pub fn active_turn(&self) -> Option<&TranscriptTurn> {
        self.state.transcript.active.as_ref()
    }

    /// Latest materialized session config.
    pub fn config(&self) -> &SessionConfigState {
        &self.state.config
    }

    /// Latest context-window usage.
    pub fn usage(&self) -> Option<&SessionUsage> {
        self.state.usage.as_ref()
    }

    /// Latest session title.
    pub fn title(&self) -> Option<&str> {
        self.state.title.as_deref()
    }

    pub fn agents(&self) -> &[AgentState] {
        &self.state.agents
    }

    pub fn plan(&self) -> Option<&PlanState> {
        self.state.plan.as_ref()
    }

    /// Fold a finite sequence of updates and return every slice. The
    /// trailing active turn (if any) is closed at EOF (reference
    /// `AcpTranscriptParser.replay`).
    pub fn replay(entries: &[ReplayEntry], conversation_id: impl Into<String>) -> ReplayResult {
        let mut parser = Self::new(conversation_id);
        let mut at = 0u64;
        parser.begin_replay();
        for entry in entries {
            if let Some(entry_at) = entry.at {
                at = entry_at;
            }
            parser.push(&entry.update, at);
            at = at.saturating_add(1);
        }
        parser.end_replay(at);

        ReplayResult {
            committed: parser.history().to_vec(),
            active: parser.active_turn().cloned(),
            config: parser.config().clone(),
            usage: parser.usage().cloned(),
            title: parser.title().map(str::to_string),
            agents: parser.agents().to_vec(),
            plan: parser.plan().cloned(),
        }
    }
}
