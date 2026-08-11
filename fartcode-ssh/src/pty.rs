//! Remote PTY (E12-05): `PtyManager` over an SSH channel.
//!
//! `fartcode_core::terminals::pty::PtyManager` is a **blocking** trait — the
//! terminal manager drives it from ordinary threads — while russh is async.
//! The bridge is one tokio task per session that owns the channel: the sync
//! side talks to it over an unbounded mpsc (sends never block) and reads
//! output from a shared buffer. No `block_on` on the caller's thread, so a
//! wedged host cannot stall the UI thread that opened the terminal.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fartcode_core::shell_escape::single_quote;
use fartcode_core::terminals::pty::{EnvPolicy, PtyExit, PtyHandle, PtyManager, PtySize};
use fartcode_core::Error;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use crate::SshClient;

/// Poll interval for the blocking `wait_exit` — short enough to feel
/// immediate, long enough not to spin a core.
const POLL: Duration = Duration::from_millis(25);

/// Builds the remote command line for a PTY launch.
///
/// Shape: `cd '<cwd>' && exec env [-i] [-u VAR]... K=V... 'cmd' 'arg'...`
///
/// - `EnvPolicy::AllowlistedOnly` adds `-i`, so the child sees only what is
///   passed here (the E3-08 allowlist is the sole source, same guarantee
///   `env_clear` gives locally). `TERM` is re-added because a PTY without one
///   makes every full-screen agent render garbage.
/// - `remove` becomes `env -u`, applied after the pairs (ADR-0034: a stray
///   `ANTHROPIC_API_KEY` flips the claude CLI to API billing).
/// - Every element is single-quoted; nothing is interpolated.
pub fn remote_launch_line(
    cmd: &str,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    policy: EnvPolicy,
    remove: &[String],
) -> String {
    let mut parts: Vec<String> = vec!["exec".into(), "env".into()];
    if policy == EnvPolicy::AllowlistedOnly {
        parts.push("-i".into());
        if !env.iter().any(|(k, _)| k == "TERM") {
            parts.push(single_quote("TERM=xterm-256color"));
        }
    }
    for var in remove {
        parts.push("-u".into());
        parts.push(single_quote(var));
    }
    for (key, value) in env {
        parts.push(single_quote(&format!("{key}={value}")));
    }
    parts.push(single_quote(cmd));
    parts.extend(args.iter().map(|a| single_quote(a)));

    format!(
        "cd {} && {}",
        single_quote(&cwd.to_string_lossy()),
        parts.join(" ")
    )
}

// ── Session task ─────────────────────────────────────────────

/// What the blocking handle asks the session task to do.
enum Op {
    Write(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Kill,
}

#[derive(Default)]
struct Shared {
    /// Output the reader has drained but the caller has not taken yet.
    buffer: Vec<u8>,
    exit: Option<PtyExit>,
    /// The channel is gone (EOF/close/kill): no further output will arrive.
    closed: bool,
}

/// A live remote PTY.
pub struct RemotePtyHandle {
    ops: UnboundedSender<Op>,
    shared: Arc<Mutex<Shared>>,
}

impl RemotePtyHandle {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Shared>, Error> {
        self.shared
            .lock()
            .map_err(|_| Error::Internal("remote pty state mutex poisoned".into()))
    }

    fn send(&self, op: Op) -> Result<(), Error> {
        self.ops
            .send(op)
            .map_err(|_| Error::SshChannel("remote pty session has ended".into()))
    }
}

impl PtyHandle for RemotePtyHandle {
    fn write(&mut self, data: &str) -> Result<(), Error> {
        self.send(Op::Write(data.as_bytes().to_vec()))
    }

    fn try_read(&mut self, buf: &mut Vec<u8>) -> Result<bool, Error> {
        let mut shared = self.lock()?;
        if shared.buffer.is_empty() {
            return Ok(false);
        }
        buf.append(&mut shared.buffer);
        Ok(true)
    }

    fn wait_exit(&mut self, timeout: Duration) -> Result<PtyExit, Error> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(exit) = self.lock()?.exit.clone() {
                return Ok(exit);
            }
            if Instant::now() >= deadline {
                // Same variant the local pty reports, so callers keep one
                // timeout branch for both transports.
                return Err(Error::LifecycleScriptTimeout(format!(
                    "remote pty wait ({timeout:?})"
                )));
            }
            std::thread::sleep(POLL);
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Error> {
        // Same clamping as the local pty: agents misbehave below these.
        self.send(Op::Resize {
            cols: cols.max(2),
            rows: rows.max(1),
        })
    }

    fn try_wait_exit(&mut self) -> Result<Option<PtyExit>, Error> {
        Ok(self.lock()?.exit.clone())
    }

    fn flush(&mut self) -> Result<(), Error> {
        // The reader task marks `closed` only after the channel is drained,
        // so waiting for it is exactly "everything the child wrote is here".
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.lock()?.closed {
            if Instant::now() >= deadline {
                return Ok(());
            }
            std::thread::sleep(POLL);
        }
        Ok(())
    }

    fn kill(&mut self) -> Result<(), Error> {
        // Best effort: a session that already ended has no receiver, which is
        // the outcome kill wanted anyway.
        let _ = self.send(Op::Kill);
        Ok(())
    }
}

// ── Manager ─────────────────────────────────────────────────

/// Spawns PTYs on a remote host. One manager per connection.
pub struct SshPtyManager {
    client: Arc<SshClient>,
    runtime: tokio::runtime::Handle,
}

impl SshPtyManager {
    /// Binds a connected client to the current tokio runtime.
    pub fn new(client: Arc<SshClient>) -> Result<Self, Error> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            Error::Internal("SshPtyManager::new must be called from a tokio runtime".into())
        })?;
        Ok(Self { client, runtime })
    }

    pub fn with_runtime(client: Arc<SshClient>, runtime: tokio::runtime::Handle) -> Self {
        Self { client, runtime }
    }
}

impl PtyManager for SshPtyManager {
    fn spawn(
        &self,
        cmd: &str,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
        size: PtySize,
        env_policy: EnvPolicy,
        remove: &[String],
    ) -> Result<Box<dyn PtyHandle>, Error> {
        let command = remote_launch_line(cmd, args, cwd, env, env_policy, remove);
        let client = self.client.clone();
        let cols = u32::from(size.cols.max(2));
        let rows = u32::from(size.rows.max(1));

        // Opening the channel is async; block the *calling* thread only for
        // the handshake, never for the session's lifetime.
        let channel = tokio::task::block_in_place(|| {
            self.runtime
                .block_on(async move { client.pty_exec(&command, cols, rows).await })
        })?;

        let (ops, mut rx) = unbounded_channel::<Op>();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let task_shared = shared.clone();

        self.runtime.spawn(async move {
            let mut channel = channel;
            loop {
                tokio::select! {
                    op = rx.recv() => match op {
                        Some(Op::Write(data)) => {
                            if channel.data(&data[..]).await.is_err() {
                                break;
                            }
                        }
                        Some(Op::Resize { cols, rows }) => {
                            let _ = channel
                                .window_change(u32::from(cols), u32::from(rows), 0, 0)
                                .await;
                        }
                        // Kill, or every handle dropped: close the channel.
                        Some(Op::Kill) | None => break,
                    },
                    msg = channel.wait() => match msg {
                        Some(russh::ChannelMsg::Data { ref data }) => {
                            if let Ok(mut s) = task_shared.lock() {
                                s.buffer.extend_from_slice(data);
                            }
                        }
                        Some(russh::ChannelMsg::ExtendedData { ref data, .. }) => {
                            // A PTY merges stderr into the stream; anything
                            // arriving out-of-band is still terminal output.
                            if let Ok(mut s) = task_shared.lock() {
                                s.buffer.extend_from_slice(data);
                            }
                        }
                        Some(russh::ChannelMsg::ExitStatus { exit_status }) => {
                            if let Ok(mut s) = task_shared.lock() {
                                s.exit = Some(PtyExit {
                                    exit_code: Some(exit_status),
                                    signal: None,
                                });
                            }
                        }
                        Some(russh::ChannelMsg::ExitSignal { ref signal_name, .. }) => {
                            if let Ok(mut s) = task_shared.lock() {
                                s.exit = Some(PtyExit {
                                    exit_code: None,
                                    signal: Some(format!("{signal_name:?}")),
                                });
                            }
                        }
                        Some(_) => {}
                        // Channel closed: the session is over.
                        None => break,
                    },
                }
            }

            let _ = channel.close().await;
            if let Ok(mut s) = task_shared.lock() {
                // A channel that closed without a status still ended — report
                // it, or `wait_exit` would poll until its timeout.
                if s.exit.is_none() {
                    s.exit = Some(PtyExit {
                        exit_code: None,
                        signal: None,
                    });
                }
                s.closed = true;
            }
        });

        Ok(Box::new(RemotePtyHandle { ops, shared }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn allowlisted_only_clears_the_remote_env_and_keeps_a_term() {
        let line = remote_launch_line(
            "claude",
            &["--continue".to_string()],
            Path::new("/srv/repo"),
            &pairs(&[("REMOTE_WORKSPACE_ID", "w1")]),
            EnvPolicy::AllowlistedOnly,
            &["ANTHROPIC_API_KEY".to_string()],
        );
        assert_eq!(
            line,
            "cd '/srv/repo' && exec env -i 'TERM=xterm-256color' -u 'ANTHROPIC_API_KEY' \
             'REMOTE_WORKSPACE_ID=w1' 'claude' '--continue'"
        );
    }

    #[test]
    fn inherit_does_not_clear() {
        let line = remote_launch_line(
            "bash",
            &[],
            Path::new("/srv/repo"),
            &pairs(&[("FOO", "bar")]),
            EnvPolicy::Inherit,
            &[],
        );
        assert_eq!(line, "cd '/srv/repo' && exec env 'FOO=bar' 'bash'");
    }

    #[test]
    fn hostile_paths_and_values_stay_data() {
        let line = remote_launch_line(
            "agent",
            &["--flag=a b".to_string()],
            Path::new("/srv/re po/a';touch pwned;'"),
            &pairs(&[("X", "'; rm -rf / #")]),
            EnvPolicy::AllowlistedOnly,
            &[],
        );
        // The only unquoted shell syntax in the line is the `cd ... &&` and
        // the `env` flags this function itself emits.
        let quoted_cwd = single_quote("/srv/re po/a';touch pwned;'");
        assert!(line.starts_with(&format!("cd {quoted_cwd} && ")));
        assert!(!line.contains("&& rm -rf"));
        assert!(!line.contains("; touch pwned"));
        // Each hostile payload survives as one quoted element.
        assert!(line.contains(&single_quote("X='; rm -rf / #")));
        assert!(line.contains(&single_quote("--flag=a b")));
    }
}
