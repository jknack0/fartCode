//! E2-06 — Agent launcher integration: spawns a provider CLI in a PTY with
//! the allowlisted env + task env, delivers the prompt, emits run events,
//! and applies the respawn policy.
//!
//! Uses a fake `goose` binary (goose is not installed on CI hosts) shadowed
//! via a temp PATH dir, so the whole flow runs through the real detection +
//! launcher code. The fake agent writes what it observed to a report file
//! (passed via the task env), which the tests assert on.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use fartcode_core::db::SqliteDb;
use fartcode_core::dependencies::HostDependencyStore;
use fartcode_core::events::{BroadcastEventBus, EventBus, InternalEvent};
use fartcode_core::pty::launcher::{AgentLaunchContext, AgentLauncher};
use fartcode_core::Error;
use fartcode_terminal::PortablePtyManager;

/// These tests mutate the process-global PATH — serialize them so parallel
/// runs can't observe each other's temp dirs (and their fake binaries).
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Restores the process env on drop (PATH mutations + secrets).
struct EnvGuard(Vec<(&'static str, Option<String>)>);
impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self(vec![(key, prev)])
    }
}
impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prev) in &self.0 {
            match prev {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}

/// Fake provider CLI: records the env + argv prompt to `$GOOSE_REPORT`,
/// reads stdin, exits with `$FAKE_EXIT` (default 0).
fn write_fake_goose(dir: &Path, exit_code: &str) -> PathBuf {
    let script = dir.join("goose");
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/bash
echo "GOOSE_STARTED args=$*" > "$GOOSE_REPORT.started"
echo "fartcode_task=$FARTCODE_TASK_ID fartcode_port=$FARTCODE_PORT args=$* secret=${{SECRET_TOKEN:-absent}} term=$TERM" > "$GOOSE_REPORT"
exit {exit_code}
"#
        ),
    )
    .unwrap();
    let mut perms = std::fs::metadata(&script).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    perms.set_mode(0o755);
    std::fs::set_permissions(&script, perms).unwrap();
    script
}

struct Fixture {
    _db: Arc<SqliteDb>,
    launcher: Arc<AgentLauncher>,
    _bus: Arc<BroadcastEventBus>,
    // Subscribed BEFORE run() — broadcast receivers miss messages sent
    // before they subscribe.
    rx: tokio::sync::broadcast::Receiver<InternalEvent>,
    report: PathBuf,
    // Keep the fake-goose dir + worktree alive for the fixture's lifetime
    // (TempDirs delete themselves on drop — the spawn cwd must exist).
    _bin_dir: tempfile::TempDir,
    _worktree: tempfile::TempDir,
    _guard: EnvGuard,
    _secret_guard: EnvGuard,
    ctx: AgentLaunchContext,
}

impl Fixture {
    fn new(exit_code: &str) -> Self {
        let bin_dir = tempfile::tempdir().unwrap();
        write_fake_goose(bin_dir.path(), exit_code);
        let path = std::env::var("PATH").unwrap_or_default();
        let guard = EnvGuard::set(
            "PATH",
            &format!("{}:{}", bin_dir.path().to_string_lossy(), path),
        );
        // A hostile secret that must never reach the agent.
        let secret_guard = EnvGuard::set("SECRET_TOKEN", "top-secret");

        let db = SqliteDb::init_in_memory().unwrap();
        let bus = Arc::new(BroadcastEventBus::new(64));
        let rx = bus.subscribe();
        let launcher = Arc::new(AgentLauncher::new(
            Arc::new(PortablePtyManager),
            Arc::new(HostDependencyStore::new(
                db.clone(),
                Arc::new(fartcode_core::dependencies::ProcessInstallRunner),
            )),
            bus.clone(),
        ));

        let worktree = tempfile::tempdir().unwrap();
        let report = worktree.path().join("goose-report.txt");
        let mut ctx = AgentLaunchContext::with_task_env(
            "goose",
            "conv-1",
            "task-1",
            "Fake Task",
            worktree.path().to_path_buf(),
            worktree.path(),
        );
        // The fake agent's report path rides in the task env.
        ctx.task_env
            .push(("GOOSE_REPORT".into(), report.to_string_lossy().into_owned()));

        Self {
            _db: db,
            launcher,
            _bus: bus,
            rx,
            report,
            _bin_dir: bin_dir,
            _worktree: worktree,
            _guard: guard,
            _secret_guard: secret_guard,
            ctx,
        }
    }

    fn events_for(&mut self, conversation_id: &str) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                InternalEvent::AgentRunStarted {
                    conversation_id: c, ..
                } if c == conversation_id => out.push("started".into()),
                InternalEvent::AgentRunFinished {
                    conversation_id: c, ..
                } if c == conversation_id => out.push("finished".into()),
                InternalEvent::AgentSessionExited { conversation_id: c }
                    if c == conversation_id =>
                {
                    out.push("exited".into())
                }
                _ => {}
            }
        }
        out
    }

    fn report(&self) -> String {
        std::fs::read_to_string(&self.report).unwrap_or_default()
    }
}

/// The fake goose runs in the worktree with the env contract + allowlist
/// (no secrets), the argv prompt lands in the args, TERM is forced, and the
/// run/finish/exited events fire.
#[test]
fn launches_agent_in_worktree_with_allowlisted_env_and_prompt() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut fx = Fixture::new("0");
    let ctx = AgentLaunchContext {
        initial_prompt: Some("please fix the bug".into()),
        ..fx.ctx.clone()
    };

    let outcome = fx.launcher.run(&ctx).unwrap();
    assert_eq!(outcome.spawns, 1, "no respawn by default");
    assert_eq!(outcome.last_exit_code, Some(0));

    eprintln!(
        "REPORT_PATH={} EXISTS={} STARTED_AT_REPORT={} DOT_STARTED_IN_WORKTREE={}",
        fx.report.display(),
        fx.report.exists(),
        fx.report.with_extension("txt.started").exists(),
        fx._worktree.path().join(".started").exists()
    );
    let report = fx.report();
    assert!(
        report.contains("fartcode_task=task-1"),
        "task env: {report}"
    );
    let port = report
        .split("fartcode_port=")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|p| p.parse::<u32>().ok());
    assert!(
        port.is_some_and(|p| (50000..60000).contains(&p)),
        "FARTCODE_PORT injected (50000-59999): {report}"
    );
    assert!(
        report.contains("please fix the bug"),
        "argv prompt delivered: {report}"
    );
    assert!(
        report.contains("secret=absent"),
        "SECRET_TOKEN must never reach the agent: {report}"
    );
    assert!(
        report.contains("term=xterm-256color"),
        "TERM forced: {report}"
    );

    let events = fx.events_for("conv-1");
    assert_eq!(events, vec!["started", "finished", "exited"], "{events:?}");
}

/// Respawning: exit 1 + respawn_resume → up to MAX_RESPAWNS+1 spawns;
/// tmux-enabled suppresses respawn entirely.
#[test]
fn respawn_policy_is_applied() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut fx = Fixture::new("1");
    let ctx = AgentLaunchContext {
        initial_prompt: Some("retry me".into()),
        respawn_resume: true,
        ..fx.ctx.clone()
    };
    let outcome = fx.launcher.run(&ctx).unwrap();
    assert_eq!(outcome.spawns, 3, "initial + 2 respawns: {outcome:?}");
    let events = fx.events_for("conv-1");
    assert_eq!(
        events.iter().filter(|e| e.as_str() == "started").count(),
        3,
        "{events:?}"
    );
    assert!(events.last().is_some_and(|e| e == "exited"));

    // tmux enabled -> no respawn even with respawn_resume.
    let fx2 = Fixture::new("1");
    let ctx2 = AgentLaunchContext {
        initial_prompt: Some("retry me".into()),
        respawn_resume: true,
        tmux_enabled: true,
        ..fx2.ctx.clone()
    };
    let outcome2 = fx2.launcher.run(&ctx2).unwrap();
    assert_eq!(outcome2.spawns, 1, "no respawn under tmux");
}

/// Keystroke-strategy regression (blocking-fix): an agent that dies before
/// producing output must not leave `wait_exit` polling a reaped child for
/// AGENT_WAIT_TIMEOUT — the pump observes the exit and run() returns it.
#[test]
fn keystroke_agent_exiting_early_is_observed_not_hung() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let bin_dir = tempfile::tempdir().unwrap();
    let script = bin_dir.path().join("kimi");
    std::fs::write(
        &script,
        "#!/bin/bash\necho started > \"$GOOSE_REPORT\"\nexit 7\n",
    )
    .unwrap();
    let mut perms = std::fs::metadata(&script).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    perms.set_mode(0o755);
    std::fs::set_permissions(&script, perms).unwrap();
    let path = std::env::var("PATH").unwrap_or_default();
    let _guard = EnvGuard::set(
        "PATH",
        &format!("{}:{}", bin_dir.path().to_string_lossy(), path),
    );

    let db = SqliteDb::init_in_memory().unwrap();
    let bus = Arc::new(BroadcastEventBus::new(16));
    let launcher = Arc::new(AgentLauncher::new(
        Arc::new(PortablePtyManager),
        Arc::new(HostDependencyStore::new(
            db.clone(),
            Arc::new(fartcode_core::dependencies::ProcessInstallRunner),
        )),
        bus.clone(),
    ));
    let worktree = tempfile::tempdir().unwrap();
    let report = worktree.path().join("kimi-report.txt");
    let mut ctx = AgentLaunchContext::with_task_env(
        "kimi",
        "conv-3",
        "task-3",
        "Keystroke Agent",
        worktree.path().to_path_buf(),
        worktree.path(),
    );
    ctx.task_env
        .push(("GOOSE_REPORT".into(), report.to_string_lossy().into_owned()));
    ctx.initial_prompt = Some("do the thing".into());
    let _ = worktree;

    let outcome = launcher.run(&ctx).unwrap();
    assert_eq!(
        outcome.last_exit_code,
        Some(7),
        "observed exit, not a 24h wait: {outcome:?}"
    );
    assert_eq!(outcome.spawns, 1, "{outcome:?}");
    assert!(
        report.exists(),
        "the fake agent ran (its report file exists)"
    );
}

/// A non-installed provider surfaces a typed error (AgentNotFound), not a
/// panic.
#[test]
fn missing_binary_errors_typed() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let bin_dir = tempfile::tempdir().unwrap();
    let guard = EnvGuard::set(
        "PATH",
        &format!("{}:/nonexistent", bin_dir.path().to_string_lossy()),
    );
    let db = SqliteDb::init_in_memory().unwrap();
    let bus = Arc::new(BroadcastEventBus::new(8));
    let launcher = Arc::new(AgentLauncher::new(
        Arc::new(PortablePtyManager),
        Arc::new(HostDependencyStore::new(
            db.clone(),
            Arc::new(fartcode_core::dependencies::ProcessInstallRunner),
        )),
        bus.clone(),
    ));
    let worktree = tempfile::tempdir().unwrap();
    let _ctx = AgentLaunchContext::with_task_env(
        "goose",
        "conv-2",
        "task-2",
        "No Binary",
        worktree.path().to_path_buf(),
        worktree.path(),
    );
    let _ = worktree;
    let err = launcher.resolve_binary("goose").unwrap_err();
    assert!(matches!(err, Error::AgentNotFound(_)), "{err:?}");
    drop(guard);
}

// ── E12-05: remote launches ────────────────────────────────────

/// (command, cwd, env) recorded per spawn.
type SpawnRecord = (String, PathBuf, Vec<(String, String)>);

/// Records what the launcher asked for instead of spawning anything: the
/// remote transport is `fartcode-ssh`'s job, the ROUTING decision is this
/// crate's, and only the latter is testable without a host.
#[derive(Default)]
struct RecordingPty {
    spawns: Mutex<Vec<SpawnRecord>>,
}

struct InstantExit;
impl fartcode_core::terminals::pty::PtyHandle for InstantExit {
    fn write(&mut self, _data: &str) -> Result<(), Error> {
        Ok(())
    }
    fn try_read(&mut self, _buf: &mut Vec<u8>) -> Result<bool, Error> {
        Ok(false)
    }
    fn wait_exit(
        &mut self,
        _timeout: std::time::Duration,
    ) -> Result<fartcode_core::terminals::pty::PtyExit, Error> {
        Ok(fartcode_core::terminals::pty::PtyExit {
            exit_code: Some(0),
            signal: None,
        })
    }
    fn resize(&mut self, _cols: u16, _rows: u16) -> Result<(), Error> {
        Ok(())
    }
    fn try_wait_exit(&mut self) -> Result<Option<fartcode_core::terminals::pty::PtyExit>, Error> {
        Ok(Some(fartcode_core::terminals::pty::PtyExit {
            exit_code: Some(0),
            signal: None,
        }))
    }
    fn flush(&mut self) -> Result<(), Error> {
        Ok(())
    }
    fn kill(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

impl fartcode_core::terminals::pty::PtyManager for RecordingPty {
    fn spawn(
        &self,
        cmd: &str,
        _args: &[String],
        cwd: &Path,
        env: &[(String, String)],
        _size: fartcode_core::terminals::pty::PtySize,
        _policy: fartcode_core::terminals::pty::EnvPolicy,
        _remove: &[String],
    ) -> Result<Box<dyn fartcode_core::terminals::pty::PtyHandle>, Error> {
        self.spawns
            .lock()
            .unwrap()
            .push((cmd.to_string(), cwd.to_path_buf(), env.to_vec()));
        Ok(Box::new(InstantExit))
    }
}

struct FakeRoute {
    pty: Arc<RecordingPty>,
    workspace_id: String,
}

impl fartcode_core::terminals::pty::RemotePtyLookup for FakeRoute {
    fn resolve(
        &self,
        task_id: &str,
    ) -> Result<Option<fartcode_core::terminals::pty::RemotePtyRoute>, Error> {
        // Only the remote task routes remotely — everything else stays local.
        Ok((task_id == "task-1").then(|| {
            (
                self.pty.clone() as Arc<dyn fartcode_core::terminals::pty::PtyManager>,
                self.workspace_id.clone(),
            )
        }))
    }
}

#[test]
fn remote_tasks_launch_on_the_remote_pty_with_a_workspace_id_and_no_local_agent_socket() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let bin_dir = tempfile::tempdir().unwrap();
    write_fake_goose(bin_dir.path(), "0");
    let path = std::env::var("PATH").unwrap_or_default();
    let _guard = EnvGuard::set(
        "PATH",
        &format!("{}:{}", bin_dir.path().to_string_lossy(), path),
    );
    // A local agent socket that must NOT travel to the remote host.
    let _sock = EnvGuard::set("SSH_AUTH_SOCK", "/tmp/local-agent.sock");

    let db = SqliteDb::init_in_memory().unwrap();
    let bus = Arc::new(BroadcastEventBus::new(64));
    let recording = Arc::new(RecordingPty::default());
    let launcher = AgentLauncher::new(
        Arc::new(PortablePtyManager),
        Arc::new(HostDependencyStore::new(
            db.clone(),
            Arc::new(fartcode_core::dependencies::ProcessInstallRunner),
        )),
        bus.clone(),
    )
    .with_remote_pty(Arc::new(FakeRoute {
        pty: recording.clone(),
        workspace_id: "ws-remote-1".into(),
    }));

    let worktree = PathBuf::from("/srv/repos/app/.fartCode/worktrees/seg/feature-x");
    let ctx = AgentLaunchContext::with_task_env(
        "goose",
        "conv-remote",
        "task-1",
        "Remote Task",
        worktree.clone(),
        Path::new("/srv/repos/app"),
    );
    launcher.run(&ctx).unwrap();

    let spawns = recording.spawns.lock().unwrap();
    assert_eq!(spawns.len(), 1, "the remote pty must be the one used");
    let (_, cwd, env) = &spawns[0];
    // AC6: the cwd is the task's REMOTE worktree — a path that does not
    // exist on this machine, which is why a local spawn had to be ruled out.
    assert_eq!(cwd, &worktree);
    let get = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
    // AC7: the workspace id travels, the local agent socket does not.
    assert_eq!(get("REMOTE_WORKSPACE_ID").as_deref(), Some("ws-remote-1"));
    assert_eq!(get("SSH_AUTH_SOCK"), None);
    assert_eq!(get("FARTCODE_TASK_ID").as_deref(), Some("task-1"));
}
