//! App-side driver of the `ade-acp-runtime` worker binary (E2-11-2).
//!
//! One [`SessionHost`] owns one worker child process (one ACP session).
//! Wire format between host and worker: newline-delimited JSON-RPC —
//! host→worker requests (`session/start|prompt|cancel|stop`), worker→host
//! responses, and worker→host notifications (`worker/update`,
//! `worker/permission_request`, `worker/exited`).
//!
//! Env security: the host hands the worker a [`WorkerSession`] whose env is
//! already server-resolved — the renderer's `env` never survives
//! [`crate::protocol::prepare_session`].

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::error::Error;
use crate::protocol::{worker_methods, WorkerSession};

/// A permission request surfaced by the worker, awaiting a decision.
#[derive(Debug, Clone)]
pub struct PermissionSurfaced {
    /// Correlation id — pass back to [`SessionHost::resolve_permission`].
    pub request_id: String,
    /// The parsed ACP permission request (session, tool call, options).
    pub request: agent_client_protocol_schema::v1::RequestPermissionRequest,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<serde_json::Value, Error>>>>>;

/// Drives one worker process.
pub struct SessionHost {
    child: Mutex<Child>,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Pending,
    next_id: Mutex<u64>,
    updates_rx: Mutex<mpsc::Receiver<serde_json::Value>>,
    permissions_rx: Mutex<mpsc::Receiver<PermissionSurfaced>>,
}

impl SessionHost {
    /// Spawns the worker binary and starts its session. Resolves with the
    /// ACP session id returned by the adapter.
    pub async fn start(worker_bin: &Path, session: WorkerSession) -> Result<(Self, String), Error> {
        let mut child = Command::new(worker_bin)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .map_err(|e| Error::Spawn(format!("worker {}: {e}", worker_bin.display())))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Protocol("worker stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Protocol("worker stdout unavailable".into()))?;

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (updates_tx, updates_rx) = mpsc::channel(256);
        let (permissions_tx, permissions_rx) = mpsc::channel(32);

        tokio::spawn(reader_loop(
            stdout,
            pending.clone(),
            updates_tx.clone(),
            permissions_tx.clone(),
        ));

        let host = Self {
            child: Mutex::new(child),
            stdin: Arc::new(Mutex::new(stdin)),
            pending,
            next_id: Mutex::new(1),
            updates_rx: Mutex::new(updates_rx),
            permissions_rx: Mutex::new(permissions_rx),
        };

        let response = host
            .request(
                worker_methods::SESSION_START,
                serde_json::to_value(&session)
                    .map_err(|e| Error::Protocol(format!("serialize session: {e}")))?,
            )
            .await?;
        let session_id = response["sessionId"]
            .as_str()
            .ok_or_else(|| Error::Protocol("worker start: missing sessionId".into()))?
            .to_string();
        Ok((host, session_id))
    }

    /// Sends a prompt; resolves with the turn result (`stopReason`) when the
    /// worker reports the turn end. Updates stream via `updates()` meanwhile.
    pub async fn prompt(&self, text: &str) -> Result<serde_json::Value, Error> {
        self.request(
            worker_methods::SESSION_PROMPT,
            serde_json::json!({ "text": text }),
        )
        .await
    }

    /// Cancels the in-flight turn.
    pub async fn cancel(&self) -> Result<(), Error> {
        self.request(worker_methods::SESSION_CANCEL, serde_json::json!({}))
            .await?;
        Ok(())
    }

    /// Stops the session: the worker tears down the adapter and exits.
    /// Returns the worker's exit code.
    pub async fn stop(&self) -> Result<Option<i32>, Error> {
        // The worker may already be gone; treat a closed channel as success.
        let _ = self
            .request(worker_methods::SESSION_STOP, serde_json::json!({}))
            .await;
        let mut child = self.child.lock().await;
        let code = child
            .wait()
            .await
            .map(|s| s.code())
            .map_err(|e| Error::Closed(format!("waiting on worker: {e}")))?;
        Ok(code)
    }

    /// Kills the worker process without the graceful stop handshake.
    pub async fn kill(&self) -> Result<(), Error> {
        let mut child = self.child.lock().await;
        child
            .kill()
            .await
            .map_err(|e| Error::Closed(format!("kill worker: {e}")))?;
        Ok(())
    }

    /// Answers a surfaced permission request (from `permissions()`).
    pub async fn resolve_permission(
        &self,
        request_id: &str,
        outcome: PermissionOutcome,
    ) -> Result<(), Error> {
        let frame = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "worker/permission_response",
            "params": {
                "requestId": request_id,
                "outcome": match &outcome {
                    PermissionOutcome::Selected(option_id) =>
                        serde_json::json!({ "outcome": "selected", "optionId": option_id }),
                    PermissionOutcome::Cancelled =>
                        serde_json::json!({ "outcome": "cancelled" }),
                },
            },
        });
        self.write_frame(&frame).await
    }

    /// Drain pending `session/update` params (raw JSON; typed at the UI
    /// layer, E2-11-4).
    pub async fn try_recv_update(&self) -> Option<serde_json::Value> {
        self.updates_rx.lock().await.try_recv().ok()
    }

    /// Next surfaced permission request, if any.
    pub async fn try_recv_permission(&self) -> Option<PermissionSurfaced> {
        self.permissions_rx.lock().await.try_recv().ok()
    }

    async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, Error> {
        let id = {
            let mut next = self.next_id.lock().await;
            let id = *next;
            *next += 1;
            id
        };
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
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
            .map_err(|_| Error::Closed("worker exited before responding".into()))?
    }

    async fn write_frame(&self, frame: &serde_json::Value) -> Result<(), Error> {
        let mut line = serde_json::to_vec(frame)
            .map_err(|e| Error::Protocol(format!("serialize frame: {e}")))?;
        line.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(&line)
            .await
            .map_err(|e| Error::Closed(format!("worker stdin: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| Error::Closed(format!("worker stdin flush: {e}")))?;
        Ok(())
    }
}

/// How the user resolved a permission request.
#[derive(Debug, Clone)]
pub enum PermissionOutcome {
    /// Chose one of the offered options.
    Selected(String),
    /// The turn was cancelled.
    Cancelled,
}

async fn reader_loop(
    stdout: tokio::process::ChildStdout,
    pending: Pending,
    updates_tx: mpsc::Sender<serde_json::Value>,
    permissions_tx: mpsc::Sender<PermissionSurfaced>,
) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let frame: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "malformed worker frame dropped");
                continue;
            }
        };
        let obj = match frame.as_object() {
            Some(o) => o,
            None => continue,
        };
        let id = obj.get("id").and_then(|v| v.as_u64());
        let has_method = obj.contains_key("method");

        match (id, has_method) {
            // Response to a host request.
            (Some(id), false) => {
                let outcome = if let Some(error) = obj.get("error") {
                    Err(Error::WorkerError {
                        code: error.get("code").and_then(|c| c.as_i64()).unwrap_or(0),
                        message: error
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("unknown worker error")
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
                }
            }
            // Notification from the worker.
            (None, true) => {
                let method = obj["method"].as_str().unwrap_or("");
                let params = obj.get("params").cloned().unwrap_or(serde_json::json!({}));
                match method {
                    worker_methods::WORKER_UPDATE => {
                        let _ = updates_tx.send(params).await;
                    }
                    worker_methods::WORKER_PERMISSION_REQUEST => {
                        let request_id = params["requestId"].as_str().unwrap_or("").to_string();
                        match serde_json::from_value(params["request"].clone()) {
                            Ok(request) => {
                                let _ = permissions_tx
                                    .send(PermissionSurfaced {
                                        request_id,
                                        request,
                                    })
                                    .await;
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "undecodable permission request dropped");
                            }
                        }
                    }
                    worker_methods::WORKER_EXITED => {
                        // Terminal signal; the child reaper in stop() reports
                        // the code.
                        break;
                    }
                    other => {
                        tracing::debug!(method = other, "unhandled worker notification");
                    }
                }
            }
            _ => {}
        }
    }
    // Worker stdout closed — fail any in-flight requests.
    for (_, tx) in pending.lock().await.drain() {
        let _ = tx.send(Err(Error::Closed("worker exited".into())));
    }
}
