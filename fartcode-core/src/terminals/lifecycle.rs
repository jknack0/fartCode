//! Lifecycle scripts (E1-06): setup/run/teardown execution in a PTY with the
//! env contract, status events, output tail, and session dedupe.
//!
//! Ports: `workspace-lifecycle-service.ts` (input transform, tail cap),
//! `runLifecycleScript.ts` (run options), `lifecycle-script-coordinator.ts`
//! (session dedupe), plus the fartCode-specific env contract from the ticket.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::events::{BroadcastEventBus, EventBus, InternalEvent};
use crate::tasks::naming::sanitize_name;
use crate::Error;

use super::pty::{EnvPolicy, PtyManager, PtySize};

/// Reference `LIFECYCLE_SCRIPT_TERMINAL_ID_PREFIX`.
pub const LIFECYCLE_SCRIPT_TERMINAL_ID_PREFIX: &str = "script-lifecycle-";
/// Reference `OUTPUT_TAIL_CAP` (16 KiB) — logs beyond this are dropped
/// (oldest first) so the drawer stays bounded.
pub const OUTPUT_TAIL_CAP: usize = 16 * 1024;
/// `FARTCODE_PORT = 50000 + (hash32(portSeed) % 1000) * 10` (ticket env contract).
pub const FARTCODE_PORT_BASE: u16 = 50_000;
/// Fallback wait when `timeout_ms` is None (the reference waits indefinitely
/// on the persistent agent terminal; a dedicated Phase-0 shell gets this cap
/// so a wedged script can't leak a session forever).
pub const DEFAULT_WAIT_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleScriptType {
    Setup,
    Run,
    Teardown,
}

impl LifecycleScriptType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Run => "run",
            Self::Teardown => "teardown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "setup" => Some(Self::Setup),
            "run" => Some(Self::Run),
            "teardown" => Some(Self::Teardown),
            _ => None,
        }
    }
}

/// A resolved script ready to run (reference `LifecycleScript`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleScript {
    pub r#type: LifecycleScriptType,
    pub script: String,
    pub shell_setup: Option<String>,
}

/// `script-lifecycle-<type>` (reference `createLifecycleScriptTerminalId`).
pub fn create_lifecycle_script_terminal_id(r#type: LifecycleScriptType) -> String {
    format!("{LIFECYCLE_SCRIPT_TERMINAL_ID_PREFIX}{}", r#type.as_str())
}

/// PTY session id for a lifecycle script:
/// `makePtySessionId(projectId, workspaceId, createLifecycleScriptTerminalId(type))`.
pub fn make_lifecycle_script_session_id(
    project_id: &str,
    workspace_id: &str,
    r#type: LifecycleScriptType,
) -> String {
    crate::conversations::make_pty_session_id(
        project_id,
        workspace_id,
        &create_lifecycle_script_terminal_id(r#type),
    )
}

/// Reference `terminalInputForScript`: normalize `\r?\n` → `\r`; with `exit`,
/// append `; exit\r` (posix) or `\rexit\r` (cmd.exe).
pub fn terminal_input_for_script(script: &str, exit: bool, windows_cmd_exit: bool) -> String {
    let normalized = script.replace("\r\n", "\r").replace('\n', "\r");
    if !exit {
        return format!("{normalized}\r");
    }
    let before_exit = normalized.trim_end_matches('\r');
    if windows_cmd_exit {
        format!("{before_exit}\rexit\r")
    } else {
        format!("{before_exit}; exit\r")
    }
}

/// Reference `stripTerminalControls`: strips CSI/OSC escapes + `\r` so the
/// drawer tail is readable plain text.
pub fn strip_terminal_controls(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\x1b' && i + 1 < chars.len() {
            match chars[i + 1] {
                '[' => {
                    // CSI: \x1b[...[A-Za-z]
                    i += 2;
                    while i < chars.len() && !chars[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    i += 1; // skip terminator
                    continue;
                }
                ']' => {
                    // OSC: \x1b]...ST (BEL or ESC \)
                    let mut j = i + 2;
                    while j < chars.len() {
                        if chars[j] == '\x07' {
                            j += 1;
                            break;
                        }
                        if chars[j] == '\x1b' && j + 1 < chars.len() && chars[j + 1] == '\\' {
                            j += 2;
                            break;
                        }
                        j += 1;
                    }
                    i = j;
                    continue;
                }
                _ => {
                    i += 2;
                    continue;
                }
            }
        }
        if chars[i] != '\r' {
            out.push(chars[i]);
        }
        i += 1;
    }
    out
}

/// Reference `appendOutputTail`: cap the accumulated tail at `OUTPUT_TAIL_CAP`.
/// The cut snaps to a char boundary so multi-byte UTF-8 output can never
/// panic `split_off`.
pub fn append_output_tail(current: &str, chunk: &str) -> String {
    let mut next = current.to_string();
    next.push_str(&strip_terminal_controls(chunk));
    if next.len() > OUTPUT_TAIL_CAP {
        let cut = next.len() - OUTPUT_TAIL_CAP;
        let cut = (0..=cut)
            .rev()
            .find(|&i| next.is_char_boundary(i))
            .unwrap_or(cut);
        next.split_off(cut)
    } else {
        next
    }
}

/// FNV-1a 32-bit hash (deterministic, no extra deps).
pub fn hash32(input: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in input.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// `FARTCODE_PORT = 50000 + (hash32(portSeed) % 1000) * 10` — deterministic,
/// isolated per workspace (two tasks in the same project get different
/// ports because the seed is the worktree path, not the project).
pub fn fartcode_port(port_seed: &str) -> u16 {
    FARTCODE_PORT_BASE + ((hash32(port_seed) % 1000) * 10) as u16
}

/// The lifecycle env contract (ticket-specified, exact names):
/// `FARTCODE_TASK_ID`, `FARTCODE_TASK_NAME` (slugified, fallback "task"),
/// `FARTCODE_TASK_PATH`, `FARTCODE_ROOT_PATH`, `FARTCODE_DEFAULT_BRANCH` (default "main"),
/// `FARTCODE_PORT` from `fartcode_port(port_seed)`.
pub fn task_env(
    task_id: &str,
    task_name: &str,
    task_path: &Path,
    root_path: &Path,
    default_branch: &str,
    port_seed: &str,
) -> Vec<(String, String)> {
    let name = {
        let slug = sanitize_name(task_name);
        if slug.is_empty() {
            "task".to_string()
        } else {
            slug
        }
    };
    vec![
        ("FARTCODE_TASK_ID".into(), task_id.to_string()),
        ("FARTCODE_TASK_NAME".into(), name),
        (
            "FARTCODE_TASK_PATH".into(),
            task_path.to_string_lossy().into_owned(),
        ),
        (
            "FARTCODE_ROOT_PATH".into(),
            root_path.to_string_lossy().into_owned(),
        ),
        (
            "FARTCODE_DEFAULT_BRANCH".into(),
            if default_branch.is_empty() {
                "main".to_string()
            } else {
                default_branch.to_string()
            },
        ),
        ("FARTCODE_PORT".into(), fartcode_port(port_seed).to_string()),
    ]
}

/// Reference `LifecycleScriptExecutionResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleRunResult {
    /// `wait_for_exit: false` — the run continues on a background thread.
    Started,
    AlreadyRunning,
    Exited {
        exit_code: Option<u32>,
        signal: Option<String>,
        output_tail: String,
    },
    /// Hit `timeoutMs` with `surface_failure` disabled (a timed-out run is
    /// otherwise indistinguishable from a clean exit).
    TimedOut {
        output_tail: String,
    },
}

/// Run options (reference `runLifecycleScript` / coordinator defaults).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunOptions {
    pub wait_for_exit: bool,
    pub exit: bool,
    /// Reference: keep the persistent agent terminal alive after the script
    /// exits for manual inspection. Phase 0 runs each script in a dedicated
    /// shell (no interactive surface yet — E2-06), so this is a no-op.
    pub respawn_after_exit: bool,
    pub log_failure: bool,
    pub surface_failure: bool,
    pub continue_on_failure: bool,
    pub timeout_ms: Option<u64>,
}

/// Manual-run options (reference `runLifecycleScript`).
pub fn manual_run_options() -> RunOptions {
    RunOptions {
        wait_for_exit: true,
        exit: false,
        respawn_after_exit: true,
        log_failure: true,
        surface_failure: true,
        continue_on_failure: false,
        timeout_ms: None,
    }
}

/// Runs lifecycle scripts for a task. Phase 0 executes each script in a
/// dedicated shell PTY (the reference types into the agent's persistent
/// terminal — that wiring arrives with E2-06; the primitive and the
/// semantics are identical).
pub struct LifecycleScriptService {
    pty: Arc<dyn PtyManager>,
    events: Arc<BroadcastEventBus>,
    shell: String,
    active: Mutex<HashSet<String>>,
}

impl LifecycleScriptService {
    pub fn new(pty: Arc<dyn PtyManager>, events: Arc<BroadcastEventBus>) -> Self {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| {
            if cfg!(windows) {
                "cmd.exe".into()
            } else {
                "/bin/bash".into()
            }
        });
        Self {
            pty,
            events,
            shell,
            active: Mutex::new(HashSet::new()),
        }
    }

    /// Runs one script; emits `running|succeeded|failed|stopped` status
    /// events and dedupes by session id (`already-running`).
    ///
    /// `wait_for_exit: true` blocks until the script exits (setup/teardown
    /// style). `false` returns `Started` immediately and the run continues
    /// on a background thread that owns the session slot and reports the
    /// terminal status itself (reference fire-and-forget typing).
    pub fn run(
        self: &Arc<Self>,
        session_id: &str,
        script: &LifecycleScript,
        cwd: &Path,
        env: &[(String, String)],
        options: &RunOptions,
    ) -> Result<LifecycleRunResult, Error> {
        // Dedupe active sessions (reference ptySessionRegistry).
        {
            let mut active = self
                .active
                .lock()
                .map_err(|_| Error::Internal("lifecycle active-set poisoned".into()))?;
            if active.contains(session_id) {
                return Ok(LifecycleRunResult::AlreadyRunning);
            }
            active.insert(session_id.to_string());
        }

        if !options.wait_for_exit {
            // Fire-and-forget: the background thread holds the slot and
            // releases it when the run finishes.
            let me = self.clone();
            let sid = session_id.to_string();
            let script = script.clone();
            let cwd = cwd.to_path_buf();
            let env = env.to_vec();
            let options = options.clone();
            std::thread::spawn(move || {
                if let Err(e) = me.run_blocking(&sid, &script, &cwd, &env, &options) {
                    tracing::warn!(error = %e, session = %sid, "lifecycle background run failed");
                }
                me.release(&sid);
            });
            return Ok(LifecycleRunResult::Started);
        }

        let result = self.run_blocking(session_id, script, cwd, env, options);
        self.release(session_id);
        result
    }

    fn release(&self, session_id: &str) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(session_id);
        }
    }

    /// The synchronous execution core (no dedupe, no slot management — the
    /// caller owns the session slot).
    fn run_blocking(
        &self,
        session_id: &str,
        script: &LifecycleScript,
        cwd: &Path,
        env: &[(String, String)],
        options: &RunOptions,
    ) -> Result<LifecycleRunResult, Error> {
        let status = |s: &str| {
            self.events
                .send(InternalEvent::LifecycleScriptStatusChanged {
                    session_id: session_id.to_string(),
                    script_type: script.r#type.as_str().to_string(),
                    status: s.to_string(),
                });
        };

        let mut output_tail = String::new();
        let mut exit_code = None;
        let mut signal = None;
        let mut timed_out = false;

        let run_result = (|| -> Result<(), Error> {
            status("running");

            // shellSetup is prepended to the shell-line commands (reference
            // pty-spawn-platform "prepends shellSetup to shell-line commands").
            let combined = match &script.shell_setup {
                Some(setup) if !setup.is_empty() => format!("{setup}\n{}", script.script),
                _ => script.script.clone(),
            };
            let input = terminal_input_for_script(&combined, options.exit, cfg!(windows));
            let input = input.as_bytes();

            let mut handle = self.pty.spawn(
                &self.shell,
                &[],
                cwd,
                env,
                PtySize::default(),
                EnvPolicy::Inherit,
            )?;
            handle.write(&String::from_utf8_lossy(input))?;

            let timeout = options
                .timeout_ms
                .map(Duration::from_millis)
                .unwrap_or(Duration::from_secs(DEFAULT_WAIT_TIMEOUT_SECS));
            match handle.wait_exit(timeout) {
                Ok(exit) => {
                    exit_code = exit.exit_code;
                    signal = exit.signal;
                }
                Err(Error::LifecycleScriptTimeout(_)) => {
                    timed_out = true;
                    let _ = handle.kill();
                }
                Err(e) => return Err(e),
            }

            // Join the reader so the final chunk the child wrote before
            // exiting is in the buffer (child reap vs pty EOF race).
            let _ = handle.flush();
            let mut buf = Vec::new();
            while handle.try_read(&mut buf).unwrap_or(false) {
                if buf.is_empty() {
                    break;
                }
                output_tail = append_output_tail(&output_tail, &String::from_utf8_lossy(&buf));
                buf.clear();
            }
            Ok(())
        })();

        if let Err(e) = run_result {
            status("stopped");
            return Err(e);
        }

        if timed_out {
            status("stopped");
            if options.log_failure {
                tracing::warn!(session = %session_id, "lifecycle script timed out");
            }
            if options.surface_failure && !options.continue_on_failure {
                return Err(Error::LifecycleScriptTimeout(session_id.to_string()));
            }
            return Ok(LifecycleRunResult::TimedOut { output_tail });
        }

        let success = signal.is_none() && matches!(exit_code, None | Some(0));
        status(if success { "succeeded" } else { "failed" });
        if !success {
            if options.log_failure {
                tracing::warn!(
                    session = %session_id,
                    exit = ?exit_code,
                    signal = ?signal,
                    "lifecycle script failed"
                );
            }
            if options.surface_failure && !options.continue_on_failure {
                return Err(Error::LifecycleScriptFailed {
                    session_id: session_id.to_string(),
                    exit_code,
                    signal,
                    output_tail: output_tail.clone(),
                });
            }
        }
        Ok(LifecycleRunResult::Exited {
            exit_code,
            signal,
            output_tail,
        })
    }
}
