//! Deterministic, stable id synthesis for transcript items.
//!
//! Direct port of the reference `reducer/ids.ts` — the same string formats,
//! so render identity survives a future migration.

/// `${conversationId}:turn:${turnIndex}` — 0-based position in the session.
pub fn make_turn_id(conversation_id: &str, turn_index: u64) -> String {
    format!("{conversation_id}:turn:{turn_index}")
}

/// `${turnId}:message:${messageId}` — provider id or synthesized segment id.
pub fn make_message_id(turn_id: &str, message_id: &str, _role: &str) -> String {
    format!("{turn_id}:message:{message_id}")
}

/// `${turnId}:thinking:${messageId}` — kind-tag prefix disambiguates from
/// messages when a provider reuses a messageId across both kinds.
pub fn make_thinking_id(turn_id: &str, message_id: &str) -> String {
    format!("{turn_id}:thinking:{message_id}")
}

/// `${turnId}:tool:${toolCallId}`.
pub fn make_tool_id(turn_id: &str, tool_call_id: &str) -> String {
    format!("{turn_id}:tool:{tool_call_id}")
}

/// `${firstChildId}:group`.
pub fn make_tool_group_id(first_child_id: &str) -> String {
    format!("{first_child_id}:group")
}

/// Parent item id for nested tool calls, scoped to the same turn.
pub fn make_parent_id(turn_id: &str, parent_tool_call_id: Option<&str>) -> Option<String> {
    parent_tool_call_id.map(|p| make_tool_id(turn_id, p))
}

/// `${toolId}:${path}` — one diff item per changed file per tool call.
pub fn make_diff_id(tool_id: &str, path: &str) -> String {
    format!("{tool_id}:{path}")
}

/// `${turnId}:plan` — one plan per turn.
pub fn make_plan_id(turn_id: &str) -> String {
    format!("{turn_id}:plan")
}
