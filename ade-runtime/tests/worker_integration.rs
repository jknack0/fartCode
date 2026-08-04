//! E2-11-2 acceptance: the ACP worker end to end.
//!
//! SessionHost drives the real `ade-acp-runtime` binary, which spawns the
//! fake adapter and runs a session. Covers: renderer env discarded,
//! server env reaches the adapter, start → prompt → updates → stop with
//! exit code surfaced, permission round-trip through the worker, and
//! cancel mid-turn.

use std::path::PathBuf;
use std::time::Duration;

use ade_runtime::protocol::{prepare_session, EnvPair, SessionInput};
use ade_runtime::session_host::{PermissionOutcome, SessionHost};

fn worker_bin() -> PathBuf {
    env!("CARGO_BIN_EXE_ade-acp-runtime").into()
}

fn adapter_bin() -> PathBuf {
    // The fake adapter is ade-acp's bin; it lives in the shared target dir
    // next to this test binary's parent (`target/debug`). Workspace-wide
    // `cargo test` builds it; for isolated runs build ade-acp first.
    let exe = std::env::current_exe().expect("current exe");
    let path = exe
        .parent() // deps/
        .and_then(|p| p.parent()) // debug/
        .map(|p| p.join("fake_acp_adapter"))
        .expect("target dir");
    assert!(
        path.exists(),
        "fake_acp_adapter not built — run `cargo build -p ade-acp --bins` (workspace `cargo test` builds it)"
    );
    path
}

fn session_input(cwd: &std::path::Path, renderer_env: Vec<EnvPair>) -> SessionInput {
    SessionInput {
        conversation_id: "conv-worker-1".into(),
        project_id: "proj-1".into(),
        task_id: "task-1".into(),
        provider_id: "claude".into(),
        workspace_id: "ws-1".into(),
        cwd: cwd.to_string_lossy().to_string(),
        session_id: None,
        model: None,
        initial_queue: vec![],
        adapter_path: adapter_bin().to_string_lossy().to_string(),
        adapter_args: vec![],
        env: renderer_env,
    }
}

#[tokio::test]
async fn full_worker_session_start_prompt_stop() {
    let dir = tempfile::tempdir().unwrap();
    let prepared = prepare_session(session_input(dir.path(), vec![]), &[]).unwrap();
    let (host, session_id) = SessionHost::start(&worker_bin(), prepared).await.unwrap();
    assert_eq!(session_id, "fake-session-1");

    // Prompt turn streams updates through the worker.
    let turn = host.prompt("say hello").await.unwrap();
    assert_eq!(turn["stopReason"], "end_turn");
    let mut chunks = vec![];
    while let Some(update) = host.try_recv_update().await {
        if update["update"]["sessionUpdate"] == "agent_message_chunk" {
            chunks.push(
                update["update"]["content"]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            );
        }
    }
    assert!(
        chunks.join("").contains("Hello from the fake adapter"),
        "updates must reach the host; got {chunks:?}"
    );

    let exit_code = host.stop().await.unwrap();
    assert_eq!(exit_code, Some(0));
}

#[tokio::test]
async fn renderer_env_discarded_and_server_env_reaches_adapter() {
    let dir = tempfile::tempdir().unwrap();
    let input = session_input(
        dir.path(),
        vec![
            EnvPair {
                key: "ADE_TEST_ENV".into(),
                value: "renderer-injected".into(),
            },
            EnvPair {
                key: "LD_PRELOAD".into(),
                value: "/tmp/evil.so".into(),
            },
        ],
    );
    // Server-side resolution is the only env source.
    let prepared =
        prepare_session(input, &[("ADE_TEST_ENV".into(), "server-resolved".into())]).unwrap();
    let (host, _session_id) = SessionHost::start(&worker_bin(), prepared).await.unwrap();

    // The adapter's envcheck mode echoes what it received: it MUST be the
    // server-resolved value, never the renderer's.
    host.prompt("envcheck please").await.unwrap();
    let mut env_seen = String::new();
    while let Some(update) = host.try_recv_update().await {
        if update["update"]["sessionUpdate"] == "agent_message_chunk" {
            env_seen.push_str(update["update"]["content"]["text"].as_str().unwrap_or(""));
        }
    }
    assert!(
        env_seen.contains("ENV: server-resolved"),
        "adapter must see server env, got {env_seen:?}"
    );
    assert!(
        !env_seen.contains("renderer-injected"),
        "renderer env leaked to the adapter: {env_seen:?}"
    );
    host.stop().await.unwrap();
}

#[tokio::test]
async fn permission_request_round_trip_through_worker() {
    let dir = tempfile::tempdir().unwrap();
    let prepared = prepare_session(session_input(dir.path(), vec![]), &[]).unwrap();
    let (host, _session_id) = SessionHost::start(&worker_bin(), prepared).await.unwrap();
    // Share across the prompt task + the permission-answering test thread.
    let host = std::sync::Arc::new(host);

    let prompt_host = std::sync::Arc::clone(&host);
    let turn = tokio::spawn(async move { prompt_host.prompt("please use the tool").await });

    // Wait for the surfaced permission request and allow it.
    let surfaced = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(p) = host.try_recv_permission().await {
                return p;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("permission request must be surfaced by the worker");
    assert!(
        !surfaced.request.options.is_empty(),
        "surfaced request carries the adapter's options"
    );
    host.resolve_permission(
        &surfaced.request_id,
        PermissionOutcome::Selected("allow-once".into()),
    )
    .await
    .unwrap();

    let result = turn.await.unwrap().unwrap();
    assert_eq!(result["stopReason"], "end_turn");
    // The allow path produced a completed tool_call_update.
    let mut completed = false;
    while let Some(update) = host.try_recv_update().await {
        if update["update"]["sessionUpdate"] == "tool_call_update"
            && update["update"]["status"] == "completed"
        {
            completed = true;
        }
    }
    assert!(
        completed,
        "allowed tool call must complete through the worker"
    );
    host.stop().await.unwrap();
}

#[tokio::test]
async fn cancel_mid_turn_through_worker() {
    let dir = tempfile::tempdir().unwrap();
    let prepared = prepare_session(session_input(dir.path(), vec![]), &[]).unwrap();
    let (host, _session_id) = SessionHost::start(&worker_bin(), prepared).await.unwrap();
    let host = std::sync::Arc::new(host);

    let prompt_host = std::sync::Arc::clone(&host);
    let turn = tokio::spawn(async move { prompt_host.prompt("hang forever").await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    host.cancel().await.unwrap();

    let result = tokio::time::timeout(Duration::from_secs(5), turn)
        .await
        .expect("cancel must end the turn promptly")
        .unwrap()
        .unwrap();
    assert_eq!(result["stopReason"], "cancelled");
    host.stop().await.unwrap();
}
