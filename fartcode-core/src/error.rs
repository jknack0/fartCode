//! The single error enum used by every crate (ARCHITECTURE.md §3).
//!
//! Rules:
//! - Every public fallible function returns `Result<T, fartcode_core::Error>`.
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

    // -- Issues (E17) --
    #[error("issue not found: {0}")]
    IssueNotFound(String),

    #[error("invalid issue input: {0}")]
    InvalidIssueInput(String),

    #[error("blocked-by edge {from} blocked by {to} would create a dependency cycle")]
    IssueDependencyCycle { from: String, to: String },

    #[error("invalid proposal: {0}")]
    InvalidProposal(String),

    // -- Board columns (E18-01, ADR-0037) --
    #[error("board column not found: {0}")]
    BoardColumnNotFound(String),

    #[error("invalid board column input: {0}")]
    InvalidBoardColumnInput(String),

    /// Deleting a column that still has issues is rejected — cards must be
    /// moved first (no silent orphaning of the mirror pointer).
    #[error("board column {id} still has {count} issue(s); move them before deleting")]
    BoardColumnHasIssues { id: String, count: i64 },

    /// Deleting a column that is another column's `advance_to` target is
    /// refused (E18-07, #66). Letting the FK's `ON DELETE SET NULL`
    /// degrade the referrer would silently re-route `on_settle: advance`
    /// to next-by-position, which can walk cards into an adjacent
    /// agent step and fire an unconfirmed dispatch — the ADR-0037 item 4
    /// spend hazard. Repoint (or clear) the referrer first.
    #[error("column {id} is the advance target of {referrer} — repoint it first")]
    BoardColumnIsAdvanceTarget { id: String, referrer: String },

    /// `step_confirm` with nothing parked (never parked, already
    /// launched, cleared by a drag, or gone stale) — E18-04 queue flow.
    #[error("no parked step for issue {0}")]
    NoParkedStep(String),

    #[error("empty session id")]
    EmptySessionId,

    // -- Provider accounts (E3-07) --
    #[error("provider account not found: {0}")]
    ProviderAccountNotFound(String),

    #[error("credential store error: {0}")]
    CredentialStore(String),

    #[error("secret not found for credential_ref {0}")]
    CredentialSecretMissing(String),

    // -- I/O --
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    // -- File watching (E4-01) --
    #[error("file watch error: {0}")]
    Watch(String),

    // -- GitHub (E4-07/E4-09) --
    #[error("github error: {0}")]
    Github(String),

    #[error("github authentication required: {0}")]
    GithubAuth(String),

    /// 403/429 with the rate limit exhausted (or secondary limit). `reset_at`
    /// is the unix epoch second from `X-RateLimit-Reset` when present.
    #[error("github rate limit hit — try again later")]
    GithubRateLimited { reset_at: Option<i64> },

    #[error("pull request not found: {0}")]
    PullRequestNotFound(String),

    // -- Workspace files (E4-05) --
    #[error("path escapes the workspace: {0}")]
    PathEscape(String),

    // -- Line comments (E4-11 agent tool) --
    /// Malformed/out-of-range anchor, missing file, or a workspace that
    /// can't be resolved — the agent tool's guardrail errors.
    #[error("invalid line comment: {0}")]
    InvalidLineComment(String),

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
