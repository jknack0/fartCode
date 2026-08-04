//! The single error enum used by every crate (ARCHITECTURE.md §3).
//!
//! Rules:
//! - Every public fallible function returns `Result<T, ade_core::Error>`.
//! - If a domain needs a new variant, add it here — do not create per-domain error types.
//! - `Internal(String)` is the escape hatch for one-off messages during prototyping;
//!   refactor into a named variant before merging.

use std::path::PathBuf;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    // -- Database --
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("migration failed: {0}")]
    Migration(String),

    #[error("versioned JSON parse failed for column {column}: {reason}")]
    VersionedJson { column: String, reason: String },

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    // -- Settings --
    #[error("invalid setting key: {0}")]
    InvalidSettingKey(String),

    #[error("invalid setting value for {key}: {reason}")]
    InvalidSettingValue { key: String, reason: String },

    // -- Projects --
    #[error("project not found: {0}")]
    ProjectNotFound(String),

    #[error("project path already registered: {0}")]
    DuplicateProjectPath(PathBuf),

    #[error("project path does not exist: {0}")]
    ProjectPathNotFound(PathBuf),

    // -- Tasks --
    #[error("invalid task input: {0}")]
    InvalidTaskInput(String),
    /// E1-05: worktree directory must be absolute (posix / win drive/UNC);
    /// `~` expands via the home dir.
    #[error("invalid-worktree-directory: {0}")]
    InvalidWorktreeDirectory(String),

    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("invalid task status transition: {from} -> {to}")]
    InvalidStatusTransition { from: String, to: String },

    // -- Worktrees/Git --
    #[error("git error: {0}")]
    Git(String),

    /// Cleanup git ops (e.g. `worktree prune` on task deletion) run with a
    /// bounded timeout so a wedged git can never hang teardown.
    #[error("git operation timed out: {0}")]
    GitTimeout(String),

    #[error("worktree path already exists: {0}")]
    WorktreeExists(PathBuf),

    #[error("worktree exists at the expected path but is checked out on a different branch: {0}")]
    WorktreeBranchConflict(String),

    #[error("cannot remove project root workspace")]
    CannotRemoveProjectRoot,

    // -- PTY --
    #[error("PTY error: {0}")]
    Pty(String),
    /// Lifecycle script hit its `timeoutMs` (E1-06).
    #[error("lifecycle script timed out: {0}")]
    LifecycleScriptTimeout(String),
    /// Worktree has uncommitted changes (E2-07 follow-up: dirty-check before
    /// removal prevents data loss of agent work).
    #[error("worktree has uncommitted changes: {0}")]
    DirtyWorktree(String),
    /// Lifecycle script exited non-zero when `surfaceFailure` is set.
    #[error("lifecycle script failed: session {session_id} exit={exit_code:?} signal={signal:?}")]
    LifecycleScriptFailed {
        session_id: String,
        exit_code: Option<u32>,
        signal: Option<String>,
        output_tail: String,
    },

    /// A running PTY session was cancelled by teardown (E2-09 task deletion).
    #[error("session cancelled")]
    SessionCancelled,

    #[error("agent executable not found: {0}")]
    AgentNotFound(String),

    #[error("agent exited with non-zero status: {exit_code}")]
    AgentExited { exit_code: i32 },

    // -- Conversations --
    #[error("conversation not found: {0}")]
    ConversationNotFound(String),

    #[error("empty session id")]
    EmptySessionId,

    // -- I/O --
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    // -- Catch-all --
    #[error("{0}")]
    Internal(String),
}

// Tauri commands return Result<T, String>, so we need this conversion:
impl From<Error> for String {
    fn from(e: Error) -> String {
        e.to_string()
    }
}
