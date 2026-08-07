//! E2-11-4 acceptance: browser-free event tests — updates emitted for a
//! fake-adapter session arrive in order, and the live-model snapshot
//! (`acp:transcript` payload) reflects the reduced transcript + slices.
//!
//! The [`RecordingEvents`] sink captures every event the cell fires, in
//! firing order, with no frontend in the loop.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol_schema::v1::{SessionId, SessionUpdate};
use fartcode_acp::client::SessionUpdateEvent;
use fartcode_acp::session::{
    LiveModels, PermissionRequestedEvent, SessionEvents, SessionManager, StartInput,
};
use fartcode_acp::transcript::models::{ThinkingStatus, ToolItemKind, ToolStatus};
use parking_lot::Mutex;

fn adapter_path() -> PathBuf {
    env!("CARGO_BIN_EXE_fake_acp_adapter").into()
}

#[derive(Default)]
struct RecordingEvents {
    updates: Mutex<Vec<SessionUpdateEvent>>,
    snapshots: Mutex<Vec<LiveModels>>,
    permissions: Mutex<Vec<PermissionRequestedEvent>>,
}

impl SessionEvents for RecordingEvents {
    fn update(&self, _conversation_id: &str, event: &SessionUpdateEvent) {
        self.updates.lock().push(event.clone());
    }

    fn transcript_changed(&self, _conversation_id: &str, models: &LiveModels) {
        self.snapshots.lock().push(models.clone());
    }

    fn permission_requested(&self, _conversation_id: &str, event: &PermissionRequestedEvent) {
        self.permissions.lock().push(event.clone());
    }
}

struct Harness {
    manager: Arc<SessionManager>,
    events: Arc<RecordingEvents>,
}

async fn harness(conversation_id: &str, cwd: &Path) -> (Harness, Arc<fartcode_acp::AcpClient>) {
    let manager = Arc::new(SessionManager::new(None));
    let events = Arc::new(RecordingEvents::default());
    let resolver =
        SessionManager::permission_resolver(Arc::downgrade(&manager), conversation_id.to_string());
    let client = Arc::new(
        fartcode_acp::AcpClient::spawn(
            adapter_path(),
            &[],
            &[],
            Some(cwd),
            Some(cwd.to_path_buf()),
            resolver,
        )
        .await
        .unwrap(),
    );
    client.initialize().await.unwrap();
    let loop_client = Arc::clone(&client);
    tokio::spawn(async move { loop_client.run_event_loop().await });
    (Harness { manager, events }, client)
}

fn start_input(
    conversation_id: &str,
    cwd: &Path,
    session_id: Option<SessionId>,
    client: Arc<fartcode_acp::AcpClient>,
    events: Arc<RecordingEvents>,
) -> StartInput {
    StartInput {
        conversation_id: conversation_id.to_string(),
        provider_id: "fake".to_string(),
        cwd: cwd.to_path_buf(),
        session_id,
        supports_load_session: true,
        initial_queue: Vec::new(),
        events: Some(events),
        client,
    }
}

async fn eventually(f: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !f() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition never became true"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// -- acceptance: updates arrive in order --------------------------------------

/// The `rich` prompt streams thought → message → tool_call → plan →
/// config → usage → title → tool_call_update → message. Every one must
/// reach the event sink in that exact order.
#[tokio::test]
async fn rich_turn_updates_arrive_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let (h, client) = harness("conv-events", dir.path()).await;
    h.manager
        .start(start_input(
            "conv-events",
            dir.path(),
            None,
            client,
            h.events.clone(),
        ))
        .await
        .unwrap();

    h.manager.prompt("conv-events", "rich", None).await.unwrap();

    let updates = h.events.updates.lock().clone();
    let kinds: Vec<&'static str> = updates
        .iter()
        .map(|u| match &u.update {
            SessionUpdate::AgentThoughtChunk(_) => "thought",
            SessionUpdate::AgentMessageChunk(_) => "message",
            SessionUpdate::ToolCall(_) => "tool_call",
            SessionUpdate::Plan(_) => "plan",
            SessionUpdate::ConfigOptionUpdate(_) => "config",
            SessionUpdate::UsageUpdate(_) => "usage",
            SessionUpdate::SessionInfoUpdate(_) => "title",
            SessionUpdate::ToolCallUpdate(_) => "tool_call_update",
            _ => "other",
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "thought",
            "message",
            "tool_call",
            "plan",
            "config",
            "usage",
            "title",
            "tool_call_update",
            "message",
        ],
        "updates must reach the sink in arrival order"
    );
}

// -- acceptance: live models reflect the reduction ------------------------------

#[tokio::test]
async fn live_models_snapshot_reduces_the_rich_turn() {
    let dir = tempfile::tempdir().unwrap();
    let (h, client) = harness("conv-models", dir.path()).await;
    h.manager
        .start(start_input(
            "conv-models",
            dir.path(),
            None,
            client,
            h.events.clone(),
        ))
        .await
        .unwrap();

    h.manager.prompt("conv-models", "rich", None).await.unwrap();

    let models = h.manager.live_models("conv-models").unwrap();

    // One committed turn, no active turn.
    assert!(models.active_turn.is_none());
    assert_eq!(models.committed.len(), 1);
    let turn = &models.committed[0];

    // Items: synthesized user message, thinking (finalized), assistant
    // message, execute tool call (done, with output), plan marker.
    let items = &turn.items;
    assert!(
        items.len() >= 5,
        "expected user message + thinking + agent message + tool + plan, got {items:?}"
    );
    // First item is the injected user message.
    match &items[0] {
        fartcode_acp::transcript::TranscriptItem::Message(m) => {
            assert_eq!(m.role, fartcode_acp::transcript::Role::User);
            assert_eq!(m.text, "rich");
        }
        other => panic!("first item must be the user message, got {other:?}"),
    }
    // Thinking row is finalized.
    let thinking = items.iter().find_map(|i| match i {
        fartcode_acp::transcript::TranscriptItem::Thinking(t) => Some(t),
        _ => None,
    });
    let thinking = thinking.expect("a thinking row must exist");
    assert_eq!(thinking.status, ThinkingStatus::Done);
    assert!(thinking.text.starts_with("Let me think."));
    // Tool call completed with output.
    let tool = items.iter().find_map(|i| match i {
        fartcode_acp::transcript::TranscriptItem::Tool(t)
            if t.kind == ToolItemKind::ExecuteToolCall =>
        {
            Some(t)
        }
        _ => None,
    });
    let tool = tool.expect("an execute tool call must exist");
    assert_eq!(tool.status, ToolStatus::Done);
    assert_eq!(tool.output_text.as_deref(), Some("all tests passed"));
    // Plan marker present.
    let plan_item = items.iter().find_map(|i| match i {
        fartcode_acp::transcript::TranscriptItem::Tool(t)
            if t.kind == ToolItemKind::CreatePlanToolCall =>
        {
            Some(t)
        }
        _ => None,
    });
    assert!(plan_item.is_some(), "plan marker must exist");

    // Slices: config, usage, title, plan.
    let config = &models.config;
    let model_options = config.model_options.as_ref().expect("model selector");
    assert_eq!(model_options.selected.as_deref(), Some("fake-1"));
    assert_eq!(model_options.available.len(), 2);
    let usage = models.usage.as_ref().expect("usage");
    assert_eq!(usage.context_used, 1234);
    assert_eq!(usage.context_size, 200000);
    assert_eq!(models.title.as_deref(), Some("Rich turn"));
    let plan = models.plan.as_ref().expect("plan slice");
    assert_eq!(plan.entries.len(), 2);
    assert_eq!(
        plan.entries[1].status,
        fartcode_acp::transcript::PlanEntryStatus::InProgress
    );

    // Every snapshot fired through the sink; the last one is terminal.
    let snapshots = h.events.snapshots.lock().clone();
    assert!(!snapshots.is_empty());
    let last = snapshots.last().unwrap();
    assert!(last.active_turn.is_none());
    assert_eq!(last.committed.len(), 1);
}

// -- acceptance: replayed history reduces into committed turns --------------------

#[tokio::test]
async fn resumed_session_replay_settles_as_committed_turns() {
    let dir = tempfile::tempdir().unwrap();
    let (h, client) = harness("conv-replay", dir.path()).await;
    // The fake adapter replays two updates (user + agent message) on load.
    let outcome = h
        .manager
        .start(start_input(
            "conv-replay",
            dir.path(),
            Some(SessionId::new("fake-session-1")),
            client,
            h.events.clone(),
        ))
        .await
        .unwrap();
    assert!(outcome.resumed);

    let models = h.manager.live_models("conv-replay").unwrap();
    assert!(models.active_turn.is_none(), "replay settles at EOF");
    assert_eq!(
        models.committed.len(),
        1,
        "two replayed messages form one turn"
    );
    let turn = &models.committed[0];
    let texts: Vec<&str> = turn
        .items
        .iter()
        .filter_map(|i| match i {
            fartcode_acp::transcript::TranscriptItem::Message(m) => Some(m.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["earlier user message", "earlier agent reply"]);
}

// -- acceptance: permission request event fires -----------------------------------

#[tokio::test]
async fn permission_request_fires_event() {
    let dir = tempfile::tempdir().unwrap();
    let (h, client) = harness("conv-perm", dir.path()).await;
    h.manager
        .start(start_input(
            "conv-perm",
            dir.path(),
            None,
            client,
            h.events.clone(),
        ))
        .await
        .unwrap();

    let manager = Arc::clone(&h.manager);
    let turn_task =
        tokio::spawn(async move { manager.prompt("conv-perm", "tool time", None).await });

    eventually(|| !h.events.permissions.lock().is_empty()).await;
    let event = h.events.permissions.lock()[0].clone();
    assert_eq!(event.pending.options.len(), 2);

    h.manager
        .resolve_permission("conv-perm", &event.request_id, "allow-once")
        .unwrap();
    turn_task.await.unwrap().unwrap();
}

// -- raw log export ----------------------------------------------------------------

#[tokio::test]
async fn raw_log_export_captures_the_turn_traffic() {
    let dir = tempfile::tempdir().unwrap();
    let (h, client) = harness("conv-log", dir.path()).await;
    h.manager
        .start(start_input(
            "conv-log",
            dir.path(),
            None,
            client,
            h.events.clone(),
        ))
        .await
        .unwrap();
    h.manager.prompt("conv-log", "hello", None).await.unwrap();

    let export: serde_json::Value =
        serde_json::from_str(&h.manager.export_raw_log("conv-log").unwrap()).unwrap();
    assert_eq!(export["meta"]["conversationId"], "conv-log");
    assert_eq!(export["meta"]["providerId"], "fake");
    assert_eq!(export["meta"]["acpSessionId"], "fake-session-1");
    let kinds: Vec<&str> = export["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event"]["kind"].as_str().unwrap())
        .collect();
    // prompt → (session_update × 2) → prompt_result, in order.
    assert_eq!(kinds.first(), Some(&"prompt"));
    assert_eq!(kinds.last(), Some(&"prompt_result"));
    assert_eq!(
        kinds.iter().filter(|k| *k == &"session_update").count(),
        2,
        "two agent message chunks"
    );
}
