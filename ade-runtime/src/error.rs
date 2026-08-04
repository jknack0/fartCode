//! ade-runtime error type.

/// Errors from the worker host and process hosts.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Spawning a process failed (worker or adapter).
    #[error("spawn failed: {0}")]
    Spawn(String),

    /// I/O on the worker/adapter stdio.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Malformed frame or protocol violation on the worker channel.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// The worker or adapter exited; carries the exit code when known.
    #[error("process exited ({detail})")]
    Exited {
        /// Exit code or a description when unavailable.
        detail: String,
    },

    /// The worker returned a JSON-RPC error for a request.
    #[error("worker error {code}: {message}")]
    WorkerError {
        /// JSON-RPC error code.
        code: i64,
        /// Error message.
        message: String,
    },

    /// The worker is busy (one session per worker in v1).
    #[error("worker busy: {0}")]
    Busy(String),

    /// The worker connection is closed.
    #[error("worker closed: {0}")]
    Closed(String),

    /// Not implemented yet (typed, not a panic).
    #[error("unsupported: {0}")]
    Unsupported(String),
}
