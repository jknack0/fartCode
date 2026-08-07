//! `fartcode-acp-runtime` — the out-of-process ACP worker (E2-11-2).
//!
//! Hosted by `fartcode-app` via `fartcode_runtime::SessionHost`. One worker = one ACP
//! session. Speaks newline-delimited JSON-RPC to the host (stdin requests
//! `session/start|prompt|cancel|stop`; stdout responses + notifications
//! `worker/update`, `worker/permission_request`, `worker/exited`) and ACP
//! to the adapter binary over its own stdio.
//!
//! The adapter env arrives inside the `session/start` params
//! (`WorkerSession.env`) — already server-resolved by the host (E3-07).
//! The renderer never contributes env (see `fartcode_runtime::protocol`).

use std::collections::HashMap;
use std::sync::Arc;

use agent_client_protocol_schema::v1::SessionConfigOptionValue;
use fartcode_acp::{AcpClient, PermissionDecision, PermissionRequest, StdioTransport};
use fartcode_runtime::process_host::{AdapterProcessConfig, LocalProcessHost, ProcessHost};
use fartcode_runtime::protocol::{worker_methods, WorkerSession};
use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot};

type Out = Arc<Mutex<std::io::Stdout>>;

fn write_frame(out: &Out, frame: Value) {
    use std::io::Write;
    let mut stdout = out.lock();
    let mut line = serde_json::to_vec(&frame).unwrap_or_default();
    line.push(b'\n');
    let _ = stdout.write_all(&line);
    let _ = stdout.flush();
}

fn reply(out: &Out, request: &Value, result: Value) {
    write_frame(
        out,
        json!({ "jsonrpc": "2.0", "id": request["id"], "result": result }),
    );
}

fn reply_error(out: &Out, request: &Value, code: i64, message: &str) {
    write_frame(
        out,
        json!({ "jsonrpc": "2.0", "id": request["id"], "error": { "code": code, "message": message } }),
    );
}

/// Pending permission decisions: requestId → decision channel.
type PermissionMap = Arc<Mutex<HashMap<String, oneshot::Sender<PermissionDecision>>>>;

#[tokio::main]
async fn main() {
    let out: Out = Arc::new(Mutex::new(std::io::stdout()));
    let pending_permissions: PermissionMap = Arc::new(Mutex::new(HashMap::new()));

    let mut client_slot: Option<Arc<AcpClient>> = None;
    let mut session_slot: Option<agent_client_protocol_schema::v1::SessionId> = None;

    let stdin = std::io::stdin();
    use std::io::BufRead;
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let frame: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "malformed host frame dropped");
                continue;
            }
        };
        let method = frame.get("method").and_then(|m| m.as_str()).unwrap_or("");

        match method {
            "worker/permission_response" => {
                let request_id = frame["params"]["requestId"].as_str().unwrap_or("");
                let decision = match &frame["params"]["outcome"]["outcome"] {
                    v if v.as_str() == Some("selected") => {
                        let option_id = frame["params"]["outcome"]["optionId"]
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        PermissionDecision::Selected(option_id)
                    }
                    _ => PermissionDecision::Cancelled,
                };
                if let Some(tx) = pending_permissions.lock().remove(request_id) {
                    let _ = tx.send(decision);
                }
            }
            worker_methods::SESSION_START => {
                let session: WorkerSession = match serde_json::from_value(frame["params"].clone()) {
                    Ok(s) => s,
                    Err(e) => {
                        reply_error(&out, &frame, -32602, &format!("invalid session: {e}"));
                        continue;
                    }
                };
                match start_session(&out, &session, Arc::clone(&pending_permissions)).await {
                    Ok((client, session_id)) => {
                        // Update pump: ACP session/update → worker/update.
                        let mut stream = client.updates();
                        let pump_out = Arc::clone(&out);
                        let sid = session_id.clone();
                        tokio::spawn(async move {
                            while let Ok(raw) = stream.recv().await {
                                if raw["sessionId"].as_str() == Some(sid.0.as_ref()) {
                                    write_frame(
                                        &pump_out,
                                        json!({
                                            "jsonrpc": "2.0",
                                            "method": worker_methods::WORKER_UPDATE,
                                            "params": raw,
                                        }),
                                    );
                                }
                            }
                        });
                        // Agent-request pump (fs/terminal/permission handled
                        // via LocalHandlers with the host-routed resolver).
                        let loop_client = Arc::clone(&client);
                        tokio::spawn(async move { loop_client.run_event_loop().await });

                        reply(&out, &frame, json!({ "sessionId": session_id.0.as_ref() }));
                        client_slot = Some(client);
                        session_slot = Some(session_id);
                    }
                    Err(e) => {
                        reply_error(&out, &frame, -32000, &e.to_string());
                    }
                }
            }
            worker_methods::SESSION_PROMPT => {
                // Spawned, not awaited inline: the stdin loop must stay free
                // to read permission responses / cancel while the turn is in
                // flight (awaiting here deadlocks the channel).
                let (Some(client), Some(session_id)) = (client_slot.clone(), session_slot.clone())
                else {
                    reply_error(&out, &frame, -32000, "no session started");
                    continue;
                };
                let text = frame["params"]["text"].as_str().unwrap_or("").to_string();
                let request_id = frame["id"].clone();
                let prompt_out = Arc::clone(&out);
                tokio::spawn(async move {
                    let block: agent_client_protocol_schema::v1::ContentBlock =
                        match serde_json::from_value(json!({ "type": "text", "text": text })) {
                            Ok(b) => b,
                            Err(e) => {
                                write_frame(
                                    &prompt_out,
                                    json!({ "jsonrpc": "2.0", "id": request_id, "error": { "code": -32602, "message": format!("bad content: {e}") } }),
                                );
                                return;
                            }
                        };
                    match client.prompt(&session_id, vec![block]).await {
                        Ok(turn) => {
                            let stop_reason =
                                serde_json::to_value(&turn.response).unwrap_or(json!({}));
                            write_frame(
                                &prompt_out,
                                json!({ "jsonrpc": "2.0", "id": request_id, "result": stop_reason }),
                            );
                        }
                        Err(e) => {
                            write_frame(
                                &prompt_out,
                                json!({ "jsonrpc": "2.0", "id": request_id, "error": { "code": -32000, "message": e.to_string() } }),
                            );
                        }
                    }
                });
            }
            worker_methods::SESSION_CANCEL => {
                if let (Some(client), Some(session_id)) = (&client_slot, &session_slot) {
                    let _ = client.cancel(session_id).await;
                }
                reply(&out, &frame, json!({}));
            }
            worker_methods::SESSION_STOP => {
                reply(&out, &frame, json!({}));
                if let Some(client) = client_slot.take() {
                    client.shutdown().await;
                }
                write_frame(
                    &out,
                    json!({
                        "jsonrpc": "2.0",
                        "method": worker_methods::WORKER_EXITED,
                        "params": { "code": 0 },
                    }),
                );
                std::process::exit(0);
            }
            other => reply_error(&out, &frame, -32601, &format!("unknown method {other}")),
        }
    }
    // Host closed stdin — tear down and exit.
    if let Some(client) = client_slot.take() {
        client.shutdown().await;
    }
}

async fn start_session(
    out: &Out,
    session: &WorkerSession,
    pending_permissions: PermissionMap,
) -> Result<(Arc<AcpClient>, agent_client_protocol_schema::v1::SessionId), fartcode_runtime::Error>
{
    // Spawn the adapter through the process host (env = server-resolved
    // credentials only; see fartcode_runtime::protocol).
    let host = LocalProcessHost;
    let cwd = std::path::PathBuf::from(&session.cwd);
    let child = host.spawn(&AdapterProcessConfig {
        program: std::path::PathBuf::from(&session.adapter_path),
        args: session.adapter_args.clone(),
        env: session
            .env
            .iter()
            .map(|p| (p.key.clone(), p.value.clone()))
            .collect(),
        cwd: cwd.clone(),
    })?;

    let (updates_tx, updates_rx) = broadcast::channel(256);
    let (events_tx, events_rx) = mpsc::channel(64);
    let transport = Arc::new(
        StdioTransport::from_child(child, updates_tx, events_tx)
            .map_err(|e| fartcode_runtime::Error::Protocol(e.to_string()))?,
    );

    // Permission resolver: surface to the host, await its decision, then
    // answer the adapter's ACP request.
    let resolver_out = Arc::clone(out);
    let resolver = move |request: PermissionRequest,
                         answerer: Arc<fartcode_acp::PermissionAnswerer>| {
        let out = Arc::clone(&resolver_out);
        let pending = Arc::clone(&pending_permissions);
        tokio::spawn(async move {
            let request_id = uuid_v4();
            let wire_request = serde_json::to_value(&request.request).unwrap_or(json!({}));
            write_frame(
                &out,
                json!({
                    "jsonrpc": "2.0",
                    "method": worker_methods::WORKER_PERMISSION_REQUEST,
                    "params": { "requestId": request_id, "request": wire_request },
                }),
            );
            let (tx, rx) = oneshot::channel();
            pending.lock().insert(request_id, tx);
            let decision = rx.await.unwrap_or(PermissionDecision::Cancelled);
            let _ = answerer.answer(decision).await;
        });
    };

    let client = Arc::new(AcpClient::from_transport(
        transport,
        updates_rx,
        events_rx,
        Some(cwd.clone()),
        resolver,
    ));

    client
        .initialize()
        .await
        .map_err(|e| fartcode_runtime::Error::Protocol(e.to_string()))?;

    // Resume or create.
    let session_id = match &session.session_id {
        Some(id) => {
            let sid = agent_client_protocol_schema::v1::SessionId::new(id.as_str());
            match client.load_session(&sid, &cwd).await {
                Ok(_) => sid,
                Err(e) => {
                    tracing::warn!(error = %e, "session/load failed; starting a new session");
                    client
                        .new_session(&cwd)
                        .await
                        .map_err(|e| fartcode_runtime::Error::Protocol(e.to_string()))?
                }
            }
        }
        None => client
            .new_session(&cwd)
            .await
            .map_err(|e| fartcode_runtime::Error::Protocol(e.to_string()))?,
    };

    // Model override via config options (v1 has no set_model method);
    // adapters without model config ignore it.
    if let Some(model) = &session.model {
        if let Err(e) = client
            .set_config_option(
                &session_id,
                "model",
                SessionConfigOptionValue::ValueId {
                    value: agent_client_protocol_schema::v1::SessionConfigValueId::new(
                        model.as_str(),
                    ),
                },
            )
            .await
        {
            tracing::warn!(error = %e, "model override not applied (adapter may not support it)");
        }
    }

    // Drain the initial queue (queued prompts run before user turns).
    for queued in &session.initial_queue {
        let block: agent_client_protocol_schema::v1::ContentBlock =
            match serde_json::from_value(json!({ "type": "text", "text": queued.text })) {
                Ok(b) => b,
                Err(_) => continue,
            };
        if let Err(e) = client.prompt(&session_id, vec![block]).await {
            tracing::warn!(error = %e, "queued prompt failed");
        }
    }

    Ok((client, session_id))
}

/// Request id for permission correlation — unique within this worker
/// process (one session per worker).
fn uuid_v4() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "perm-{}",
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
    )
}
