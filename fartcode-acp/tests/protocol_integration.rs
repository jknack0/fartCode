//! E2-11-1 acceptance: ACP protocol client against the fake adapter binary.
//!
//! Covers: start → initialize → session → prompt → turn updates → stop;
//! session-id return; permission round-trip; cancel mid-turn; session/load
//! replay; version mismatch rejection; malformed-frame tolerance; scoped
//! `fs/read_text_file` handling.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol_schema::v1::{ContentBlock, SessionId, SessionUpdate};
use fartcode_acp::{AcpClient, Error, PermissionDecision, PermissionRequest};
use tokio::sync::mpsc;

fn adapter_path() -> PathBuf {
    env!("CARGO_BIN_EXE_fake_acp_adapter").into()
}

fn text_block(text: &str) -> ContentBlock {
    serde_json::from_value(serde_json::json!({ "type": "text", "text": text })).unwrap()
}

/// Spawns a client whose permission resolver surfaces each request on
/// `decisions` and waits for the test to send a [`PermissionDecision`]
/// back on the paired channel before answering the adapter.
async fn spawn_client(
    cwd: &std::path::Path,
    decisions: mpsc::Sender<(PermissionRequest, mpsc::Sender<PermissionDecision>)>,
) -> Arc<AcpClient> {
    let client = AcpClient::spawn(
        adapter_path(),
        &[],
        &[],
        Some(cwd),
        Some(cwd.to_path_buf()),
        move |request: PermissionRequest, answerer| {
            let decisions = decisions.clone();
            tokio::spawn(async move {
                let (decision_tx, mut decision_rx) = mpsc::channel(1);
                if decisions.send((request, decision_tx)).await.is_err() {
                    return; // test dropped; leave the request unanswered
                }
                // The test sends the decision; default to allow-once if it
                // never does so a scripted flow cannot hang the adapter.
                let decision = tokio::time::timeout(Duration::from_secs(2), decision_rx.recv())
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or(PermissionDecision::Selected("allow-once".into()));
                let _ = answerer.answer(decision).await;
            });
        },
    )
    .await
    .unwrap();
    let client = Arc::new(client);
    let loop_client = Arc::clone(&client);
    tokio::spawn(async move { loop_client.run_event_loop().await });
    client
}

fn agent_text(updates: &[fartcode_acp::SessionUpdateEvent]) -> String {
    updates
        .iter()
        .filter_map(|u| match &u.update {
            SessionUpdate::AgentMessageChunk(chunk) => match &chunk.content {
                agent_client_protocol_schema::v1::ContentBlock::Text(t) => Some(t.text.to_string()),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn tool_call_statuses(updates: &[fartcode_acp::SessionUpdateEvent]) -> Vec<&'static str> {
    updates
        .iter()
        .filter_map(|u| match &u.update {
            SessionUpdate::ToolCallUpdate(update) => match update.fields.status.as_ref() {
                Some(agent_client_protocol_schema::v1::ToolCallStatus::Completed) => {
                    Some("completed")
                }
                Some(agent_client_protocol_schema::v1::ToolCallStatus::Failed) => Some("failed"),
                Some(agent_client_protocol_schema::v1::ToolCallStatus::InProgress) => {
                    Some("in_progress")
                }
                Some(agent_client_protocol_schema::v1::ToolCallStatus::Pending) => Some("pending"),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn full_turn_streams_updates_and_returns_stop_reason() {
    let dir = tempfile::tempdir().unwrap();
    let (decisions, _rx) = mpsc::channel(4);
    let client = spawn_client(dir.path(), decisions).await;

    let init = client.initialize().await.unwrap();
    assert_eq!(init.protocol_version, 1);

    let session = client.new_session(dir.path()).await.unwrap();
    assert_eq!(session.0.as_ref(), "fake-session-1");

    let turn = client
        .prompt(&session, vec![text_block("say hello")])
        .await
        .unwrap();
    assert_eq!(turn.updates.len(), 2, "two agent chunks expected");
    assert!(agent_text(&turn.updates).contains("Hello from the fake adapter"));
    match turn.response.stop_reason {
        agent_client_protocol_schema::v1::StopReason::EndTurn => {}
        other => panic!("expected end_turn, got {other:?}"),
    }
    client.shutdown().await;
}

#[tokio::test]
async fn permission_request_round_trip_allow() {
    let dir = tempfile::tempdir().unwrap();
    let (decisions, mut decisions_rx) =
        mpsc::channel::<(PermissionRequest, mpsc::Sender<PermissionDecision>)>(4);
    let client = spawn_client(dir.path(), decisions).await;
    client.initialize().await.unwrap();
    let session = client.new_session(dir.path()).await.unwrap();

    let prompt = {
        let session = session.clone();
        let client = Arc::clone(&client);
        tokio::spawn(async move {
            client
                .prompt(&session, vec![text_block("please use the tool")])
                .await
        })
    };

    // The adapter blocks on our answer — the resolver surfaced the request.
    let (_request, decision_tx) = tokio::time::timeout(Duration::from_secs(5), decisions_rx.recv())
        .await
        .unwrap()
        .expect("resolver must surface the permission request");
    decision_tx
        .send(PermissionDecision::Selected("allow-once".into()))
        .await
        .unwrap();

    let turn = prompt.await.unwrap().unwrap();
    let statuses = tool_call_statuses(&turn.updates);
    assert!(
        statuses.contains(&"completed"),
        "allowed tool call must complete; got {statuses:?}"
    );
    client.shutdown().await;
}

#[tokio::test]
async fn permission_request_round_trip_reject() {
    let dir = tempfile::tempdir().unwrap();
    let (decisions, mut decisions_rx) =
        mpsc::channel::<(PermissionRequest, mpsc::Sender<PermissionDecision>)>(4);
    let client = spawn_client(dir.path(), decisions).await;
    client.initialize().await.unwrap();
    let session = client.new_session(dir.path()).await.unwrap();

    let prompt = {
        let session = session.clone();
        let client = Arc::clone(&client);
        tokio::spawn(async move {
            client
                .prompt(&session, vec![text_block("please use the tool")])
                .await
        })
    };
    let (_request, decision_tx) = tokio::time::timeout(Duration::from_secs(5), decisions_rx.recv())
        .await
        .unwrap()
        .expect("resolver must surface the permission request");
    decision_tx
        .send(PermissionDecision::Selected("reject-once".into()))
        .await
        .unwrap();

    let turn = prompt.await.unwrap().unwrap();
    let statuses = tool_call_statuses(&turn.updates);
    assert!(
        statuses.contains(&"failed"),
        "rejected tool call must fail; got {statuses:?}"
    );
    client.shutdown().await;
}

#[tokio::test]
async fn cancel_mid_turn_ends_prompt_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let (decisions, _rx) = mpsc::channel(4);
    let client = spawn_client(dir.path(), decisions).await;
    client.initialize().await.unwrap();
    let session = client.new_session(dir.path()).await.unwrap();

    let prompt = {
        let session = session.clone();
        let client = Arc::clone(&client);
        tokio::spawn(async move {
            client
                .prompt(&session, vec![text_block("hang forever")])
                .await
        })
    };
    // Let the adapter enter its waiting loop before cancelling.
    tokio::time::sleep(Duration::from_millis(100)).await;
    client.cancel(&session).await.unwrap();

    let turn = tokio::time::timeout(Duration::from_secs(5), prompt)
        .await
        .expect("cancel must end the turn promptly")
        .unwrap()
        .unwrap();
    match turn.response.stop_reason {
        agent_client_protocol_schema::v1::StopReason::Cancelled => {}
        other => panic!("expected cancelled, got {other:?}"),
    }
    client.shutdown().await;
}

#[tokio::test]
async fn load_session_replays_history_updates() {
    let dir = tempfile::tempdir().unwrap();
    let (decisions, _rx) = mpsc::channel(4);
    let client = spawn_client(dir.path(), decisions).await;
    client.initialize().await.unwrap();

    let session = SessionId::new("fake-session-1");
    let (_response, updates) = client.load_session(&session, dir.path()).await.unwrap();
    assert_eq!(updates.len(), 2, "replay must stream both history entries");
    let text = agent_text(&updates);
    assert!(text.contains("earlier agent reply"));
    client.shutdown().await;
}

#[tokio::test]
async fn version_mismatch_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let client = AcpClient::spawn(
        adapter_path(),
        &[],
        &[("FAKE_ACP_VERSION".into(), "9".into())],
        Some(dir.path()),
        None,
        |_request, _answerer| {},
    )
    .await
    .unwrap();
    let err = client.initialize().await.unwrap_err();
    match err {
        Error::VersionMismatch { agent, client } => {
            assert_eq!(agent, 9);
            assert_eq!(client, 1);
        }
        other => panic!("expected VersionMismatch, got {other:?}"),
    }
    client.shutdown().await;
}

#[tokio::test]
async fn malformed_frames_do_not_break_the_turn() {
    let dir = tempfile::tempdir().unwrap();
    let (decisions, _rx) = mpsc::channel(4);
    let client = spawn_client(dir.path(), decisions).await;
    client.initialize().await.unwrap();
    let session = client.new_session(dir.path()).await.unwrap();

    let turn = client
        .prompt(&session, vec![text_block("emitgarbage please")])
        .await
        .unwrap();
    assert!(agent_text(&turn.updates).contains("recovered after garbage"));
    client.shutdown().await;
}

#[tokio::test]
async fn fs_read_text_file_is_served_workspace_scoped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("fixture.txt"), "fixture-payload").unwrap();
    let (decisions, _rx) = mpsc::channel(4);
    let client = spawn_client(dir.path(), decisions).await;
    client.initialize().await.unwrap();
    let session = client.new_session(dir.path()).await.unwrap();

    let turn = client
        .prompt(&session, vec![text_block("readfile now")])
        .await
        .unwrap();
    assert!(
        agent_text(&turn.updates).contains("File says: fixture-payload"),
        "fs request must be answered with the workspace file content"
    );
    client.shutdown().await;
}
