//! ade-acp — ACP (Agent Client Protocol) client.
//!
//! Speaks the Agent Client Protocol (JSON-RPC over stdio) to provider
//! adapter binaries. Wire types come from the official
//! `agent-client-protocol-schema` crate (ADR-0024); this crate owns the
//! transport, the client-side request handlers, and the session lifecycle.
//!
//! - [`transport::StdioTransport`] — newline-delimited JSON-RPC over a child
//!   process's stdin/stdout, with pending-request correlation and an update
//!   fan-out stream.
//! - [`client::AcpClient`] — lifecycle methods (`initialize`, `new_session`,
//!   `load_session`, `prompt`, `cancel`, `set_mode`, `set_config_option`).
//! - [`handlers::ClientHandlers`] — the adapter-side surface: permission
//!   requests, `fs/*` file access, `terminal/*` (handler trait; the local
//!   impl is Phase-2/4 territory).

pub mod client;
pub mod error;
pub mod handlers;
pub mod session;
pub mod transport;

pub use client::{AcpClient, InitializeResult, PromptResult, SessionUpdateEvent};
pub use error::Error;
pub use handlers::{
    ClientHandlers, LocalHandlers, PermissionAnswerer, PermissionDecision, PermissionRequest,
};
pub use session::{
    Lifecycle, QueuedPrompt, SessionCell, SessionIdStore, SessionManager, StartInput, StartOutcome,
    Turn, TurnOutcome,
};
pub use transport::{Incoming, StdioTransport};

/// ACP method names (stable strings from the v1 spec; the schema crate keeps
/// its constants `pub(crate)`).
pub mod methods {
    /// Client → agent.
    pub const INITIALIZE: &str = "initialize";
    pub const SESSION_NEW: &str = "session/new";
    pub const SESSION_LOAD: &str = "session/load";
    pub const SESSION_RESUME: &str = "session/resume";
    pub const SESSION_PROMPT: &str = "session/prompt";
    pub const SESSION_CANCEL: &str = "session/cancel";
    pub const SESSION_SET_MODE: &str = "session/set_mode";
    pub const SESSION_SET_CONFIG_OPTION: &str = "session/set_config_option";

    /// Agent → client.
    pub const SESSION_UPDATE: &str = "session/update";
    pub const SESSION_REQUEST_PERMISSION: &str = "session/request_permission";
    pub const FS_READ_TEXT_FILE: &str = "fs/read_text_file";
    pub const FS_WRITE_TEXT_FILE: &str = "fs/write_text_file";
    pub const TERMINAL_CREATE: &str = "terminal/create";
    pub const TERMINAL_OUTPUT: &str = "terminal/output";
    pub const TERMINAL_RELEASE: &str = "terminal/release";
    pub const TERMINAL_WAIT_FOR_EXIT: &str = "terminal/wait_for_exit";
    pub const TERMINAL_KILL: &str = "terminal/kill";
}

/// The protocol major version this client negotiates (ACP v1).
pub const PROTOCOL_VERSION: agent_client_protocol_schema::ProtocolVersion =
    agent_client_protocol_schema::ProtocolVersion::V1;
