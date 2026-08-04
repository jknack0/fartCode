//! ACP client: session lifecycle over a [`StdioTransport`].
//!
//! The client owns the initialize handshake, session creation/resume,
//! prompt turns (collecting the `session/update` stream while the turn is
//! in flight), cancel, mode, and config-option changes.

use std::path::Path;
use std::sync::Arc;

use agent_client_protocol_schema::v1::{
    ClientCapabilities, FileSystemCapabilities, Implementation, InitializeRequest,
    InitializeResponse, LoadSessionRequest, LoadSessionResponse, NewSessionRequest,
    NewSessionResponse, PromptRequest, PromptResponse, SessionId, SessionNotification,
    SessionUpdate, SetSessionConfigOptionRequest, SetSessionConfigOptionResponse,
    SetSessionModeRequest, SetSessionModeResponse,
};
use tokio::sync::{broadcast, mpsc, Mutex};

use crate::error::Error;
use crate::handlers::LocalHandlers;
use crate::transport::{Incoming, StdioTransport};
use crate::{methods, PROTOCOL_VERSION};

/// Result of the `initialize` handshake.
#[derive(Debug, Clone)]
pub struct InitializeResult {
    /// Negotiated protocol version (must equal [`PROTOCOL_VERSION`]).
    pub protocol_version: u16,
    /// Full parsed response (capabilities, auth methods, agent info).
    pub response: InitializeResponse,
}

/// One `session/update` delivered for a session, typed.
#[derive(Debug, Clone)]
pub struct SessionUpdateEvent {
    /// Session the update belongs to.
    pub session_id: SessionId,
    /// The update payload.
    pub update: SessionUpdate,
}

/// Result of a completed prompt turn.
#[derive(Debug, Clone)]
pub struct PromptResult {
    /// Parsed `session/prompt` response (carries the stop reason).
    pub response: PromptResponse,
    /// Every `session/update` received while the turn was in flight, in
    /// arrival order.
    pub updates: Vec<SessionUpdateEvent>,
}

/// ACP client bound to one adapter process.
pub struct AcpClient {
    transport: Arc<StdioTransport>,
    updates_rx: broadcast::Receiver<serde_json::Value>,
    events_rx: Mutex<mpsc::Receiver<Incoming>>,
    handlers: Arc<LocalHandlers>,
}

impl AcpClient {
    /// Spawns the adapter and wires the transport. `handlers_root` is the
    /// conversation workspace root for `fs/*` scoping; `permission_resolver`
    /// receives surfaced permission requests.
    pub async fn spawn(
        program: impl AsRef<std::ffi::OsStr>,
        args: &[String],
        env: &[(String, String)],
        cwd: Option<&Path>,
        handlers_root: Option<std::path::PathBuf>,
        permission_resolver: impl Fn(crate::handlers::PermissionRequest, Arc<crate::handlers::PermissionAnswerer>)
            + Send
            + Sync
            + 'static,
    ) -> Result<Self, Error> {
        let (updates_tx, updates_rx) = broadcast::channel(256);
        let (events_tx, events_rx) = mpsc::channel(64);
        let transport =
            Arc::new(StdioTransport::spawn(program, args, env, cwd, updates_tx, events_tx).await?);
        Ok(Self {
            transport,
            updates_rx,
            events_rx: Mutex::new(events_rx),
            handlers: Arc::new(LocalHandlers::new(handlers_root, permission_resolver)),
        })
    }

    /// Adopts a transport built elsewhere (E2-11-2: the worker's process
    /// hosts spawn the adapter and hand the child to the client).
    pub fn from_transport(
        transport: Arc<StdioTransport>,
        updates_rx: broadcast::Receiver<serde_json::Value>,
        events_rx: mpsc::Receiver<Incoming>,
        handlers_root: Option<std::path::PathBuf>,
        permission_resolver: impl Fn(crate::handlers::PermissionRequest, Arc<crate::handlers::PermissionAnswerer>)
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            transport,
            updates_rx,
            events_rx: Mutex::new(events_rx),
            handlers: Arc::new(LocalHandlers::new(handlers_root, permission_resolver)),
        }
    }

    /// Performs the `initialize` handshake, rejecting version mismatches.
    pub async fn initialize(&self) -> Result<InitializeResult, Error> {
        let request = InitializeRequest::new(PROTOCOL_VERSION)
            .client_capabilities(client_capabilities())
            .client_info(Implementation::new("ade", env!("CARGO_PKG_VERSION")));
        let params = serde_json::to_value(&request)
            .map_err(|e| Error::Protocol(format!("serialize initialize: {e}")))?;
        let raw = self.transport.request(methods::INITIALIZE, params).await?;
        let response: InitializeResponse = serde_json::from_value(raw)
            .map_err(|e| Error::Protocol(format!("initialize response: {e}")))?;
        let agent_version = response.protocol_version.as_u16();
        if agent_version != PROTOCOL_VERSION.as_u16() {
            return Err(Error::VersionMismatch {
                agent: agent_version,
                client: PROTOCOL_VERSION.as_u16(),
            });
        }
        Ok(InitializeResult {
            protocol_version: agent_version,
            response,
        })
    }

    /// Creates a session (`session/new`) in `cwd`.
    pub async fn new_session(&self, cwd: &Path) -> Result<SessionId, Error> {
        let request = NewSessionRequest::new(cwd.to_path_buf());
        let params = serde_json::to_value(&request)
            .map_err(|e| Error::Protocol(format!("serialize session/new: {e}")))?;
        let raw = self.transport.request(methods::SESSION_NEW, params).await?;
        let response: NewSessionResponse = serde_json::from_value(raw)
            .map_err(|e| Error::Protocol(format!("session/new response: {e}")))?;
        Ok(response.session_id)
    }

    /// Resumes a session (`session/load`). The agent replays history as
    /// `session/update` notifications before responding; this call streams
    /// those into the returned updates alongside the final response.
    pub async fn load_session(
        &self,
        session_id: &SessionId,
        cwd: &Path,
    ) -> Result<(LoadSessionResponse, Vec<SessionUpdateEvent>), Error> {
        let request = LoadSessionRequest::new(session_id.clone(), cwd.to_path_buf());
        let params = serde_json::to_value(&request)
            .map_err(|e| Error::Protocol(format!("serialize session/load: {e}")))?;
        let mut stream = self.updates_rx.resubscribe();
        let raw = self
            .transport
            .request(methods::SESSION_LOAD, params)
            .await?;
        let response: LoadSessionResponse = serde_json::from_value(raw)
            .map_err(|e| Error::Protocol(format!("session/load response: {e}")))?;
        let updates = drain_updates(&mut stream, session_id);
        Ok((response, updates))
    }

    /// Sends a prompt and awaits the turn end, collecting streamed updates.
    pub async fn prompt(
        &self,
        session_id: &SessionId,
        content_blocks: Vec<agent_client_protocol_schema::v1::ContentBlock>,
    ) -> Result<PromptResult, Error> {
        let request = PromptRequest::new(session_id.clone(), content_blocks);
        let params = serde_json::to_value(&request)
            .map_err(|e| Error::Protocol(format!("serialize session/prompt: {e}")))?;

        let mut stream = self.updates_rx.resubscribe();
        let raw = self
            .transport
            .request(methods::SESSION_PROMPT, params)
            .await?;
        let response: PromptResponse = serde_json::from_value(raw)
            .map_err(|e| Error::Protocol(format!("session/prompt response: {e}")))?;
        let updates = drain_updates(&mut stream, session_id);
        Ok(PromptResult { response, updates })
    }

    /// Cancels the in-flight turn (`session/cancel` notification).
    pub async fn cancel(&self, session_id: &SessionId) -> Result<(), Error> {
        let params = serde_json::json!({ "sessionId": session_id });
        self.transport.notify(methods::SESSION_CANCEL, params).await
    }

    /// Switches the session mode (`session/set_mode`).
    pub async fn set_mode(
        &self,
        session_id: &SessionId,
        mode_id: &str,
    ) -> Result<SetSessionModeResponse, Error> {
        let request = SetSessionModeRequest::new(session_id.clone(), mode_id.to_string());
        let params = serde_json::to_value(&request)
            .map_err(|e| Error::Protocol(format!("serialize set_mode: {e}")))?;
        let raw = self
            .transport
            .request(methods::SESSION_SET_MODE, params)
            .await?;
        serde_json::from_value(raw).map_err(|e| Error::Protocol(format!("set_mode response: {e}")))
    }

    /// Sets a session config option (`session/set_config_option`).
    pub async fn set_config_option(
        &self,
        session_id: &SessionId,
        config_id: &str,
        value: agent_client_protocol_schema::v1::SessionConfigOptionValue,
    ) -> Result<SetSessionConfigOptionResponse, Error> {
        let request =
            SetSessionConfigOptionRequest::new(session_id.clone(), config_id.to_string(), value);
        let params = serde_json::to_value(&request)
            .map_err(|e| Error::Protocol(format!("serialize set_config_option: {e}")))?;
        let raw = self
            .transport
            .request(methods::SESSION_SET_CONFIG_OPTION, params)
            .await?;
        serde_json::from_value(raw)
            .map_err(|e| Error::Protocol(format!("set_config_option response: {e}")))
    }

    /// Subscribes to raw `session/update` params (e.g. for a live UI feed).
    pub fn updates(&self) -> broadcast::Receiver<serde_json::Value> {
        self.updates_rx.resubscribe()
    }

    /// Drains pending agent requests and the closed signal; returns when the
    /// adapter exits. Spawn once (e.g. `tokio::spawn` on an `Arc<AcpClient>`);
    /// permission/fs/terminal requests are answered through the handlers.
    pub async fn run_event_loop(&self) {
        let mut events = self.events_rx.lock().await;
        while let Some(event) = events.recv().await {
            match event {
                Incoming::AgentRequest(req) => {
                    self.handlers
                        .handle(&self.transport, req.id, &req.method, req.params)
                        .await;
                }
                Incoming::Closed => break,
            }
        }
    }

    /// Terminates the adapter.
    pub async fn shutdown(&self) {
        self.transport.shutdown().await;
    }
}

/// Client capabilities we advertise: scoped fs read/write; no terminals yet.
fn client_capabilities() -> ClientCapabilities {
    let mut caps = ClientCapabilities::new();
    let mut fs = FileSystemCapabilities::new();
    fs.read_text_file = true;
    fs.write_text_file = true;
    caps.fs = fs;
    caps.terminal = false;
    caps
}

/// Collects the updates buffered on `stream` for `session_id` (non-blocking;
/// the transport queues them while the awaited request was in flight).
fn drain_updates(
    stream: &mut broadcast::Receiver<serde_json::Value>,
    session_id: &SessionId,
) -> Vec<SessionUpdateEvent> {
    let mut out = Vec::new();
    while let Ok(raw) = stream.try_recv() {
        let notification: SessionNotification = match serde_json::from_value(raw) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(error = %e, "undecodable session/update dropped");
                continue;
            }
        };
        if notification.session_id != *session_id {
            continue;
        }
        out.push(SessionUpdateEvent {
            session_id: notification.session_id,
            update: notification.update,
        });
    }
    out
}
