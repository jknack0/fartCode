//! Live-model types produced by the transcript reducer (ticket E2-11-4).
//!
//! Rust port of the reference `packages/core/src/acp/models/*` shapes that
//! the reducer owns: transcript turns/items, config state, usage, plan, and
//! agent slices. Everything here is serde-serializable (camelCase) so the
//! app layer can hand it to the frontend verbatim.
//!
//! One deliberate deviation: the reference models tool calls as an 11-way
//! discriminated union of interfaces; here [`ToolCallItem`] is a single
//! struct whose `kind` field carries the discriminator and whose
//! kind-specific fields are optional. The serialized shape is identical.

use serde::Serialize;

// -- transcript turns --------------------------------------------------------

/// Who opened a turn (reference `TranscriptTurnInitiator`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnInitiator {
    User,
    Agent,
}

/// Successful turn reasons: ACP stop reasons plus runtime quiescence
/// (reference `DoneTurnReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoneTurnReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    Quiesced,
}

/// Runtime-normalized failure categories (reference `ErrorTurnReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorTurnReason {
    PromptFailed,
    ProcessClosed,
    SpawnFailed,
    InitializeFailed,
    NewSessionFailed,
    LoadSessionFailed,
    CancelFailed,
    SetConfigFailed,
    SetModeFailed,
}

/// Non-error interruption reasons (reference `InterruptedTurnReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptedTurnReason {
    ProcessClosed,
    Replaced,
}

/// Durable settlement for a whole turn (reference
/// `TranscriptTurnOutcome`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TranscriptTurnOutcome {
    Done {
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<DoneTurnReason>,
    },
    Cancelled {
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<ErrorTurnReason>,
    },
    Interrupted {
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<InterruptedTurnReason>,
    },
}

/// One reduced exchange (reference `TranscriptTurn`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptTurn {
    /// Reducer-generated turn id scoping every item id in this exchange.
    pub id: String,
    /// Stable session order, assigned when the turn opens.
    pub seq: u64,
    /// Who opened the turn.
    pub initiator: TurnInitiator,
    pub items: Vec<TranscriptItem>,
    /// Settlement; absent while the turn is in flight.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<TranscriptTurnOutcome>,
}

// -- transcript items --------------------------------------------------------

/// One renderable row in a turn (reference `TranscriptItem` union).
///
/// The `Tool`/`Group` variants are large on purpose — they are flat data
/// payloads cloned into snapshots; boxing would add a heap allocation per
/// item on the hot fold path for no measurable win at Phase-2 transcript
/// sizes.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TranscriptItem {
    Message(TranscriptMessage),
    Thinking(TranscriptThinking),
    Tool(ToolCallItem),
    Group(ToolGroup),
}

impl TranscriptItem {
    pub fn id(&self) -> &str {
        match self {
            Self::Message(m) => &m.id,
            Self::Thinking(t) => &t.id,
            Self::Tool(t) => &t.id,
            Self::Group(g) => &g.id,
        }
    }

    pub fn seq(&self) -> u64 {
        match self {
            Self::Message(m) => m.seq,
            Self::Thinking(t) => t.seq,
            Self::Tool(t) => t.seq,
            Self::Group(g) => g.seq,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MessageKindTag {
    Message,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ThinkingKindTag {
    Thinking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum GroupKindTag {
    #[serde(rename = "tool-group")]
    ToolGroup,
}

/// A user or assistant message (reference `TranscriptMessage`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptMessage {
    pub(crate) kind: MessageKindTag,
    /// Provider message id scoped to the turn, or reducer-synthesized id.
    pub id: String,
    /// Stable order within the owning turn.
    pub seq: u64,
    pub role: Role,
    pub text: String,
}

impl TranscriptMessage {
    pub fn new(id: String, seq: u64, role: Role, text: String) -> Self {
        Self {
            kind: MessageKindTag::Message,
            id,
            seq,
            role,
            text,
        }
    }
}

/// Message author (reference role literal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    User,
    Assistant,
}

/// Streaming reasoning row (reference `TranscriptThinking`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptThinking {
    pub(crate) kind: ThinkingKindTag,
    pub id: String,
    pub seq: u64,
    /// Provider or synthesized stream segment id for merging chunks.
    pub segment_id: String,
    pub text: String,
    pub status: ThinkingStatus,
    /// Epoch ms when the row opened.
    pub started_at: u64,
    /// Frozen duration once finalized.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl TranscriptThinking {
    pub fn new(id: String, seq: u64, segment_id: String, text: String, started_at: u64) -> Self {
        Self {
            kind: ThinkingKindTag::Thinking,
            id,
            seq,
            segment_id,
            text,
            status: ThinkingStatus::Thinking,
            started_at,
            duration_ms: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ThinkingStatus {
    Thinking,
    Done,
}

/// Tool status as displayed (reference `ToolStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolStatus {
    Running,
    Done,
    Error,
}

/// Tool call item discriminator (reference kebab-case `kind` literals).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ToolItemKind {
    #[serde(rename = "execute-tool-call")]
    ExecuteToolCall,
    #[serde(rename = "read-tool-call")]
    ReadToolCall,
    #[serde(rename = "create-file-tool-call")]
    CreateFileToolCall,
    #[serde(rename = "modify-file-tool-call")]
    ModifyFileToolCall,
    #[serde(rename = "delete-file-tool-call")]
    DeleteFileToolCall,
    #[serde(rename = "search-tool-call")]
    SearchToolCall,
    #[serde(rename = "mcp-tool-call")]
    McpToolCall,
    #[serde(rename = "web-fetch-tool-call")]
    WebFetchToolCall,
    #[serde(rename = "spawn-subagent-tool-call")]
    SpawnSubagentToolCall,
    #[serde(rename = "create-plan-tool-call")]
    CreatePlanToolCall,
    #[serde(rename = "unknown-tool-call")]
    UnknownToolCall,
}

/// One tool call row (reference `ToolCallItem` union, flattened — see module
/// docs). Kind-specific fields are `None` unless the kind carries them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallItem {
    pub kind: ToolItemKind,
    pub id: String,
    pub seq: u64,
    pub tool_call_id: String,
    pub title: String,
    pub status: ToolStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<ToolNode>>,
    // execute
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_id: Option<String>,
    // read / create-file / modify-file / delete-file
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_text: Option<String>,
    // search
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_count: Option<u64>,
    // mcp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    // web fetch
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_title: Option<String>,
    // subagent / unknown
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
    // plan
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    // unknown
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_kind: Option<String>,
}

impl ToolCallItem {
    /// Base fields every kind shares; kind-specific fields start `None`.
    /// (`input_summary` is set by the caller when a provider supplies it.)
    pub fn base(
        kind: ToolItemKind,
        id: String,
        seq: u64,
        tool_call_id: String,
        title: String,
        status: ToolStatus,
        parent_tool_call_id: Option<String>,
    ) -> Self {
        Self {
            kind,
            id,
            seq,
            tool_call_id,
            title,
            status,
            input_summary: None,
            parent_tool_call_id,
            children: None,
            command: None,
            output_text: None,
            terminal_id: None,
            path: None,
            content: None,
            old_text: None,
            new_text: None,
            query: None,
            match_count: None,
            tool: None,
            server: None,
            url: None,
            page_title: None,
            name: None,
            agent_id: None,
            background: None,
            plan_id: None,
            tool_kind: None,
        }
    }
}

/// A tool node in a nested tree (reference `ToolNode`). Same size
/// rationale as [`TranscriptItem`].
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ToolNode {
    Tool(ToolCallItem),
    Group(ToolGroup),
}

impl ToolNode {
    pub fn status(&self) -> ToolStatus {
        match self {
            Self::Tool(t) => t.status,
            Self::Group(g) => g.status,
        }
    }
}

/// Collapsed batch of sibling reads (reference `ToolGroup`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolGroup {
    pub(crate) kind: GroupKindTag,
    pub id: String,
    pub seq: u64,
    pub label: String,
    pub group_kind: String,
    pub status: ToolStatus,
    pub children: Vec<ToolNode>,
}

impl ToolGroup {
    pub fn read_batch(
        id: String,
        seq: u64,
        label: String,
        status: ToolStatus,
        children: Vec<ToolNode>,
    ) -> Self {
        Self {
            kind: GroupKindTag::ToolGroup,
            id,
            seq,
            label,
            group_kind: "read-batch".to_string(),
            status,
            children,
        }
    }
}

// -- live model slices -------------------------------------------------------

/// Session config selectors derived from `config_option_update`
/// (reference `SessionConfigState`, minus the model `features` metadata the
/// current ACP stream never carries).
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfigState {
    pub model_options: Option<SelectGroup>,
    pub efforts: Option<SelectGroup>,
    pub mode_options: Option<SelectGroup>,
    pub available_commands: Vec<SessionCommand>,
}

/// One selector group (reference `modelOptions`/`efforts`/`modeOptions`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectGroup {
    /// Provider-owned ACP config option id used when sending updates.
    pub config_id: String,
    pub selected: Option<String>,
    pub available: Vec<ConfigChoice>,
}

/// One selectable value (reference `ModelChoice`/`EffortOption`/
/// `ModeOption`, unified).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigChoice {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A provider-advertised slash command (reference `SessionCommand`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCommand {
    pub name: String,
    pub description: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_hint: Option<String>,
}

/// Context-window usage (reference `SessionUsage`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUsage {
    pub context_size: u64,
    pub context_used: u64,
    pub cost: Option<UsageCost>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCost {
    pub amount: f64,
    pub currency: String,
}

/// Session-scoped plan slice (reference `PlanState`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanState {
    pub id: String,
    /// Latest full snapshot; providers replace the whole list each update.
    pub entries: Vec<PlanEntry>,
    /// Epoch ms of the last plan update.
    pub updated_at: u64,
}

/// Stable id of the session-scoped plan slice.
pub const SESSION_PLAN_ID: &str = "session-plan";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    /// Reducer-synthesized id (ACP plan updates carry no stable ids).
    pub id: String,
    pub content: String,
    pub status: PlanEntryStatus,
    pub priority: PlanEntryPriority,
}

/// Raw provider plan entry before the reducer assigns an id.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanEntryInput {
    pub content: String,
    pub status: PlanEntryStatus,
    pub priority: PlanEntryPriority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanEntryStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanEntryPriority {
    High,
    Medium,
    Low,
}

/// A spawned (sub)agent tracked by the session (reference `AgentState`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentState {
    pub agent_id: String,
    pub tool_call_id: String,
    /// Turn that launched the agent; `None` for orphan updates.
    pub launch_turn_id: Option<String>,
    pub name: String,
    pub status: AgentStatus,
    /// Epoch ms when first observed.
    pub started_at: u64,
    /// Epoch ms when a terminal status was reached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentStatus {
    Running,
    Completed,
    Failed,
}
