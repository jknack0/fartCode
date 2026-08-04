//! ACP client error type.

/// Errors from the ACP transport and client.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Spawning the adapter process failed.
    #[error("adapter spawn failed: {0}")]
    Spawn(#[from] std::io::Error),

    /// Protocol-level violation (unexpected frame shape, bad handshake).
    #[error("protocol error: {0}")]
    Protocol(String),

    /// The adapter returned a JSON-RPC error for a request.
    #[error("agent error {code}: {message}")]
    AgentError {
        /// JSON-RPC error code.
        code: i32,
        /// Error message from the agent.
        message: String,
    },

    /// The agent negotiated a protocol version this client cannot speak.
    #[error("protocol version mismatch: agent offered v{agent}, client speaks v{client}")]
    VersionMismatch {
        /// Agent's protocol major version.
        agent: u16,
        /// Client's protocol major version.
        client: u16,
    },

    /// The adapter exited / the transport is closed.
    #[error("connection closed: {0}")]
    Closed(String),
}
