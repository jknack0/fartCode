//! E2-11-4 acceptance: reducer golden tests.
//!
//! Deterministic, pure fold sequences — interleaved chunk streams
//! (message + thought + tool_call + plan) must produce the EXACT
//! transcript: turn ids, item ids, seqs, text, statuses, and the plan
//! slice. Timestamps are fixed so durations and ordering are pinned.

use ade_acp::transcript::models::{
    PlanEntryStatus, Role, ThinkingStatus, ToolItemKind, ToolStatus, TranscriptItem,
    TranscriptTurnOutcome,
};
use ade_acp::transcript::{MessageRole, NormalizedEvent, TranscriptParser};
use agent_client_protocol_schema::v1::SessionUpdate;
use serde_json::json;

const CONVERSATION: &str = "conv-golden";
const TURN_ID: &str = "conv-golden:turn:0";

fn update(value: serde_json::Value) -> SessionUpdate {
    serde_json::from_value(value).unwrap()
}

fn text_chunk(kind: &str, text: &str, message_id: Option<&str>) -> SessionUpdate {
    let mut value = json!({
        "sessionUpdate": kind,
        "content": { "type": "text", "text": text },
    });
    if let Some(id) = message_id {
        value["messageId"] = json!(id);
    }
    update(value)
}

// -- golden 1: interleaved message + thought + tool_call + plan ---------------

/// One turn interleaving every transcript item kind; the exact item list is
/// the golden expectation.
#[test]
fn interleaved_sequence_produces_the_exact_transcript() {
    let mut parser = TranscriptParser::new(CONVERSATION);

    // User message opens the turn.
    parser.push_event(
        NormalizedEvent::Message {
            role: MessageRole::User,
            message_id: Some("m-user".into()),
            text: "Do the work".into(),
        },
        1_000,
    );
    // Thought chunk (no provider id → synthesized segment).
    parser.push(&text_chunk("agent_thought_chunk", "Hmm. ", None), 1_010);
    parser.push(
        &text_chunk("agent_thought_chunk", "Let me check.", None),
        1_020,
    );
    // Agent message closes the thought segment.
    parser.push(
        &text_chunk("agent_message_chunk", "On it.", Some("m-agent")),
        1_050,
    );
    // Tool call lifecycle: pending → in_progress → completed.
    parser.push(
        &update(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "call_1",
            "title": "cargo test",
            "kind": "execute",
            "status": "pending",
        })),
        1_100,
    );
    parser.push(
        &update(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call_1",
            "status": "in_progress",
        })),
        1_150,
    );
    // Plan update mid-turn.
    parser.push(
        &update(json!({
            "sessionUpdate": "plan",
            "entries": [
                { "content": "step one", "priority": "high", "status": "completed" },
                { "content": "step two", "priority": "medium", "status": "in_progress" },
            ],
        })),
        1_200,
    );
    parser.push(
        &update(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call_1",
            "status": "completed",
            "content": [{
                "type": "content",
                "content": { "type": "text", "text": "ok" },
            }],
        })),
        1_250,
    );
    // Final agent chunk on the SAME message id appends.
    parser.push(
        &text_chunk("agent_message_chunk", " Done.", Some("m-agent")),
        1_300,
    );
    parser.settle_turn(TranscriptTurnOutcome::Done { reason: None }, 1_400);

    let committed = parser.history();
    assert_eq!(committed.len(), 1);
    let turn = &committed[0];
    assert_eq!(turn.id, TURN_ID);
    assert_eq!(turn.seq, 0);
    assert_eq!(turn.initiator, ade_acp::transcript::TurnInitiator::User);
    assert_eq!(
        turn.outcome,
        Some(TranscriptTurnOutcome::Done { reason: None })
    );

    // Exact item sequence: user message, thinking, assistant message,
    // execute tool, plan marker. Seqs are dense and ordered.
    let items = &turn.items;
    assert_eq!(items.len(), 5, "golden item count");
    let seqs: Vec<u64> = items.iter().map(|i| i.seq()).collect();
    assert_eq!(seqs, vec![0, 1, 2, 3, 4]);

    match &items[0] {
        TranscriptItem::Message(m) => {
            assert_eq!(m.id, format!("{TURN_ID}:message:m-user"));
            assert_eq!(m.role, Role::User);
            assert_eq!(m.text, "Do the work");
        }
        other => panic!("item 0 must be the user message, got {other:?}"),
    }
    match &items[1] {
        TranscriptItem::Thinking(t) => {
            assert_eq!(t.id, format!("{TURN_ID}:thinking:auto:thinking:0"));
            assert_eq!(t.segment_id, "auto:thinking:0");
            assert_eq!(t.text, "Hmm. Let me check.");
            assert_eq!(
                t.status,
                ThinkingStatus::Done,
                "thought finalized by the next content"
            );
            assert_eq!(t.started_at, 1_010);
            assert_eq!(
                t.duration_ms,
                Some(40),
                "closed at the agent message (1_050)"
            );
        }
        other => panic!("item 1 must be the thinking row, got {other:?}"),
    }
    match &items[2] {
        TranscriptItem::Message(m) => {
            assert_eq!(m.id, format!("{TURN_ID}:message:m-agent"));
            assert_eq!(m.role, Role::Assistant);
            assert_eq!(m.text, "On it. Done.", "same message id appends");
        }
        other => panic!("item 2 must be the assistant message, got {other:?}"),
    }
    match &items[3] {
        TranscriptItem::Tool(t) => {
            assert_eq!(t.id, format!("{TURN_ID}:tool:call_1"));
            assert_eq!(t.kind, ToolItemKind::ExecuteToolCall);
            assert_eq!(t.tool_call_id, "call_1");
            assert_eq!(t.title, "cargo test");
            assert_eq!(t.command.as_deref(), Some("cargo test"));
            assert_eq!(t.status, ToolStatus::Done);
            assert_eq!(t.output_text.as_deref(), Some("ok"));
        }
        other => panic!("item 3 must be the execute tool, got {other:?}"),
    }
    match &items[4] {
        TranscriptItem::Tool(t) => {
            assert_eq!(t.id, format!("{TURN_ID}:plan"));
            assert_eq!(t.kind, ToolItemKind::CreatePlanToolCall);
            assert_eq!(t.title, "Plan updated");
            assert_eq!(
                t.status,
                ToolStatus::Done,
                "finalizeItems settles running → done at commit"
            );
            assert_eq!(t.plan_id.as_deref(), Some("session-plan"));
        }
        other => panic!("item 4 must be the plan marker, got {other:?}"),
    }

    // Plan slice carries the full snapshot with synthesized entry ids.
    let plan = parser.plan().expect("plan slice");
    assert_eq!(plan.id, "session-plan");
    assert_eq!(plan.updated_at, 1_200);
    assert_eq!(plan.entries.len(), 2);
    assert_eq!(plan.entries[0].id, "session-plan:entry:0");
    assert_eq!(plan.entries[0].content, "step one");
    assert_eq!(plan.entries[0].status, PlanEntryStatus::Completed);
    assert_eq!(plan.entries[1].status, PlanEntryStatus::InProgress);
}

// -- golden 2: turn boundaries on new user messages ---------------------------

#[test]
fn new_user_message_id_opens_a_new_turn() {
    let mut parser = TranscriptParser::new(CONVERSATION);

    parser.push_event(
        NormalizedEvent::Message {
            role: MessageRole::User,
            message_id: Some("u1".into()),
            text: "first".into(),
        },
        10,
    );
    parser.push(&text_chunk("agent_message_chunk", "reply one", None), 20);
    parser.push_event(
        NormalizedEvent::Message {
            role: MessageRole::User,
            message_id: Some("u2".into()),
            text: "second".into(),
        },
        30,
    );
    parser.push(&text_chunk("agent_message_chunk", "reply two", None), 40);
    parser.end_turn(50);

    let committed = parser.history();
    assert_eq!(committed.len(), 2, "a new user messageId starts turn 2");
    assert_eq!(committed[0].id, format!("{CONVERSATION}:turn:0"));
    assert_eq!(committed[1].id, format!("{CONVERSATION}:turn:1"));
    assert_eq!(committed[0].seq, 0);
    assert_eq!(committed[1].seq, 1);
    assert_eq!(committed[0].items.len(), 2);
    assert_eq!(committed[1].items.len(), 2);

    // Repeating the SAME user messageId does not reopen.
    parser.push_event(
        NormalizedEvent::Message {
            role: MessageRole::User,
            message_id: Some("u3".into()),
            text: "third".into(),
        },
        60,
    );
    parser.push_event(
        NormalizedEvent::Message {
            role: MessageRole::User,
            message_id: Some("u3".into()),
            text: " chunk".into(),
        },
        61,
    );
    parser.end_turn(70);
    let committed = parser.history();
    assert_eq!(committed.len(), 3);
    let turn3 = &committed[2];
    assert_eq!(turn3.items.len(), 1, "same id appends to one user message");
    match &turn3.items[0] {
        TranscriptItem::Message(m) => assert_eq!(m.text, "third chunk"),
        other => panic!("expected message, got {other:?}"),
    }
}

// -- golden 3: lazy agent turn + replay settlement ------------------------------

#[test]
fn agent_content_without_user_message_lazily_opens_an_agent_turn() {
    let mut parser = TranscriptParser::new(CONVERSATION);
    parser.push(
        &text_chunk("agent_message_chunk", "background note", None),
        100,
    );
    let active = parser.active_turn().expect("lazy-open turn");
    assert_eq!(active.initiator, ade_acp::transcript::TurnInitiator::Agent);
    parser.end_replay(200);
    // end_replay commits the trailing turn (bounded replay closes at EOF).
    assert!(parser.active_turn().is_none());
    assert_eq!(parser.history().len(), 1);
    assert_eq!(
        parser.history()[0].initiator,
        ade_acp::transcript::TurnInitiator::Agent
    );
}

// -- golden 4: diff tool calls become file-operation items ----------------------

#[test]
fn diff_tool_calls_fold_into_file_operations() {
    let mut parser = TranscriptParser::new(CONVERSATION);
    parser.push_event(
        NormalizedEvent::Message {
            role: MessageRole::User,
            message_id: Some("u".into()),
            text: "edit it".into(),
        },
        10,
    );
    parser.push(
        &update(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "call_edit",
            "title": "Edit foo.rs",
            "kind": "edit",
            "status": "in_progress",
            "content": [{
                "type": "diff",
                "path": "/repo/foo.rs",
                "oldText": "let a = 1;",
                "newText": "let a = 2;",
            }],
        })),
        20,
    );
    parser.push(
        &update(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call_edit",
            "status": "completed",
        })),
        30,
    );
    parser.end_turn(40);

    let turn = &parser.history()[0];
    // user message + modify-file item (the edit-kind call with diffs folds
    // into the file operation, not a generic tool row).
    assert_eq!(turn.items.len(), 2);
    match &turn.items[1] {
        TranscriptItem::Tool(t) => {
            assert_eq!(t.kind, ToolItemKind::ModifyFileToolCall);
            assert_eq!(t.id, format!("{TURN_ID}:tool:call_edit:/repo/foo.rs"));
            assert_eq!(t.path.as_deref(), Some("/repo/foo.rs"));
            assert_eq!(t.old_text.as_deref(), Some("let a = 1;"));
            assert_eq!(t.new_text.as_deref(), Some("let a = 2;"));
            assert_eq!(
                t.status,
                ToolStatus::Done,
                "status propagated to the diff item"
            );
        }
        other => panic!("expected modify-file item, got {other:?}"),
    }
}

// -- golden 5: read-batch grouping ------------------------------------------------

#[test]
fn consecutive_reads_collapse_into_a_read_batch_group() {
    let mut parser = TranscriptParser::new(CONVERSATION);
    parser.push_event(
        NormalizedEvent::Message {
            role: MessageRole::User,
            message_id: Some("u".into()),
            text: "look".into(),
        },
        10,
    );
    for (i, path) in ["a.rs", "b.rs", "c.rs"].iter().enumerate() {
        parser.push(
            &update(json!({
                "sessionUpdate": "tool_call",
                "toolCallId": format!("read_{i}"),
                "title": format!("Read {path}"),
                "kind": "read",
                "status": "completed",
            })),
            20 + i as u64,
        );
    }
    parser.end_turn(50);

    let turn = &parser.history()[0];
    // user message + ONE group wrapping three reads.
    assert_eq!(turn.items.len(), 2);
    match &turn.items[1] {
        TranscriptItem::Group(g) => {
            assert_eq!(g.group_kind, "read-batch");
            assert_eq!(g.label, "3 file reads");
            assert_eq!(g.status, ToolStatus::Done);
            assert_eq!(g.children.len(), 3);
        }
        other => panic!("expected a read-batch group, got {other:?}"),
    }
}

// -- golden 6: config/mode/usage/title slices -------------------------------------

#[test]
fn meta_slices_land_on_the_parser_state() {
    let mut parser = TranscriptParser::new(CONVERSATION);

    parser.push(
        &update(json!({
            "sessionUpdate": "config_option_update",
            "configOptions": [
                {
                    "id": "model", "name": "Model", "category": "model",
                    "type": "select", "currentValue": "m1",
                    "options": [{ "value": "m1", "name": "Model One" }],
                },
                {
                    "id": "mode", "name": "Mode", "category": "mode",
                    "type": "select", "currentValue": "default",
                    "options": [{ "value": "default", "name": "Default" }],
                },
            ],
        })),
        10,
    );
    // Mode selection before modeOptions exist → pending, applied later.
    parser.push(
        &update(json!({
            "sessionUpdate": "current_mode_update",
            "currentModeId": "default",
        })),
        20,
    );
    parser.push(
        &update(json!({
            "sessionUpdate": "available_commands_update",
            "availableCommands": [
                { "name": "compact", "description": "Compact the history",
                  "input": { "hint": "optional focus" } },
            ],
        })),
        30,
    );
    parser.push(
        &update(json!({
            "sessionUpdate": "usage_update",
            "used": 500, "size": 100000,
            "cost": { "amount": 0.25, "currency": "USD" },
        })),
        40,
    );
    parser.push(
        &update(json!({
            "sessionUpdate": "session_info_update",
            "title": "My session",
        })),
        50,
    );

    let config = parser.config();
    assert_eq!(
        config.model_options.as_ref().unwrap().selected.as_deref(),
        Some("m1")
    );
    assert_eq!(
        config.mode_options.as_ref().unwrap().selected.as_deref(),
        Some("default")
    );
    assert_eq!(config.available_commands.len(), 1);
    assert_eq!(config.available_commands[0].name, "compact");
    assert_eq!(
        config.available_commands[0].input_hint.as_deref(),
        Some("optional focus")
    );

    let usage = parser.usage().unwrap();
    assert_eq!(usage.context_used, 500);
    assert_eq!(usage.context_size, 100000);
    assert_eq!(usage.cost.as_ref().unwrap().currency, "USD");

    assert_eq!(parser.title(), Some("My session"));
    // No transcript items were created by meta events.
    assert!(parser.history().is_empty());
    assert!(parser.active_turn().is_none());
}
