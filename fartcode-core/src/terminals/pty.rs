//! PTY abstraction (E1-06/E2-06).
//!
//! The trait lives here so `fartcode-core` services (lifecycle scripts, later the
//! agent launcher) can drive a terminal without depending on `fartcode-terminal`.
//! The concrete impl is `fartcode-terminal::PortablePtyManager` (portable-pty),
//! wired in `fartcode-app` (ARCHITECTURE §7; see ADR-0014 for the trait-location
//! deviation from the reference's `fartcode_terminal::PtyManager`).

use std::path::Path;
use std::time::Duration;

use crate::Error;

/// Terminal geometry (portable-pty `PtySize` equivalent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

impl Default for PtySize {
    fn default() -> Self {
        Self { rows: 24, cols: 80 }
    }
}

/// How a PTY child ended. `signal` is the signal name when killed by a
/// signal (reference `PtyExitInfo.signal`), `None` otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyExit {
    pub exit_code: Option<u32>,
    pub signal: Option<String>,
}

impl PtyExit {
    /// Reference success rule: exit code 0/undefined AND no signal.
    pub fn is_success(&self) -> bool {
        self.signal.is_none() && matches!(self.exit_code, None | Some(0))
    }
}

/// A running PTY session (shell child + master fd).
pub trait PtyHandle: Send {
    /// Writes raw bytes to the PTY master (typed input).
    fn write(&mut self, data: &str) -> Result<(), Error>;
    /// Drains whatever the background reader has accumulated; `Ok(true)`
    /// means `buf` gained bytes this call.
    fn try_read(&mut self, buf: &mut Vec<u8>) -> Result<bool, Error>;
    /// Blocks until the child exits; `Err(TimedOut)` after `timeout`.
    fn wait_exit(&mut self, timeout: Duration) -> Result<PtyExit, Error>;
    /// Resizes the pty (E2-06): cols clamped to >= 2, rows to >= 1 (the
    /// reference's resize clamping — agents misbehave below those).
    fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Error>;
    /// Polls the child's exit status without blocking (E2-06 keystroke pump
    /// uses it to stop early when the agent dies before producing output).
    fn try_wait_exit(&mut self) -> Result<Option<PtyExit>, Error>;
    /// Blocks until the background reader has drained everything the child
    /// wrote before exiting (call after `wait_exit` to avoid a tail race).
    fn flush(&mut self) -> Result<(), Error>;
    /// Force-kills the child (timeout path / teardown); reaps it.
    fn kill(&mut self) -> Result<(), Error>;
}

/// A remote launch route: the host's PTY manager plus the `workspaces.id`
/// the task runs in (exported to the agent as `REMOTE_WORKSPACE_ID`).
pub type RemotePtyRoute = (std::sync::Arc<dyn PtyManager>, String);

/// Resolves the remote PTY manager for a task (E12-05).
///
/// Lives here so `fartcode-core` services (terminals, the agent launcher) can
/// route to an SSH host without depending on `fartcode-ssh`; the
/// implementation is `fartcode_app::remote_pty::RemotePtyRegistry`.
pub trait RemotePtyLookup: Send + Sync {
    /// `Some((manager, workspace_id))` when the task's workspace is remote,
    /// `None` when it is local. An `Err` means the task IS remote and the
    /// host could not be reached — callers must not fall back to local.
    fn resolve(&self, task_id: &str) -> Result<Option<RemotePtyRoute>, Error>;
}

/// How the child's env is built (E2-06 security: the agent allowlist is
/// only meaningful if the child does NOT inherit the parent env).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvPolicy {
    /// Inherit the parent env, overlaying the given pairs (shells,
    /// lifecycle scripts — reference parity).
    Inherit,
    /// `env_clear` first: the child sees ONLY the given pairs (agent
    /// launches — the E3-08 allowlist is the sole env source).
    AllowlistedOnly,
}

/// Spawns shell PTYs (object-safe — services hold `Arc<dyn PtyManager>`).
pub trait PtyManager: Send + Sync {
    /// Spawns `cmd` with `args` in `cwd` with `env` (see `EnvPolicy`),
    /// attached to a fresh pty. `remove` lists env vars stripped from the
    /// child AFTER env application (login-account scrubbing, ADR-0034:
    /// a stray `ANTHROPIC_API_KEY` flips the claude CLI to API billing).
    #[allow(clippy::too_many_arguments)]
    fn spawn(
        &self,
        cmd: &str,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
        size: PtySize,
        env_policy: EnvPolicy,
        remove: &[String],
    ) -> Result<Box<dyn PtyHandle>, Error>;
}
