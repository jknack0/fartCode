//! Client-side request handlers — the surface an adapter calls into:
//! permission requests, `fs/*` file access, `terminal/*`.
//!
//! The local impl scopes `fs/*` to the conversation's workspace root;
//! `terminal/*` returns MethodNotFound until agent-managed terminals land
//! (Phase 4). Implementations are expected to be quick — the adapter is
//! blocked on the response.
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use agent_client_protocol_schema::rpc::RequestId;
use agent_client_protocol_schema::v1::{
    ReadTextFileRequest, RequestPermissionRequest, RequestPermissionResponse, WriteTextFileRequest,
};

use crate::error::Error;
use crate::methods;
use crate::transport::StdioTransport;

/// A surfaced permission request. Resolve by answering the underlying
/// transport request with the user's choice (see `ClientHandlers::handle`).
#[derive(Debug)]
pub struct PermissionRequest {
    /// JSON-RPC id of the agent's `session/request_permission` request.
    pub request_id: RequestId,
    /// Parsed request (session, tool call, options).
    pub request: RequestPermissionRequest,
}

/// How the caller resolved a permission request.
#[derive(Debug, Clone)]
pub enum PermissionDecision {
    /// The user picked one of the offered options.
    Selected(String),
    /// The prompt turn was cancelled; the spec requires the `cancelled`
    /// outcome in that case.
    Cancelled,
}

/// Handles agent → client requests. Implement to drive a real UI; the
/// default [`LocalHandlers`] covers fs + stub terminals.
pub trait ClientHandlers: Send + Sync {
    /// Workspace root for scoping `fs/*` paths (conversation worktree).
    fn workspace_root(&self) -> Option<PathBuf>;

    /// Called for `session/request_permission`.
    fn permission_requested(&self, request: PermissionRequest);

    /// Reads a workspace file for the agent. Default: scoped local read.
    fn read_text_file(&self, request: &ReadTextFileRequest) -> Result<String, Error> {
        let path = scoped_path(self.workspace_root().as_deref(), &request.path)?;
        std::fs::read_to_string(&path)
            .map_err(|e| Error::Protocol(format!("read {}: {e}", path.display())))
    }

    /// Writes a workspace file for the agent. Default: scoped local write.
    fn write_text_file(&self, request: &WriteTextFileRequest) -> Result<(), Error> {
        let path = scoped_path(self.workspace_root().as_deref(), &request.path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Protocol(format!("create_dir_all: {e}")))?;
        }
        std::fs::write(&path, &request.content)
            .map_err(|e| Error::Protocol(format!("write {}: {e}", path.display())))
    }

    /// Called for `terminal/*`. Default: unsupported (Phase 4 feature).
    fn terminal_request(&self, method: &str) -> Result<serde_json::Value, Error> {
        Err(Error::AgentError {
            code: -32601,
            message: format!("{method} not supported by this client"),
        })
    }
}

/// Default handlers: workspace-scoped local fs; terminals unsupported.
///
/// `permission_resolver` is invoked for permission requests; it should
/// eventually produce a [`PermissionDecision`] via whatever UI exists. The
/// default records the request and answers `cancelled` only if the resolver
/// says so — the provided resolver receives the request and an answer
/// callback wired to the transport.
pub struct LocalHandlers {
    root: Option<PathBuf>,
    resolver: Arc<dyn Fn(PermissionRequest, Arc<PermissionAnswerer>) + Send + Sync>,
}

/// Answers a pending permission request through the transport.
pub struct PermissionAnswerer {
    transport: Arc<StdioTransport>,
    request_id: RequestId,
}

impl PermissionAnswerer {
    /// Sends the user's decision back to the agent.
    pub async fn answer(&self, decision: PermissionDecision) -> Result<(), Error> {
        let response = match decision {
            PermissionDecision::Selected(option_id) => {
                let outcome = agent_client_protocol_schema::v1::RequestPermissionOutcome::Selected(
                    agent_client_protocol_schema::v1::SelectedPermissionOutcome::new(
                        agent_client_protocol_schema::v1::PermissionOptionId::new(option_id),
                    ),
                );
                RequestPermissionResponse::new(outcome)
            }
            PermissionDecision::Cancelled => RequestPermissionResponse::new(
                agent_client_protocol_schema::v1::RequestPermissionOutcome::Cancelled,
            ),
        };
        let value = serde_json::to_value(&response)
            .map_err(|e| Error::Protocol(format!("serialize permission response: {e}")))?;
        self.transport.respond(self.request_id.clone(), value).await
    }
}

impl LocalHandlers {
    /// Builds handlers rooted at `workspace_root` (may be None before the
    /// conversation's worktree is known — fs requests then fail).
    pub fn new(
        workspace_root: Option<PathBuf>,
        resolver: impl Fn(PermissionRequest, Arc<PermissionAnswerer>) + Send + Sync + 'static,
    ) -> Self {
        Self {
            root: workspace_root,
            resolver: Arc::new(resolver),
        }
    }

    /// Dispatches one agent request, answering via the transport.
    pub async fn handle(
        &self,
        transport: &Arc<StdioTransport>,
        request_id: RequestId,
        method: &str,
        params: serde_json::Value,
    ) {
        match method {
            methods::SESSION_REQUEST_PERMISSION => {
                let parsed: RequestPermissionRequest = match serde_json::from_value(params) {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = transport
                            .respond_error(request_id, -32602, &format!("invalid params: {e}"))
                            .await;
                        return;
                    }
                };
                (self.resolver)(
                    PermissionRequest {
                        request_id: request_id.clone(),
                        request: parsed,
                    },
                    Arc::new(PermissionAnswerer {
                        transport: Arc::clone(transport),
                        request_id,
                    }),
                );
            }
            methods::FS_READ_TEXT_FILE => {
                let parsed: ReadTextFileRequest = match serde_json::from_value(params) {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = transport
                            .respond_error(request_id, -32602, &format!("invalid params: {e}"))
                            .await;
                        return;
                    }
                };
                match self.read_text_file(&parsed) {
                    Ok(content) => {
                        let response =
                            agent_client_protocol_schema::v1::ReadTextFileResponse::new(content);
                        let value = serde_json::to_value(&response).unwrap_or_default();
                        let _ = transport.respond(request_id, value).await;
                    }
                    Err(e) => {
                        let _ = transport
                            .respond_error(request_id, -32602, &e.to_string())
                            .await;
                    }
                }
            }
            methods::FS_WRITE_TEXT_FILE => {
                let parsed: WriteTextFileRequest = match serde_json::from_value(params) {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = transport
                            .respond_error(request_id, -32602, &format!("invalid params: {e}"))
                            .await;
                        return;
                    }
                };
                match self.write_text_file(&parsed) {
                    Ok(()) => {
                        let response =
                            agent_client_protocol_schema::v1::WriteTextFileResponse::new();
                        let value = serde_json::to_value(&response).unwrap_or_default();
                        let _ = transport.respond(request_id, value).await;
                    }
                    Err(e) => {
                        let _ = transport
                            .respond_error(request_id, -32602, &e.to_string())
                            .await;
                    }
                }
            }
            methods::TERMINAL_CREATE
            | methods::TERMINAL_OUTPUT
            | methods::TERMINAL_RELEASE
            | methods::TERMINAL_WAIT_FOR_EXIT
            | methods::TERMINAL_KILL => match self.terminal_request(method) {
                Ok(value) => {
                    let _ = transport.respond(request_id, value).await;
                }
                Err(Error::AgentError { code, message }) => {
                    let _ = transport.respond_error(request_id, code, &message).await;
                }
                Err(e) => {
                    let _ = transport
                        .respond_error(request_id, -32603, &e.to_string())
                        .await;
                }
            },
            other => {
                let _ = transport
                    .respond_error(request_id, -32601, &format!("unknown method {other}"))
                    .await;
            }
        }
    }
}

impl ClientHandlers for LocalHandlers {
    fn workspace_root(&self) -> Option<PathBuf> {
        self.root.clone()
    }

    fn permission_requested(&self, _request: PermissionRequest) {
        // The resolver closure in `handle` owns surfacing; this trait hook
        // exists for implementations that dispatch elsewhere.
    }
}

/// Resolves `path` (absolute, agent-supplied) inside `root`, rejecting
/// traversal (`..`) and paths outside the workspace. ACP requires absolute
/// paths; we additionally contain them (PRD §7: worktree path validation).
fn scoped_path(root: Option<&Path>, path: &Path) -> Result<PathBuf, Error> {
    let root = root.ok_or_else(|| Error::Protocol("no workspace root configured".into()))?;
    if !path.is_absolute() {
        return Err(Error::Protocol(format!(
            "agent-supplied path is not absolute: {}",
            path.display()
        )));
    }
    // Normalize away any `..` components defensively before containment.
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(Error::Protocol(format!(
            "agent-supplied path escapes workspace: {}",
            path.display()
        )));
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|e| Error::Protocol(format!("canonicalize root: {e}")))?;
    // The file may not exist yet (write_text_file creates it), so canonicalize
    // the deepest existing ancestor + remainder instead.
    let mut anchor = path.to_path_buf();
    let mut tail = Vec::new();
    loop {
        if anchor.exists() {
            break;
        }
        match anchor.file_name() {
            Some(name) => {
                tail.push(name.to_os_string());
                anchor.pop();
            }
            None => {
                return Err(Error::Protocol(format!(
                    "agent-supplied path has no existing ancestor: {}",
                    path.display()
                )));
            }
        }
    }
    let canonical_anchor = anchor
        .canonicalize()
        .map_err(|e| Error::Protocol(format!("canonicalize path: {e}")))?;
    let mut resolved = canonical_anchor;
    for name in tail.into_iter().rev() {
        resolved.push(name);
    }
    if !resolved.starts_with(&canonical_root) {
        return Err(Error::Protocol(format!(
            "agent-supplied path outside workspace root: {}",
            path.display()
        )));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_path_accepts_workspace_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = root.join("src/main.rs");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(&file, "fn main() {}").unwrap();

        let resolved = scoped_path(Some(root), &file).unwrap();
        assert!(resolved.starts_with(root.canonicalize().unwrap()));
    }

    #[test]
    fn scoped_path_rejects_traversal_and_outside() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        assert!(scoped_path(Some(root), &root.join("..").join("etc").join("passwd")).is_err());
        assert!(scoped_path(Some(root), Path::new("/etc/passwd")).is_err());
        assert!(scoped_path(None, Path::new("/tmp/x")).is_err());
        assert!(scoped_path(Some(root), Path::new("relative/path")).is_err());
    }

    #[test]
    fn scoped_path_allows_new_file_under_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target = root.join("new/nested/file.txt");
        assert!(scoped_path(Some(root), &target).is_ok());
    }
}
