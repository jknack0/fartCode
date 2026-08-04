//! Adapter process hosts (E2-11-2): where the adapter binary gets spawned.
//!
//! Local now; the SSH host is the Phase-3 seam (remote spawn over the
//! workspace-server `acp` contract domain). Exit codes surface as
//! [`Error::Exited`]; [`RestartPolicy`] bounds crash restarts with backoff.

use std::future::Future;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::error::Error;

/// Everything needed to spawn one adapter process.
#[derive(Debug, Clone)]
pub struct AdapterProcessConfig {
    /// Adapter binary (e.g. a `claude-agent-acp` wrapper).
    pub program: PathBuf,
    /// Adapter argv.
    pub args: Vec<String>,
    /// Env for the adapter — server-resolved only (E3-07). The renderer can
    /// never contribute entries here (see `protocol::prepare_session`).
    pub env: Vec<(String, String)>,
    /// Working directory.
    pub cwd: PathBuf,
}

/// Crash-restart policy for a supervised adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    /// No restart.
    Never,
    /// Restart on non-zero exit up to `max_attempts` times.
    OnFailure {
        /// Maximum restart attempts.
        max_attempts: u32,
    },
}

impl RestartPolicy {
    /// Backoff before the `attempt`-th restart (1-indexed): 500ms doubling,
    /// capped at 5s.
    pub fn backoff(attempt: u32) -> Duration {
        let ms = 500u64.saturating_mul(1u64 << attempt.saturating_sub(1).min(10));
        Duration::from_millis(ms.min(5000))
    }
}

/// Spawns adapter processes. Local impl now; SSH is the Phase-3 seam.
pub trait ProcessHost: Send + Sync {
    /// Spawns the adapter with piped stdio. The child MUST be left with
    /// stdin/stdout piped (the ACP transport adopts them).
    fn spawn(&self, config: &AdapterProcessConfig) -> Result<tokio::process::Child, Error>;

    /// Host description for logs/errors.
    fn name(&self) -> &'static str;
}

/// Spawns adapters on the local machine.
pub struct LocalProcessHost;

impl ProcessHost for LocalProcessHost {
    fn spawn(&self, config: &AdapterProcessConfig) -> Result<tokio::process::Child, Error> {
        let mut cmd = Command::new(&config.program);
        cmd.args(&config.args)
            .envs(config.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .current_dir(&config.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Adapter stderr goes to the worker's stderr (host-visible log
            // stream); never mixed into the stdout JSON-RPC channel.
            .stderr(Stdio::inherit());
        cmd.spawn()
            .map_err(|e| Error::Spawn(format!("{}: {}", config.program.display(), e)))
    }

    fn name(&self) -> &'static str {
        "local"
    }
}

/// SSH process host — Phase-3 seam (remote spawn over the workspace-server
/// `acp` contract domain, PRD §4.7). Present now so call sites are already
/// host-shaped.
pub struct SshProcessHost;

impl ProcessHost for SshProcessHost {
    fn spawn(&self, _config: &AdapterProcessConfig) -> Result<tokio::process::Child, Error> {
        Err(Error::Unsupported(
            "remote ACP process hosts arrive with the Phase-3 workspace-server".into(),
        ))
    }

    fn name(&self) -> &'static str {
        "ssh"
    }
}

/// Result of a supervised adapter lifecycle.
#[derive(Debug)]
pub struct SupervisedExit {
    /// Final exit code (None when the OS provides none).
    pub exit_code: Option<i32>,
    /// Total spawn attempts used (1 = no restarts).
    pub attempts: u32,
}

/// Spawns `config` through `host`, respawning on non-zero exit per
/// `policy`. `run` drives one adapter lifetime (typically until the ACP
/// transport closes) and returns that child's exit code.
///
/// Restart semantics: a zero exit ends the supervision; a non-zero exit
/// respawns after backoff until attempts are exhausted, then the last exit
/// code is returned.
pub async fn run_supervised(
    host: &dyn ProcessHost,
    config: &AdapterProcessConfig,
    policy: RestartPolicy,
    mut run: impl FnMut(
        tokio::process::Child,
    ) -> std::pin::Pin<Box<dyn Future<Output = Option<i32>> + Send>>,
) -> Result<SupervisedExit, Error> {
    let max_attempts = match policy {
        RestartPolicy::Never => 1,
        RestartPolicy::OnFailure { max_attempts } => 1 + max_attempts,
    };
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let child = host.spawn(config)?;
        let exit_code = run(child).await;
        if exit_code == Some(0) || attempt >= max_attempts {
            return Ok(SupervisedExit {
                exit_code,
                attempts: attempt,
            });
        }
        tracing::warn!(
            host = host.name(),
            attempt,
            exit_code,
            "adapter exited non-zero; restarting"
        );
        tokio::time::sleep(RestartPolicy::backoff(attempt)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(RestartPolicy::backoff(1), Duration::from_millis(500));
        assert_eq!(RestartPolicy::backoff(2), Duration::from_millis(1000));
        assert_eq!(RestartPolicy::backoff(10), Duration::from_millis(5000));
    }

    #[test]
    fn ssh_host_is_typed_unsupported() {
        let host = SshProcessHost;
        let config = AdapterProcessConfig {
            program: PathBuf::from("/bin/true"),
            args: vec![],
            env: vec![],
            cwd: PathBuf::from("/tmp"),
        };
        assert!(matches!(host.spawn(&config), Err(Error::Unsupported(_))));
    }

    #[tokio::test]
    async fn local_host_spawns_and_captures_exit_codes() {
        let host = LocalProcessHost;
        let config = AdapterProcessConfig {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), "exit 7".into()],
            env: vec![],
            cwd: std::env::temp_dir(),
        };
        let result = run_supervised(&host, &config, RestartPolicy::Never, |child| {
            Box::pin(async move {
                let mut child = child;
                child.wait().await.ok().and_then(|s| s.code())
            })
        })
        .await
        .unwrap();
        assert_eq!(result.exit_code, Some(7));
        assert_eq!(result.attempts, 1);
    }

    #[tokio::test]
    async fn supervised_restarts_on_failure_then_gives_up() {
        let host = LocalProcessHost;
        let config = AdapterProcessConfig {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), "exit 3".into()],
            env: vec![],
            cwd: std::env::temp_dir(),
        };
        let result = run_supervised(
            &host,
            &config,
            RestartPolicy::OnFailure { max_attempts: 2 },
            |child| {
                Box::pin(async move {
                    let mut child = child;
                    child.wait().await.ok().and_then(|s| s.code())
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, Some(3));
        assert_eq!(result.attempts, 3, "1 initial + 2 restarts");
    }
}
