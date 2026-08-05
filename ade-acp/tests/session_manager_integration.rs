//! E2-11-3 acceptance (kept green under the E2-11-4 reduced transcript):
//! SessionManager + SessionCell against the fake adapter binary.
//!
//! Covers: start → prompt → stop with session-id persistence and restart
//! resume; queue ordering + edit/remove/reorder while idle between turns;
//! permission routing through the cell; unknown-conversation errors.
//! E2-11-4 changed the history shape: turns are now reduced
//! `TranscriptTurn`s (no raw update stream; the prompt text is the
//! synthetic user-message item).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ade_acp::session::{SessionIdStore, SessionManager, StartInput};
use ade_acp::{Error, Lifecycle, QueuedPrompt};
use agent_client_protocol_schema::v1::SessionId;
use parking_lot::Mutex;

fn adapter_path() -> PathBuf {
    env!("CARGO_BIN_EXE_fake_acp_adapter").into()
}

/// In-memory `SessionIdStore` standing in for the conversations table
/// (the real adapter wires `DbConversationStore::set_session_id` at the
/// app layer, #32).
#[derive(Default)]
struct MemoryStore {
    ids: Mutex<HashMap<String, String>>,
}

impl SessionIdStore for MemoryStore {
    fn set_session_id(&self, conversation_id: &str, session_id: &str) -> Result<(), String> {
        self.ids
            .lock()
            .insert(conversation_id.to_string(), session_id.to_string());
        Ok(())
    }
}

impl MemoryStore {
    fn get(&self, conversation_id: &str) -> Option<String> {
        self.ids.lock().get(conversation_id).cloned()
    }
}

struct Harness {
    manager: Arc<SessionManager>,
    store: Arc<MemoryStore>,
}

/// Spawns a fresh client wired to the manager's permission router, plus the
/// event loop, and hands back (manager, store, client) ready for `start`.
async fn harness(conversation_id: &str, cwd: &Path) -> (Harness, Arc<ade_acp::AcpClient>) {
    let store = Arc::new(MemoryStore::default());
    let manager = Arc::new(SessionManager::new(Some(store.clone())));
    let resolver =
        SessionManager::permission_resolver(Arc::downgrade(&manager), conversation_id.to_string());
    let client = Arc::new(
        ade_acp::AcpClient::spawn(
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
    (Harness { manager, store }, client)
}

fn start_input(
    conversation_id: &str,
    cwd: &Path,
    session_id: Option<SessionId>,
    client: Arc<ade_acp::AcpClient>,
) -> StartInput {
    StartInput {
        conversation_id: conversation_id.to_string(),
        provider_id: "fake".to_string(),
        cwd: cwd.to_path_buf(),
        session_id,
        supports_load_session: true,
        initial_queue: Vec::new(),
        events: None,
        client,
    }
}

/// Polls `f` until it returns true (or the deadline passes).
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

/// The prompt text of a reduced turn = its synthetic user-message item.
fn prompt_text(turn: &ade_acp::transcript::TranscriptTurn) -> &str {
    turn.items
        .iter()
        .find_map(|item| match item {
            ade_acp::transcript::TranscriptItem::Message(m)
                if m.role == ade_acp::transcript::Role::User =>
            {
                Some(m.text.as_str())
            }
            _ => None,
        })
        .unwrap_or("")
}

/// Concatenated assistant text of a reduced turn.
fn agent_text(turn: &ade_acp::transcript::TranscriptTurn) -> String {
    turn.items
        .iter()
        .filter_map(|item| match item {
            ade_acp::transcript::TranscriptItem::Message(m)
                if m.role == ade_acp::transcript::Role::Assistant =>
            {
                Some(m.text.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

// -- acceptance 1: start → prompt → stop; persistence; restart resume -------

#[tokio::test]
async fn start_prompt_stop_persists_session_id() {
    let dir = tempfile::tempdir().unwrap();
    let (h, client) = harness("conv-1", dir.path()).await;
    let manager = &h.manager;

    let outcome = manager
        .start(start_input("conv-1", dir.path(), None, client))
        .await
        .unwrap();
    assert_eq!(outcome.session_id.0.as_ref(), "fake-session-1");
    assert!(!outcome.resumed);
    assert!(manager.is_running("conv-1"));
    // Session id persisted on start.
    assert_eq!(h.store.get("conv-1").as_deref(), Some("fake-session-1"));

    // Prompt → turn completes with the fake adapter's reply.
    let queued = manager.prompt("conv-1", "hello there", None).await.unwrap();
    assert!(!queued, "idle prompt runs immediately");
    let history = manager.get_chat_history("conv-1");
    assert_eq!(history.committed.len(), 1);
    let turn = &history.committed[0];
    assert_eq!(prompt_text(turn), "hello there");
    assert!(agent_text(turn).contains("Hello from the fake adapter."));
    let state = manager.get_session_state("conv-1");
    assert_eq!(state.lifecycle, Lifecycle::Ready);
    assert!(!state.is_generating);

    // Stop tears the session down.
    manager.stop("conv-1").unwrap();
    assert!(!manager.is_running("conv-1"));
    assert_eq!(
        manager.get_session_state("conv-1").lifecycle,
        Lifecycle::Closed
    );
}

#[tokio::test]
async fn restart_resumes_persisted_session() {
    let dir = tempfile::tempdir().unwrap();

    // First run: fresh session, one prompt, stop.
    let (h1, client) = harness("conv-1", dir.path()).await;
    h1.manager
        .start(start_input("conv-1", dir.path(), None, client))
        .await
        .unwrap();
    h1.manager
        .prompt("conv-1", "first run", None)
        .await
        .unwrap();
    h1.manager.stop("conv-1").unwrap();
    let persisted = h1.store.get("conv-1").unwrap();

    // Second run (new manager + new adapter process = "restart").
    let (h2, client2) = harness("conv-1", dir.path()).await;
    let outcome = h2
        .manager
        .start(start_input(
            "conv-1",
            dir.path(),
            Some(SessionId::new(persisted.as_str())),
            client2,
        ))
        .await
        .unwrap();
    assert!(outcome.resumed, "resume must take the session/load path");
    assert_eq!(outcome.session_id.0.as_ref(), "fake-session-1");

    // Replayed history settled as committed turns (the fake adapter replays
    // two updates — user + agent message — which form one turn).
    let history = h2.manager.get_chat_history("conv-1");
    assert_eq!(
        history.committed.len(),
        1,
        "replay must settle as one committed turn"
    );
    assert!(history.active.is_none());

    // The resumed session still runs prompts on the SAME session id.
    let queued = h2
        .manager
        .prompt("conv-1", "after resume", None)
        .await
        .unwrap();
    assert!(!queued);
    let history = h2.manager.get_chat_history("conv-1");
    assert_eq!(history.committed.len(), 2);
    assert_eq!(prompt_text(&history.committed[1]), "after resume");
    // Persistence ran again on resume (same id).
    assert_eq!(h2.store.get("conv-1").as_deref(), Some("fake-session-1"));
}

// -- acceptance 2: queue ordering + queue ops --------------------------------

#[tokio::test]
async fn initial_queue_runs_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let (h, client) = harness("conv-q", dir.path()).await;
    let manager = &h.manager;

    let mut input = start_input("conv-q", dir.path(), None, client);
    input.initial_queue = vec![
        QueuedPrompt {
            id: "init-a".into(),
            text: "first".into(),
            hidden_context: None,
        },
        QueuedPrompt {
            id: "init-b".into(),
            text: "second".into(),
            hidden_context: None,
        },
        QueuedPrompt {
            id: "init-c".into(),
            text: "third".into(),
            hidden_context: None,
        },
    ];
    manager.start(input).await.unwrap();

    // The initial queue drains in order.
    eventually(|| manager.get_chat_history("conv-q").committed.len() == 3).await;
    let history = manager.get_chat_history("conv-q");
    let texts: Vec<&str> = history.committed.iter().map(prompt_text).collect();
    assert_eq!(texts, ["first", "second", "third"]);
    let seqs: Vec<u64> = history.committed.iter().map(|t| t.seq).collect();
    assert_eq!(seqs, [0, 1, 2]);
}

#[tokio::test]
async fn queue_ops_while_busy_then_ordered_drain_after_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let (h, client) = harness("conv-q2", dir.path()).await;
    let manager = &h.manager;
    manager
        .start(start_input("conv-q2", dir.path(), None, client))
        .await
        .unwrap();

    // Pin the session with a hanging turn.
    let hanging = {
        let manager = Arc::clone(manager);
        tokio::spawn(async move { manager.prompt("conv-q2", "hang please", None).await })
    };
    eventually(|| manager.get_session_state("conv-q2").lifecycle == Lifecycle::Working).await;

    // Queue ops against turn state: queueing works while busy.
    let q1 = manager.queue_prompt("conv-q2", "q1", None).unwrap();
    let q2 = manager.queue_prompt("conv-q2", "q2", None).unwrap();
    let q3 = manager.queue_prompt("conv-q2", "q3", None).unwrap();
    assert_eq!(manager.get_session_state("conv-q2").queued_prompts.len(), 3);

    // Edit q1, drop q2.
    manager
        .edit_queued_prompt("conv-q2", &q1.id, "q1-edited", Some("hidden"))
        .unwrap();
    manager.remove_queued_prompt("conv-q2", &q2.id).unwrap();

    // Reorder [q3, q1]; a mismatched id set is rejected.
    let bad = manager.reorder_queue("conv-q2", std::slice::from_ref(&q3.id));
    assert!(matches!(bad, Err(Error::InvalidState(_))));
    manager
        .reorder_queue("conv-q2", &[q3.id.clone(), q1.id.clone()])
        .unwrap();
    let state = manager.get_session_state("conv-q2");
    let texts: Vec<&str> = state
        .queued_prompts
        .iter()
        .map(|p| p.text.as_str())
        .collect();
    assert_eq!(texts, ["q3", "q1-edited"]);

    // Cancel the hanging turn; the queue then drains in the new order.
    manager.cancel("conv-q2").await.unwrap();
    hanging.await.unwrap().unwrap();

    eventually(|| manager.get_chat_history("conv-q2").committed.len() == 3).await;
    let history = manager.get_chat_history("conv-q2");
    // Turn 0: the cancelled hang. Turns 1+2: the queue in reordered order.
    assert_eq!(prompt_text(&history.committed[0]), "hang please");
    assert_eq!(
        history.committed[0].outcome,
        Some(ade_acp::transcript::TranscriptTurnOutcome::Cancelled { reason: None })
    );
    assert_eq!(prompt_text(&history.committed[1]), "q3");
    assert_eq!(prompt_text(&history.committed[2]), "q1-edited");
    let state = manager.get_session_state("conv-q2");
    assert_eq!(state.lifecycle, Lifecycle::Ready);
    assert!(state.queued_prompts.is_empty());
}

// -- permission broker --------------------------------------------------------

#[tokio::test]
async fn permission_round_trip_through_cell() {
    let dir = tempfile::tempdir().unwrap();
    let (h, client) = harness("conv-p", dir.path()).await;
    let manager = &h.manager;
    manager
        .start(start_input("conv-p", dir.path(), None, client))
        .await
        .unwrap();

    // "tool" makes the fake adapter request a permission and block on it.
    let turn_task = {
        let manager = Arc::clone(manager);
        tokio::spawn(async move { manager.prompt("conv-p", "tool time", None).await })
    };
    eventually(|| {
        !manager
            .get_session_state("conv-p")
            .pending_permissions
            .is_empty()
    })
    .await;
    let pending = manager
        .get_session_state("conv-p")
        .pending_permissions
        .clone();
    assert_eq!(pending.len(), 1);
    let request_id = pending[0].request_id.clone();
    let option_ids: Vec<String> = pending[0]
        .options
        .iter()
        .map(|o| o.option_id.0.to_string())
        .collect();
    assert!(option_ids.contains(&"allow-once".to_string()));

    // Unknown request ids are rejected.
    let unknown = manager.resolve_permission("conv-p", "nope", "allow-once");
    assert!(matches!(unknown, Err(Error::InvalidState(_))));

    manager
        .resolve_permission("conv-p", &request_id, "allow-once")
        .unwrap();
    turn_task.await.unwrap().unwrap();

    let history = manager.get_chat_history("conv-p");
    assert_eq!(history.committed.len(), 1);
    let turn = &history.committed[0];
    // The allowed tool completed — visible as a done edit-kind tool item
    // (the fake adapter's "tool" branch drives a diff-less edit call).
    let tool_done = turn.items.iter().any(|item| match item {
        ade_acp::transcript::TranscriptItem::Tool(t) => {
            t.tool_call_id == "call_001" && t.status == ade_acp::transcript::ToolStatus::Done
        }
        _ => false,
    });
    assert!(tool_done, "allowed tool call must complete");
    assert!(manager
        .get_session_state("conv-p")
        .pending_permissions
        .is_empty());
}

// -- error surface --------------------------------------------------------------

#[tokio::test]
async fn unknown_conversation_errors() {
    let dir = tempfile::tempdir().unwrap();
    let (h, _client) = harness("conv-x", dir.path()).await;
    let manager = &h.manager;

    let err = manager.prompt("ghost", "hi", None).await.unwrap_err();
    assert!(matches!(err, Error::ConversationNotFound(_)));
    let err = manager.queue_prompt("ghost", "hi", None).unwrap_err();
    assert!(matches!(err, Error::ConversationNotFound(_)));
    // stop/cancel on missing conversations are silent no-ops (reference).
    manager.stop("ghost").unwrap();
    manager.cancel("ghost").await.unwrap();
    // History/state of a missing conversation are empty/closed.
    assert!(manager.get_chat_history("ghost").committed.is_empty());
    assert_eq!(
        manager.get_session_state("ghost").lifecycle,
        Lifecycle::Closed
    );
}

#[tokio::test]
async fn route_update_drops_unknown_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let (h, _client) = harness("conv-r", dir.path()).await;
    // Must not panic for unknown sessions.
    h.manager.route_update(
        &SessionId::new("nobody"),
        serde_json::from_value(serde_json::json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "lost" },
        }))
        .unwrap(),
    );
}

// -- cell-level guards (no adapter traffic needed) ------------------------------

#[tokio::test]
async fn closed_cell_rejects_queue_ops() {
    let dir = tempfile::tempdir().unwrap();
    let (h, client) = harness("conv-c", dir.path()).await;
    let manager = &h.manager;
    manager
        .start(start_input("conv-c", dir.path(), None, client))
        .await
        .unwrap();
    manager.stop("conv-c").unwrap();

    // The record is gone — every op reports conversation-not-found.
    let err = manager.queue_prompt("conv-c", "late", None).unwrap_err();
    assert!(matches!(err, Error::ConversationNotFound(_)));
}

/// Draft rev guard + history pagination are pure state; exercise them via a
/// live cell with minimal adapter traffic.
#[tokio::test]
async fn draft_rev_guard_and_history_pagination() {
    let dir = tempfile::tempdir().unwrap();
    let (h, client) = harness("conv-d", dir.path()).await;
    let manager = &h.manager;
    manager
        .start(start_input("conv-d", dir.path(), None, client))
        .await
        .unwrap();

    manager.set_prompt_draft("conv-d", 1, Some("wip")).unwrap();
    assert_eq!(manager.get_prompt_draft("conv-d").unwrap().text, "wip");
    // Stale rev is ignored.
    manager
        .set_prompt_draft("conv-d", 1, Some("older"))
        .unwrap();
    assert_eq!(manager.get_prompt_draft("conv-d").unwrap().text, "wip");
    // Newer rev replaces; None clears.
    manager
        .set_prompt_draft("conv-d", 2, Some("newer"))
        .unwrap();
    assert_eq!(manager.get_prompt_draft("conv-d").unwrap().text, "newer");
    manager.set_prompt_draft("conv-d", 3, None).unwrap();
    assert!(manager.get_prompt_draft("conv-d").is_none());

    // Pagination: run four turns, page them back two at a time (newest-last).
    for i in 0..4 {
        manager
            .prompt("conv-d", &format!("turn {i}"), None)
            .await
            .unwrap();
    }
    let page1 = manager.get_history("conv-d", None, 2);
    let texts: Vec<&str> = page1.turns.iter().map(prompt_text).collect();
    assert_eq!(texts, ["turn 2", "turn 3"]);
    assert_eq!(page1.next_cursor, Some(2));
    let page2 = manager.get_history("conv-d", page1.next_cursor, 2);
    let texts: Vec<&str> = page2.turns.iter().map(prompt_text).collect();
    assert_eq!(texts, ["turn 0", "turn 1"]);
    assert_eq!(page2.next_cursor, Some(0));
    let page3 = manager.get_history("conv-d", page2.next_cursor, 2);
    assert!(page3.turns.is_empty());
    assert_eq!(page3.next_cursor, None);
}
