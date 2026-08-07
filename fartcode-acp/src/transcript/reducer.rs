//! Pure parser-state reducer (ticket E2-11-4).
//!
//! Direct port of the reference `reducer/reducer.ts`: one pure function
//! `(ParserState, ReducerInput) → ParserState` folds raw `session/update`
//! chunks (or already-decoded events) into every slice — transcript,
//! config, usage, title, agents, plan. All state changes produce a new
//! state; no mutation.
//!
//! Deviations from the reference, deliberately:
//! - No `EnrichHook` (no provider `_meta` promotion exists in Phase 2);
//!   the agent slice still updates from baseline events.
//! - `subagent`/`subagent_update` events never arrive without an enrich
//!   hook, so [`update_agent_slice`] only handles baseline shapes (it is
//!   kept as the seam where the hook will feed).

use agent_client_protocol_schema::v1::SessionUpdate;

use crate::transcript::config_derive::derive_config_groups;
use crate::transcript::fold::{finalize_items, fold_item, FoldEvent};
use crate::transcript::ids::{make_message_id, make_thinking_id, make_turn_id};
use crate::transcript::models::{
    AgentState, PlanEntry, PlanState, SessionCommand, SessionConfigState, SessionUsage,
    ThinkingStatus, TranscriptItem, TranscriptTurn, TranscriptTurnOutcome, TurnInitiator,
    SESSION_PLAN_ID,
};
use crate::transcript::normalize::{decode_session_update, MessageRole, NormalizedEvent};

/// Which synthesized stream a message/thinking event belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SegmentKind {
    MessageUser,
    MessageAssistant,
    Thinking,
}

/// Segment bookkeeping for providers that omit messageIds (reference
/// `SegmentState`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SegmentState {
    open: Option<SegmentKind>,
    user: u64,
    assistant: u64,
    thinking: u64,
}

/// Committed history + the in-flight turn (reference `TranscriptSlice`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TranscriptSlice {
    pub committed: Vec<TranscriptTurn>,
    pub active: Option<TranscriptTurn>,
}

/// Everything the parser owns (reference `ParserState`).
#[derive(Debug, Clone, PartialEq)]
pub struct ParserState {
    pub transcript: TranscriptSlice,
    pub config: SessionConfigState,
    pub usage: Option<SessionUsage>,
    pub title: Option<String>,
    pub pending_mode_id: Option<String>,
    pub segment: SegmentState,
    pub agents: Vec<AgentState>,
    pub plan: Option<PlanState>,
}

impl Default for ParserState {
    fn default() -> Self {
        Self::new()
    }
}

impl ParserState {
    pub fn new() -> Self {
        Self {
            transcript: TranscriptSlice::default(),
            config: SessionConfigState::default(),
            usage: None,
            title: None,
            pending_mode_id: None,
            segment: SegmentState::default(),
            agents: Vec::new(),
            plan: None,
        }
    }
}

/// One fold input (reference `ReducerInput`).
pub enum ReducerInput<'a> {
    Update {
        update: &'a SessionUpdate,
        at: u64,
    },
    Event {
        event: NormalizedEvent,
        at: u64,
    },
    ReplayStart,
    ReplayEnd {
        at: u64,
    },
    TurnEnd {
        at: u64,
        outcome: Option<TranscriptTurnOutcome>,
    },
}

/// Pure reducer: `(ParserState, ReducerInput, conversation_id) → ParserState`.
pub fn reduce(state: &ParserState, input: ReducerInput<'_>, conversation_id: &str) -> ParserState {
    let (event, at) = match input {
        ReducerInput::ReplayStart => return ParserState::new(),
        ReducerInput::ReplayEnd { at } => return settle(state, at, None),
        ReducerInput::TurnEnd { at, outcome } => return settle(state, at, outcome),
        ReducerInput::Update { update, at } => (decode_session_update(update), at),
        ReducerInput::Event { event, at } => (event, at),
    };

    // Meta slices: config, modes, commands, usage, title, agents.
    match &event {
        NormalizedEvent::Config { options } => {
            let groups = derive_config_groups(options);
            let mut config = state.config.clone();
            if let Some(model_options) = groups.model_options {
                config.model_options = Some(model_options);
            }
            if let Some(efforts) = groups.efforts {
                config.efforts = Some(efforts);
            }
            if let Some(mode_options) = groups.mode_options {
                config.mode_options = Some(mode_options);
            }
            if let Some(pending) = &state.pending_mode_id {
                if let Some(modes) = config.mode_options.as_mut() {
                    modes.selected = Some(pending.clone());
                }
            }
            let mut next = state.clone();
            next.config = config;
            next.pending_mode_id = if next.config.mode_options.is_some() {
                None
            } else {
                state.pending_mode_id.clone()
            };
            return next;
        }
        NormalizedEvent::ModeSelected { mode_id } => {
            let mut next = state.clone();
            if let Some(modes) = next.config.mode_options.as_mut() {
                modes.selected = Some(mode_id.clone());
                next.pending_mode_id = None;
            } else {
                next.pending_mode_id = Some(mode_id.clone());
            }
            return next;
        }
        NormalizedEvent::Commands { commands } => {
            let available_commands: Vec<SessionCommand> = commands
                .iter()
                .map(|c| {
                    let input_hint = match &c.input {
                        Some(
                            agent_client_protocol_schema::v1::AvailableCommandInput::Unstructured(
                                u,
                            ),
                        ) => Some(u.hint.clone()),
                        _ => None,
                    };
                    SessionCommand {
                        name: c.name.clone(),
                        description: c.description.clone(),
                        source: "provider-command".to_string(),
                        input_hint,
                    }
                })
                .collect();
            let mut next = state.clone();
            next.config.available_commands = available_commands;
            return next;
        }
        NormalizedEvent::Usage { usage } => {
            let mut next = state.clone();
            next.usage = Some(usage.clone());
            return next;
        }
        NormalizedEvent::Title { title } => {
            let mut next = state.clone();
            next.title = Some(title.clone());
            return next;
        }
        NormalizedEvent::Ignored => return state.clone(),
        _ => {}
    }

    let mut transcript = state.transcript.clone();
    let mut segment = state.segment.clone();
    let plan = update_plan_slice(state.plan.as_ref(), &event, at);

    // OPEN boundary: a new user message starts a new turn.
    if let NormalizedEvent::Message {
        role: MessageRole::User,
        message_id,
        ..
    } = &event
    {
        if is_new_user_message(
            transcript.active.as_ref(),
            message_id.as_deref(),
            &segment,
            &transcript,
        ) {
            let (closed, _) = close_active(&transcript, at, None);
            transcript = closed;
            transcript = open_turn(transcript, conversation_id, TurnInitiator::User);
            segment = SegmentState::default();
        }
    }

    // Lazy open: agent-initiated content with no active turn.
    if transcript.active.is_none() {
        transcript = open_turn(transcript, conversation_id, TurnInitiator::Agent);
        segment = SegmentState::default();
    }

    let materialized = materialize_event(transcript, segment, &event, at);
    let mut transcript = materialized.transcript;
    let segment = materialized.segment;
    let fold_event = materialized.event;

    let agents = {
        let active_id = transcript.active.as_ref().map(|t| t.id.clone());
        update_agent_slice(&state.agents, &event, active_id.as_deref(), at)
    };

    let active = transcript
        .active
        .as_mut()
        .expect("active turn opened above");
    let items = fold_item(&active.items, &fold_event, &active.id, at);
    if items != active.items {
        active.items = items;
    }

    debug_assert_transcript(&transcript);

    let mut next = state.clone();
    next.transcript = transcript;
    next.segment = segment;
    next.agents = agents;
    if plan != next.plan {
        next.plan = plan;
    }
    next
}

/// Close the active turn (commit it, resetting the segment state) —
/// shared by `replay_end` and `turn_end` (reference behavior: both close
/// via `closeActive`, resetting the segment either way).
fn settle(state: &ParserState, at: u64, outcome: Option<TranscriptTurnOutcome>) -> ParserState {
    let (transcript, changed) = close_active(&state.transcript, at, outcome);
    let mut next = state.clone();
    if changed {
        next.transcript = transcript;
    }
    next.segment = SegmentState::default();
    next
}

// -- turn boundaries -----------------------------------------------------------

fn next_turn_index(t: &TranscriptSlice) -> u64 {
    t.committed.len() as u64 + u64::from(t.active.is_some())
}

fn next_turn_seq(t: &TranscriptSlice) -> u64 {
    t.committed.last().map_or(0, |last| last.seq + 1)
}

fn open_turn(
    mut t: TranscriptSlice,
    conversation_id: &str,
    initiator: TurnInitiator,
) -> TranscriptSlice {
    let id = make_turn_id(conversation_id, next_turn_index(&t));
    let turn = TranscriptTurn {
        id,
        seq: next_turn_seq(&t),
        initiator,
        items: Vec::new(),
        outcome: None,
    };
    t.active = Some(turn);
    t
}

/// Finalize and commit the active turn; `(slice, changed)` — unchanged when
/// there was no active turn (reference `closeActive`).
fn close_active(
    t: &TranscriptSlice,
    at: u64,
    outcome: Option<TranscriptTurnOutcome>,
) -> (TranscriptSlice, bool) {
    let Some(active) = &t.active else {
        return (t.clone(), false);
    };
    let mut committed_turn = active.clone();
    committed_turn.items = finalize_items(&active.items, at);
    if let Some(outcome) = outcome {
        committed_turn.outcome = Some(outcome);
    }
    let mut next = t.clone();
    next.committed.push(committed_turn);
    next.active = None;
    (next, true)
}

/// True when the incoming user message represents a NEW turn open (reference
/// `isNewUserMessage`). Uses the CURRENT active turn's id so the item lookup
/// matches what is already stored.
fn is_new_user_message(
    active: Option<&TranscriptTurn>,
    message_id: Option<&str>,
    segment: &SegmentState,
    transcript: &TranscriptSlice,
) -> bool {
    let Some(active) = active else {
        return true;
    };
    match message_id {
        None => {
            if segment.open == Some(SegmentKind::MessageUser) {
                return false;
            }
            transcript.active.as_ref().is_some_and(|t| {
                t.items.iter().any(|it| {
                    !matches!(it, TranscriptItem::Message(m) if m.role == crate::transcript::models::Role::User)
                })
            })
        }
        Some(message_id) => {
            let id = make_message_id(&active.id, message_id, "user");
            !active
                .items
                .iter()
                .any(|it| matches!(it, TranscriptItem::Message(m) if m.id == id))
        }
    }
}

// -- segmentation ----------------------------------------------------------------

fn segment_kind_of(event: &NormalizedEvent) -> SegmentKind {
    match event {
        NormalizedEvent::Thinking { .. } => SegmentKind::Thinking,
        NormalizedEvent::Message {
            role: MessageRole::User,
            ..
        } => SegmentKind::MessageUser,
        NormalizedEvent::Message { .. } => SegmentKind::MessageAssistant,
        _ => SegmentKind::MessageAssistant,
    }
}

fn synthesized_message_id(segment: &SegmentState, kind: SegmentKind) -> String {
    let stream = match kind {
        SegmentKind::MessageUser => "user",
        SegmentKind::MessageAssistant => "assistant",
        SegmentKind::Thinking => "thinking",
    };
    let index = match kind {
        SegmentKind::MessageUser => segment.user,
        SegmentKind::MessageAssistant => segment.assistant,
        SegmentKind::Thinking => segment.thinking,
    };
    format!("auto:{stream}:{index}")
}

/// Close the open synthesized segment (finalizes an open thinking row),
/// advancing the segment counter (reference `closeSynthesizedSegment`).
fn close_synthesized_segment(
    transcript: TranscriptSlice,
    segment: SegmentState,
    at: u64,
) -> (TranscriptSlice, SegmentState) {
    let Some(open_kind) = segment.open else {
        return (transcript, segment);
    };
    let message_id = synthesized_message_id(&segment, open_kind);
    let active = match &transcript.active {
        Some(active) => active,
        None => return (transcript, segment),
    };
    let item_id = match open_kind {
        SegmentKind::Thinking => make_thinking_id(&active.id, &message_id),
        _ => make_message_id(&active.id, &message_id, "segment"),
    };

    let mut next_segment = segment.clone();
    next_segment.open = None;
    match open_kind {
        SegmentKind::MessageUser => next_segment.user += 1,
        SegmentKind::MessageAssistant => next_segment.assistant += 1,
        SegmentKind::Thinking => next_segment.thinking += 1,
    }

    if open_kind != SegmentKind::Thinking {
        return (transcript, next_segment);
    }

    let mut changed = false;
    let items: Vec<TranscriptItem> = active
        .items
        .iter()
        .map(|item| match item {
            TranscriptItem::Thinking(t)
                if t.id == item_id && t.status == ThinkingStatus::Thinking =>
            {
                changed = true;
                TranscriptItem::Thinking(crate::transcript::models::TranscriptThinking {
                    status: ThinkingStatus::Done,
                    duration_ms: Some(at.saturating_sub(t.started_at)),
                    ..t.clone()
                })
            }
            other => other.clone(),
        })
        .collect();

    if !changed {
        return (transcript, next_segment);
    }
    let mut next_transcript = transcript;
    next_transcript.active.as_mut().unwrap().items = items;
    (next_transcript, next_segment)
}

/// Resolve a provider thinking messageId to the currently open segment id,
/// opening a `:segment:` suffixed continuation when the provider reuses an
/// already-finalized id (reference `resolveProviderThinkingMessageId`).
fn resolve_provider_thinking_message_id(active: &TranscriptTurn, message_id: &str) -> String {
    for item in active.items.iter().rev() {
        if let TranscriptItem::Thinking(t) = item {
            if t.status == ThinkingStatus::Thinking
                && (t.segment_id == message_id
                    || t.segment_id.starts_with(&format!("{message_id}:segment:")))
            {
                return t.segment_id.clone();
            }
        }
    }

    let base_id = make_thinking_id(&active.id, message_id);
    let base_done = active.items.iter().any(|item| {
        matches!(item, TranscriptItem::Thinking(t) if t.id == base_id && t.status == ThinkingStatus::Done)
    });
    if !base_done {
        return message_id.to_string();
    }

    let prefix = format!("{message_id}:segment:");
    let count = active
        .items
        .iter()
        .filter(|item| {
            matches!(item, TranscriptItem::Thinking(t) if t.segment_id.starts_with(prefix.as_str()))
        })
        .count();
    format!("{prefix}{}", count + 1)
}

struct Materialized {
    transcript: TranscriptSlice,
    segment: SegmentState,
    event: FoldEvent,
}

/// Guarantee every message/thinking event a stable id before folding
/// (reference `materializeEvent`).
fn materialize_event(
    transcript: TranscriptSlice,
    segment: SegmentState,
    event: &NormalizedEvent,
    at: u64,
) -> Materialized {
    match event {
        NormalizedEvent::Message {
            role,
            message_id,
            text,
        } => match message_id {
            None => {
                let kind = segment_kind_of(event);
                let (transcript, segment) = if segment.open == Some(kind) {
                    (transcript, segment)
                } else {
                    close_synthesized_segment(transcript, segment, at)
                };
                let message_id = synthesized_message_id(&segment, kind);
                let mut next_segment = segment;
                next_segment.open = Some(kind);
                Materialized {
                    transcript,
                    segment: next_segment,
                    event: FoldEvent::Message {
                        role: *role,
                        message_id,
                        text: text.clone(),
                    },
                }
            }
            Some(message_id) => {
                let (transcript, segment) = close_synthesized_segment(transcript, segment, at);
                Materialized {
                    transcript,
                    segment,
                    event: FoldEvent::Message {
                        role: *role,
                        message_id: message_id.clone(),
                        text: text.clone(),
                    },
                }
            }
        },
        NormalizedEvent::Thinking { message_id, text } => match message_id {
            None => {
                let kind = SegmentKind::Thinking;
                let (transcript, segment) = if segment.open == Some(kind) {
                    (transcript, segment)
                } else {
                    close_synthesized_segment(transcript, segment, at)
                };
                let message_id = synthesized_message_id(&segment, kind);
                let mut next_segment = segment;
                next_segment.open = Some(kind);
                Materialized {
                    transcript,
                    segment: next_segment,
                    event: FoldEvent::Thinking {
                        message_id,
                        text: text.clone(),
                    },
                }
            }
            Some(message_id) => {
                let (transcript, segment) = close_synthesized_segment(transcript, segment, at);
                let resolved = transcript
                    .active
                    .as_ref()
                    .map(|active| resolve_provider_thinking_message_id(active, message_id))
                    .unwrap_or_else(|| message_id.clone());
                Materialized {
                    transcript,
                    segment,
                    event: FoldEvent::Thinking {
                        message_id: resolved,
                        text: text.clone(),
                    },
                }
            }
        },
        NormalizedEvent::ToolCall {
            tool_call_id,
            title,
            tool_kind,
            status,
            parent_tool_call_id,
            diffs,
            input_summary,
            output_text,
            terminal_id,
        } => {
            let (transcript, segment) = close_synthesized_segment(transcript, segment, at);
            Materialized {
                transcript,
                segment,
                event: FoldEvent::ToolCall {
                    tool_call_id: tool_call_id.clone(),
                    title: title.clone(),
                    tool_kind: tool_kind.clone(),
                    status: *status,
                    parent_tool_call_id: parent_tool_call_id.clone(),
                    diffs: diffs.clone(),
                    input_summary: input_summary.clone(),
                    output_text: output_text.clone(),
                    terminal_id: terminal_id.clone(),
                },
            }
        }
        NormalizedEvent::ToolUpdate {
            tool_call_id,
            title,
            tool_kind,
            status,
            parent_tool_call_id,
            diffs,
            output_text,
            terminal_id,
        } => {
            let (transcript, segment) = close_synthesized_segment(transcript, segment, at);
            Materialized {
                transcript,
                segment,
                event: FoldEvent::ToolUpdate {
                    tool_call_id: tool_call_id.clone(),
                    title: title.clone(),
                    tool_kind: tool_kind.clone(),
                    status: *status,
                    parent_tool_call_id: parent_tool_call_id.clone(),
                    diffs: diffs.clone(),
                    output_text: output_text.clone(),
                    terminal_id: terminal_id.clone(),
                },
            }
        }
        NormalizedEvent::Plan { entries } => {
            let (transcript, segment) = close_synthesized_segment(transcript, segment, at);
            Materialized {
                transcript,
                segment,
                event: FoldEvent::Plan {
                    entries: entries.clone(),
                },
            }
        }
        // Meta/ignored events never reach here (routed above).
        _ => {
            let (transcript, segment) = close_synthesized_segment(transcript, segment, at);
            Materialized {
                transcript,
                segment,
                event: FoldEvent::Plan {
                    entries: Vec::new(),
                },
            }
        }
    }
}

// -- agent + plan slices ---------------------------------------------------------

/// Agent slice seam. Baseline events carry no subagent information, so this
/// currently only reacts to tool calls whose kind marks a spawned agent.
fn update_agent_slice(
    agents: &[AgentState],
    event: &NormalizedEvent,
    launch_turn_id: Option<&str>,
    at: u64,
) -> Vec<AgentState> {
    use crate::transcript::normalize::NormalizedToolStatus;

    let (tool_call_id, title, status) = match event {
        NormalizedEvent::ToolCall {
            tool_call_id,
            title,
            tool_kind,
            status,
            ..
        } if matches!(
            tool_kind.as_deref(),
            Some("subagent") | Some("task") | Some("agent")
        ) =>
        {
            (tool_call_id.clone(), Some(title.clone()), *status)
        }
        _ => return agents.to_vec(),
    };

    let agent_id = tool_call_id.clone();
    let mapped = match status {
        Some(NormalizedToolStatus::Completed) => crate::transcript::models::AgentStatus::Completed,
        Some(NormalizedToolStatus::Failed) => crate::transcript::models::AgentStatus::Failed,
        _ => crate::transcript::models::AgentStatus::Running,
    };
    let idx = agents
        .iter()
        .position(|a| a.agent_id == agent_id || a.tool_call_id == tool_call_id);

    let mut next = agents.to_vec();
    match idx {
        Some(i) => {
            next[i].status = mapped;
            if mapped != crate::transcript::models::AgentStatus::Running
                && next[i].completed_at.is_none()
            {
                next[i].completed_at = Some(at);
            }
        }
        None => next.push(AgentState {
            agent_id: agent_id.clone(),
            tool_call_id,
            launch_turn_id: launch_turn_id.map(str::to_string),
            name: title.unwrap_or(agent_id),
            status: mapped,
            started_at: at,
            completed_at: (mapped != crate::transcript::models::AgentStatus::Running).then_some(at),
            background: None,
            output_file: None,
            summary: None,
        }),
    }
    next
}

fn update_plan_slice(
    plan: Option<&PlanState>,
    event: &NormalizedEvent,
    at: u64,
) -> Option<PlanState> {
    let NormalizedEvent::Plan { entries } = event else {
        return plan.cloned();
    };
    Some(PlanState {
        id: SESSION_PLAN_ID.to_string(),
        entries: entries
            .iter()
            .enumerate()
            .map(|(index, entry)| PlanEntry {
                id: format!("{SESSION_PLAN_ID}:entry:{index}"),
                content: entry.content.clone(),
                status: entry.status,
                priority: entry.priority,
            })
            .collect(),
        updated_at: at,
    })
}

// -- invariants --------------------------------------------------------------------

/// Debug-only structural assertions (reference `assertTranscriptInvariants`):
/// sorted turn seqs, unique item ids, sorted sibling seqs, at most one open
/// thinking row per turn.
fn debug_assert_transcript(transcript: &TranscriptSlice) {
    if !cfg!(debug_assertions) {
        return;
    }
    let mut prev: Option<u64> = None;
    for turn in &transcript.committed {
        assert!(
            prev.is_none_or(|p| turn.seq > p),
            "committed turns are not sorted by seq"
        );
        prev = Some(turn.seq);
    }
    if let Some(active) = &transcript.active {
        assert!(
            prev.is_none_or(|p| active.seq > p),
            "active turn seq is not after history"
        );
    }
    for turn in transcript.committed.iter().chain(transcript.active.iter()) {
        assert_turn_invariants(turn);
    }
}

fn assert_turn_invariants(turn: &TranscriptTurn) {
    use std::collections::HashSet;
    let mut ids = HashSet::new();

    fn visit(item: &TranscriptItem, ids: &mut HashSet<String>) {
        assert!(
            ids.insert(item.id().to_string()),
            "duplicate item id '{}'",
            item.id()
        );
        match item {
            TranscriptItem::Tool(t) => {
                if let Some(children) = &t.children {
                    assert_sorted_siblings(children);
                    for child in children {
                        visit_node(child, ids);
                    }
                }
            }
            TranscriptItem::Group(g) => {
                assert_sorted_siblings(&g.children);
                for child in &g.children {
                    visit_node(child, ids);
                }
            }
            _ => {}
        }
    }

    fn visit_node(node: &crate::transcript::models::ToolNode, ids: &mut HashSet<String>) {
        match node {
            crate::transcript::models::ToolNode::Tool(t) => {
                visit(&TranscriptItem::Tool(t.clone()), ids)
            }
            crate::transcript::models::ToolNode::Group(g) => {
                visit(&TranscriptItem::Group(g.clone()), ids)
            }
        }
    }

    fn assert_sorted_siblings(nodes: &[crate::transcript::models::ToolNode]) {
        let mut prev = None;
        let mut seen = std::collections::HashSet::new();
        for node in nodes {
            let seq = match node {
                crate::transcript::models::ToolNode::Tool(t) => t.seq,
                crate::transcript::models::ToolNode::Group(g) => g.seq,
            };
            assert!(prev.is_none_or(|p| seq >= p), "siblings not sorted by seq");
            assert!(seen.insert(seq), "duplicate sibling seq {seq}");
            prev = Some(seq);
        }
    }

    let mut open_thinking = 0;
    let mut prev: Option<u64> = None;
    let mut seen_seqs = std::collections::HashSet::new();
    for item in &turn.items {
        let seq = item.seq();
        assert!(prev.is_none_or(|p| seq >= p), "items not sorted by seq");
        assert!(seen_seqs.insert(seq), "duplicate sibling seq {seq}");
        prev = Some(seq);
        visit(item, &mut ids);
        if let TranscriptItem::Thinking(t) = item {
            if t.status == ThinkingStatus::Thinking {
                open_thinking += 1;
            }
        }
    }
    assert!(open_thinking <= 1, "multiple open thinking rows");
}
