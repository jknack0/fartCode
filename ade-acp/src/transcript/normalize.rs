//! Normalized event vocabulary + baseline `SessionUpdate` decoder.
//!
//! Port of the reference `reducer/normalized-event.ts` + `reducer/decode.ts`:
//! stateless, pure decoding of raw ACP `session/update` notifications into
//! the reducer's internal vocabulary.
//!
//! Deviation: the reference's provider-enriched event kinds (`subagent`,
//! `search`, `mcp_tool`, `web_fetch`, `subagent_update`) only arrive via
//! vendor `_meta` enrich hooks; no enrich hooks exist in Phase 2, so they
//! are not modeled — [`NormalizedEvent`] carries exactly the baseline
//! decoder's output. `parentToolCallId` stays `None` until an enrich hook
//! lands.

use agent_client_protocol_schema::v1::{
    AvailableCommand, ContentBlock, SessionConfigOption, SessionUpdate, ToolCallContent,
};

use crate::transcript::models::{
    PlanEntryInput, PlanEntryPriority, PlanEntryStatus, SessionUsage, UsageCost,
};

/// One file diff extracted from tool-call content.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedDiff {
    pub path: String,
    pub old_text: Option<String>,
    pub new_text: String,
}

/// Tool status as reported by the provider (pre-mapping).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizedToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// The parser's internal event vocabulary (reference `NormalizedEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum NormalizedEvent {
    Message {
        role: MessageRole,
        /// Provider message id; `None` when omitted — the segmenter
        /// synthesizes a stable id before folding.
        message_id: Option<String>,
        text: String,
    },
    Thinking {
        message_id: Option<String>,
        text: String,
    },
    ToolCall {
        tool_call_id: String,
        title: String,
        /// ACP ToolKind passthrough (`edit`, `execute`, `read`, …).
        tool_kind: Option<String>,
        status: Option<NormalizedToolStatus>,
        /// Parent tool call id for nested calls; always `None` in the
        /// baseline (no enrich hook yet).
        parent_tool_call_id: Option<String>,
        diffs: Vec<NormalizedDiff>,
        input_summary: Option<String>,
        output_text: Option<String>,
        terminal_id: Option<String>,
    },
    ToolUpdate {
        tool_call_id: String,
        title: Option<String>,
        tool_kind: Option<String>,
        status: Option<NormalizedToolStatus>,
        parent_tool_call_id: Option<String>,
        diffs: Vec<NormalizedDiff>,
        output_text: Option<String>,
        terminal_id: Option<String>,
    },
    Plan {
        entries: Vec<PlanEntryInput>,
    },
    /// Full config option array from `config_option_update`.
    Config {
        options: Vec<SessionConfigOption>,
    },
    /// Selected mode id from `current_mode_update`.
    ModeSelected {
        mode_id: String,
    },
    /// Full available-command list from `available_commands_update`.
    Commands {
        commands: Vec<AvailableCommand>,
    },
    /// Usage figures from `usage_update`.
    Usage {
        usage: SessionUsage,
    },
    /// Session title from `session_info_update`.
    Title {
        title: String,
    },
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
}

/// Decode a raw ACP `SessionUpdate` into a [`NormalizedEvent`].
/// Stateless — does not depend on turn or session context.
pub fn decode_session_update(update: &SessionUpdate) -> NormalizedEvent {
    match update {
        SessionUpdate::UserMessageChunk(chunk) => {
            let Some(text) = text_of(&chunk.content) else {
                return NormalizedEvent::Ignored;
            };
            if text.is_empty() {
                return NormalizedEvent::Ignored;
            }
            NormalizedEvent::Message {
                role: MessageRole::User,
                message_id: chunk.message_id.as_ref().map(|m| m.0.to_string()),
                text,
            }
        }
        SessionUpdate::AgentMessageChunk(chunk) => {
            let Some(text) = text_of(&chunk.content) else {
                return NormalizedEvent::Ignored;
            };
            if text.is_empty() {
                return NormalizedEvent::Ignored;
            }
            NormalizedEvent::Message {
                role: MessageRole::Assistant,
                message_id: chunk.message_id.as_ref().map(|m| m.0.to_string()),
                text,
            }
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            let Some(text) = text_of(&chunk.content) else {
                return NormalizedEvent::Ignored;
            };
            if text.is_empty() {
                return NormalizedEvent::Ignored;
            }
            NormalizedEvent::Thinking {
                message_id: chunk.message_id.as_ref().map(|m| m.0.to_string()),
                text,
            }
        }
        SessionUpdate::ToolCall(call) => NormalizedEvent::ToolCall {
            tool_call_id: call.tool_call_id.0.to_string(),
            title: call.title.clone(),
            tool_kind: tool_kind_str(&call.kind),
            status: Some(status_of(call.status)),
            parent_tool_call_id: None,
            diffs: extract_diffs(&call.content),
            input_summary: None,
            output_text: None,
            terminal_id: None,
        },
        SessionUpdate::ToolCallUpdate(update) => {
            let content = update.fields.content.as_deref().unwrap_or_default();
            NormalizedEvent::ToolUpdate {
                tool_call_id: update.tool_call_id.0.to_string(),
                title: update.fields.title.clone(),
                tool_kind: update.fields.kind.as_ref().and_then(tool_kind_str),
                status: update.fields.status.map(status_of),
                parent_tool_call_id: None,
                diffs: extract_diffs(content),
                output_text: extract_text_output(content),
                terminal_id: extract_terminal_id(content),
            }
        }
        SessionUpdate::Plan(plan) => NormalizedEvent::Plan {
            entries: plan
                .entries
                .iter()
                .map(|e| PlanEntryInput {
                    content: e.content.clone(),
                    status: match e.status {
                        agent_client_protocol_schema::v1::PlanEntryStatus::Pending => {
                            PlanEntryStatus::Pending
                        }
                        agent_client_protocol_schema::v1::PlanEntryStatus::InProgress => {
                            PlanEntryStatus::InProgress
                        }
                        agent_client_protocol_schema::v1::PlanEntryStatus::Completed => {
                            PlanEntryStatus::Completed
                        }
                        _ => PlanEntryStatus::Pending,
                    },
                    priority: match e.priority {
                        agent_client_protocol_schema::v1::PlanEntryPriority::High => {
                            PlanEntryPriority::High
                        }
                        agent_client_protocol_schema::v1::PlanEntryPriority::Medium => {
                            PlanEntryPriority::Medium
                        }
                        agent_client_protocol_schema::v1::PlanEntryPriority::Low => {
                            PlanEntryPriority::Low
                        }
                        _ => PlanEntryPriority::Medium,
                    },
                })
                .collect(),
        },
        SessionUpdate::ConfigOptionUpdate(update) => NormalizedEvent::Config {
            options: update.config_options.clone(),
        },
        SessionUpdate::CurrentModeUpdate(update) => NormalizedEvent::ModeSelected {
            mode_id: update.current_mode_id.0.to_string(),
        },
        SessionUpdate::AvailableCommandsUpdate(update) => NormalizedEvent::Commands {
            commands: update.available_commands.clone(),
        },
        SessionUpdate::UsageUpdate(update) => NormalizedEvent::Usage {
            usage: SessionUsage {
                context_used: update.used,
                context_size: update.size,
                cost: update.cost.as_ref().map(|c| UsageCost {
                    amount: c.amount,
                    currency: c.currency.clone(),
                }),
            },
        },
        SessionUpdate::SessionInfoUpdate(update) => match update.title.value() {
            Some(title) => NormalizedEvent::Title {
                title: title.clone(),
            },
            None => NormalizedEvent::Ignored,
        },
        // plan_update / plan_removed are UNSTABLE id-based variants (not
        // emitted by current adapters) — ignored, same as the reference.
        _ => NormalizedEvent::Ignored,
    }
}

/// Text payload of a content block; non-text blocks are ignored (reference
/// decode behavior).
fn text_of(block: &ContentBlock) -> Option<String> {
    match block {
        ContentBlock::Text(t) => Some(t.text.clone()),
        _ => None,
    }
}

fn status_of(status: agent_client_protocol_schema::v1::ToolCallStatus) -> NormalizedToolStatus {
    use agent_client_protocol_schema::v1::ToolCallStatus as S;
    match status {
        S::Pending => NormalizedToolStatus::Pending,
        S::InProgress => NormalizedToolStatus::InProgress,
        S::Completed => NormalizedToolStatus::Completed,
        S::Failed => NormalizedToolStatus::Failed,
        _ => NormalizedToolStatus::Pending,
    }
}

/// ACP `ToolKind` → the string vocabulary the fold heuristics key on.
fn tool_kind_str(kind: &agent_client_protocol_schema::v1::ToolKind) -> Option<String> {
    use agent_client_protocol_schema::v1::ToolKind as K;
    Some(
        match kind {
            K::Read => "read",
            K::Edit => "edit",
            K::Delete => "delete",
            K::Move => "move",
            K::Search => "search",
            K::Execute => "execute",
            K::Think => "think",
            K::Fetch => "fetch",
            K::SwitchMode => "switch_mode",
            K::Other => return None,
            _ => return None,
        }
        .to_string(),
    )
}

fn extract_diffs(content: &[ToolCallContent]) -> Vec<NormalizedDiff> {
    content
        .iter()
        .filter_map(|block| match block {
            ToolCallContent::Diff(d) => Some(NormalizedDiff {
                path: d.path.to_string_lossy().into_owned(),
                old_text: d.old_text.clone(),
                new_text: d.new_text.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// Merged text output of `content`/`text` blocks, with a single wrapping
/// code fence stripped (reference `extractTextOutput`).
fn extract_text_output(content: &[ToolCallContent]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for block in content {
        if let ToolCallContent::Content(c) = block {
            if let ContentBlock::Text(t) = &c.content {
                parts.push(t.text.clone());
            }
        }
    }
    if parts.is_empty() {
        return None;
    }
    let text = parts.join("\n");
    if text.is_empty() {
        return None;
    }
    Some(strip_single_code_fence(&text))
}

/// Strips one wrapping ``` fence when the whole text is fenced.
fn strip_single_code_fence(text: &str) -> String {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return text.to_string();
    };
    let Some(newline) = rest.find('\n') else {
        return text.to_string();
    };
    let body = &rest[newline + 1..];
    if let Some(pos) = body.rfind("\n```") {
        let after = &body[pos + 4..];
        if after.trim().is_empty() {
            return body[..pos].to_string();
        }
    }
    text.to_string()
}

/// Terminal id embedded in tool-call content (reference `extractTerminalId`).
fn extract_terminal_id(content: &[ToolCallContent]) -> Option<String> {
    content.iter().find_map(|block| match block {
        ToolCallContent::Terminal(t) => Some(t.terminal_id.0.to_string()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn chunk_update(kind: &str, text: &str) -> SessionUpdate {
        serde_json::from_value(json!({
            "sessionUpdate": kind,
            "content": { "type": "text", "text": text },
        }))
        .unwrap()
    }

    #[test]
    fn decodes_message_and_thought_chunks() {
        let user = decode_session_update(&chunk_update("user_message_chunk", "hi"));
        assert!(matches!(
            user,
            NormalizedEvent::Message {
                role: MessageRole::User,
                text,
                ..
            } if text == "hi"
        ));

        let thought = decode_session_update(&chunk_update("agent_thought_chunk", "hm"));
        assert!(matches!(
            thought,
            NormalizedEvent::Thinking { text, .. } if text == "hm"
        ));

        let empty = decode_session_update(&chunk_update("agent_message_chunk", ""));
        assert_eq!(empty, NormalizedEvent::Ignored);
    }

    #[test]
    fn decodes_tool_call_with_diffs_and_status() {
        let update: SessionUpdate = serde_json::from_value(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "call_1",
            "title": "Edit foo.rs",
            "kind": "edit",
            "status": "in_progress",
            "content": [{
                "type": "diff",
                "path": "/repo/foo.rs",
                "oldText": "a",
                "newText": "b",
            }],
        }))
        .unwrap();
        let event = decode_session_update(&update);
        match event {
            NormalizedEvent::ToolCall {
                tool_call_id,
                title,
                tool_kind,
                status,
                diffs,
                ..
            } => {
                assert_eq!(tool_call_id, "call_1");
                assert_eq!(title, "Edit foo.rs");
                assert_eq!(tool_kind.as_deref(), Some("edit"));
                assert_eq!(status, Some(NormalizedToolStatus::InProgress));
                assert_eq!(diffs.len(), 1);
                assert_eq!(diffs[0].path, "/repo/foo.rs");
                assert_eq!(diffs[0].old_text.as_deref(), Some("a"));
            }
            other => panic!("expected tool_call, got {other:?}"),
        }
    }

    #[test]
    fn decodes_usage_title_commands_and_ignores_unknown() {
        let usage: SessionUpdate = serde_json::from_value(json!({
            "sessionUpdate": "usage_update",
            "used": 100,
            "size": 200000,
            "cost": { "amount": 1.5, "currency": "USD" },
        }))
        .unwrap();
        match decode_session_update(&usage) {
            NormalizedEvent::Usage { usage } => {
                assert_eq!(usage.context_used, 100);
                assert_eq!(usage.context_size, 200000);
                assert_eq!(usage.cost.as_ref().unwrap().currency, "USD");
            }
            other => panic!("expected usage, got {other:?}"),
        }

        let title: SessionUpdate = serde_json::from_value(json!({
            "sessionUpdate": "session_info_update",
            "title": "Fix the parser",
        }))
        .unwrap();
        match decode_session_update(&title) {
            NormalizedEvent::Title { title } => assert_eq!(title, "Fix the parser"),
            other => panic!("expected title, got {other:?}"),
        }

        let commands: SessionUpdate = serde_json::from_value(json!({
            "sessionUpdate": "available_commands_update",
            "availableCommands": [{ "name": "compact", "description": "Compact history" }],
        }))
        .unwrap();
        match decode_session_update(&commands) {
            NormalizedEvent::Commands { commands } => {
                assert_eq!(commands.len(), 1);
                assert_eq!(commands[0].name, "compact");
            }
            other => panic!("expected commands, got {other:?}"),
        }

        // Non-text content chunks are ignored.
        let img_update: SessionUpdate = serde_json::from_value(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "image", "data": "aGk=", "mimeType": "image/png" },
        }))
        .unwrap();
        assert_eq!(decode_session_update(&img_update), NormalizedEvent::Ignored);
    }

    #[test]
    fn strips_single_code_fence_from_tool_output() {
        assert_eq!(
            strip_single_code_fence("```rust\nfn main() {}\n```"),
            "fn main() {}"
        );
        assert_eq!(strip_single_code_fence("plain"), "plain");
        // The reference regex is end-anchored: it strips the FIRST fence line
        // and the LAST closing fence, so doubly-fenced text keeps the middle.
        assert_eq!(
            strip_single_code_fence("```a\nx\n```\n```b\ny\n```"),
            "x\n```\n```b\ny"
        );
    }
}
