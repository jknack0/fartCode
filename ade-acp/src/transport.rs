//! Newline-delimited JSON-RPC transport over a child process's stdio.
//!
//! The wire envelope is handled as raw JSON (fixed JSON-RPC 2.0 shape);
//! method payloads are typed via `agent_client_protocol_schema` at the
//! client/handler layer. Malformed frames are warnings, never panics.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use agent_client_protocol_schema::rpc::RequestId;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};

use crate::error::Error;

/// A request the agent issued toward us (permission, fs/*, terminal/*).
/// The client MUST answer with [`StdioTransport::respond`] (or
/// `respond_error`).
#[derive(Debug)]
pub struct AgentRequest {
    /// JSON-RPC request id to answer with.
    pub id: RequestId,
    /// Method name.
    pub method: String,
    /// Raw params (parse into the typed request struct at the handler).
    pub params: serde_json::Value,
}

/// What the transport hands back to the client loop.
#[derive(Debug)]
pub enum Incoming {
    /// Agent → client request that needs a response.
    AgentRequest(AgentRequest),
    /// The adapter process exited (no more frames will arrive).
    Closed,
}

type Pending = Arc<Mutex<HashMap<RequestId, oneshot::Sender<Result<serde_json::Value, Error>>>>>;

/// JSON-RPC-over-stdio transport bound to one adapter child process.
///
/// Split design: `spawn` wires everything; the reader task owns the stdout
/// pump and resolves pending requests / routes updates + agent requests;
/// writers share the stdin mutex.
pub struct StdioTransport {
    child: Mutex<Child>,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
    next_id: Mutex<i64>,
}

impl StdioTransport {
    /// Spawns the adapter and starts the reader pump.
    ///
    /// `updates` receives `session/update` notification params as raw JSON
    /// (typed at the client layer); `events` receives agent requests and
    /// the closed signal.
    /// Spawns the adapter locally and starts the reader pump (convenience
    /// wrapper over [`StdioTransport::from_child`]).
    ///
    /// `updates` receives `session/update` notification params as raw JSON
    /// (typed at the client layer); `events` receives agent requests and
    /// the closed signal.
    pub async fn spawn(
        program: impl AsRef<std::ffi::OsStr>,
        args: &[String],
        env: &[(String, String)],
        cwd: Option<&std::path::Path>,
        updates: broadcast::Sender<serde_json::Value>,
        events_tx: mpsc::Sender<Incoming>,
    ) -> Result<Self, Error> {
        let mut cmd = Command::new(program.as_ref());
        cmd.args(args)
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        let child = cmd.spawn()?;
        Self::from_child(child, updates, events_tx)
    }

    /// Adopts an already-spawned child (e.g. from a remote process host,
    /// E2-11-2 SSH transport seam) and starts the reader pump. The child
    /// MUST have piped stdin/stdout.
    pub fn from_child(
        mut child: tokio::process::Child,
        updates: broadcast::Sender<serde_json::Value>,
        events_tx: mpsc::Sender<Incoming>,
    ) -> Result<Self, Error> {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Protocol("adapter stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Protocol("adapter stdout unavailable".into()))?;

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let transport = Self {
            child: Mutex::new(child),
            stdin: Arc::new(Mutex::new(stdin)),
            pending: pending.clone(),
            next_id: Mutex::new(1),
        };

        tokio::spawn(reader_loop(stdout, pending, updates, events_tx));
        Ok(transport)
    }

    /// Sends a request; resolves when the agent answers. The returned future
    /// yields the raw `result` value or a typed agent/transport error.
    pub async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, Error> {
        let id = {
            let mut next = self.next_id.lock().await;
            let id = *next;
            *next += 1;
            RequestId::Number(id)
        };
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);

        let frame = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        if let Err(e) = self.write_frame(&frame).await {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }
        rx.await
            .map_err(|_| Error::Closed("adapter exited before responding".into()))?
    }

    /// Sends a notification (no id, no response expected).
    pub async fn notify(&self, method: &str, params: serde_json::Value) -> Result<(), Error> {
        let frame = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.write_frame(&frame).await
    }

    /// Answers an agent request with a success result.
    pub async fn respond(&self, id: RequestId, result: serde_json::Value) -> Result<(), Error> {
        let frame = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        });
        self.write_frame(&frame).await
    }

    /// Answers an agent request with a JSON-RPC error.
    pub async fn respond_error(
        &self,
        id: RequestId,
        code: i32,
        message: &str,
    ) -> Result<(), Error> {
        let frame = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message },
        });
        self.write_frame(&frame).await
    }

    async fn write_frame(&self, frame: &serde_json::Value) -> Result<(), Error> {
        let mut line = serde_json::to_vec(frame)
            .map_err(|e| Error::Protocol(format!("serialize frame: {e}")))?;
        line.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(&line)
            .await
            .map_err(|e| Error::Closed(format!("adapter stdin write: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| Error::Closed(format!("adapter stdin flush: {e}")))?;
        Ok(())
    }

    /// Waits for the adapter to exit and returns its exit code (None when
    /// the OS provides none). Call on the normal-exit path; use `shutdown`
    /// for forced teardown — do not mix both on one transport.
    pub async fn wait_exit(&self) -> Option<i32> {
        let mut child = self.child.lock().await;
        child.wait().await.ok().and_then(|status| status.code())
    }

    /// Terminates the adapter (best-effort kill, then reap).
    pub async fn shutdown(&self) {
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

async fn reader_loop(
    stdout: tokio::process::ChildStdout,
    pending: Pending,
    updates: broadcast::Sender<serde_json::Value>,
    events_tx: mpsc::Sender<Incoming>,
) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let frame: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "malformed ACP frame dropped");
                continue;
            }
        };
        let obj = match frame.as_object() {
            Some(o) => o,
            None => {
                tracing::warn!("non-object ACP frame dropped");
                continue;
            }
        };

        let id = obj.get("id").and_then(parse_request_id);
        let has_method = obj.contains_key("method");

        match (id, has_method) {
            // Response to one of our requests.
            (Some(id), false) => {
                let outcome = if let Some(error) = obj.get("error") {
                    Err(Error::AgentError {
                        code: error.get("code").and_then(|c| c.as_i64()).unwrap_or(0) as i32,
                        message: error
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("unknown agent error")
                            .to_string(),
                    })
                } else {
                    Ok(obj
                        .get("result")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null))
                };
                if let Some(tx) = pending.lock().await.remove(&id) {
                    let _ = tx.send(outcome);
                } else {
                    tracing::warn!(%id, "response for unknown request id");
                }
            }
            // Agent request (has id + method) — needs our response.
            (Some(id), true) => {
                let method = obj["method"].as_str().unwrap_or("").to_string();
                let params = obj.get("params").cloned().unwrap_or(serde_json::json!({}));
                let _ = events_tx
                    .send(Incoming::AgentRequest(AgentRequest { id, method, params }))
                    .await;
            }
            // Notification.
            (None, true) => {
                let method = obj["method"].as_str().unwrap_or("");
                if method == crate::methods::SESSION_UPDATE {
                    let params = obj.get("params").cloned().unwrap_or(serde_json::json!({}));
                    let _ = updates.send(params);
                } else {
                    tracing::debug!(method, "unhandled ACP notification");
                }
            }
            (None, false) => {
                tracing::warn!("frame with neither id nor method dropped");
            }
        }
    }
    let _ = events_tx.send(Incoming::Closed).await;
}

fn parse_request_id(v: &serde_json::Value) -> Option<RequestId> {
    match v {
        serde_json::Value::Null => Some(RequestId::Null),
        serde_json::Value::Number(n) => n.as_i64().map(RequestId::Number),
        serde_json::Value::String(s) => Some(RequestId::Str(s.clone())),
        _ => None,
    }
}
