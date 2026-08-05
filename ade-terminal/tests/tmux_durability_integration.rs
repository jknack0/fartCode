//! Tmux durability for interactive terminals (ADR-0025) — integration test
//! against the REAL tmux binary (skips when absent).
//!
//! The contract under test: the shell outlives the PTY that spawned it.
//! 1. Spawn the create-or-attach shell line through the same PTY path the
//!    app uses (`sh -c <line>` under `PortablePtyManager`, `TERM` overlaid).
//! 2. Prove the shell is live (write a file, `cd` away from the start dir).
//! 3. Kill the PTY (simulated app crash — the attach client dies).
//! 4. The tmux session must still exist.
//! 5. A fresh PTY running the SAME line must reattach to the SAME shell
//!    (the retained cwd proves identity, not a fresh session).
//! 6. The prefix sweep kills exactly this task's sessions.
//!
//! Each test uses a prefix unique to its own pid — the tmux server is a
//! process-wide shared resource, and the sweep test would otherwise kill
//! the durability test's session mid-run.

use std::path::Path;
use std::time::{Duration, Instant};

use ade_core::pty::tmux::{
    build_terminal_session_command, build_tmux_shell_line, kill_tmux_sessions_by_prefix,
    list_tmux_sessions_by_prefix, make_tmux_session_name, resolve_tmux_binary,
};
use ade_core::terminals::pty::{EnvPolicy, PtyHandle, PtyManager, PtySize};
use ade_terminal::PortablePtyManager;

/// Kills the test session no matter how the test ends (it lives in the
/// user's real tmux server).
struct SweepGuard {
    prefix: String,
}

impl Drop for SweepGuard {
    fn drop(&mut self) {
        let _ = kill_tmux_sessions_by_prefix(&self.prefix);
    }
}

fn tmux_command() -> String {
    resolve_tmux_binary()
        .map(|b| b.command.clone())
        .expect("tmux resolved")
}

fn session_exists(name: &str) -> bool {
    std::process::Command::new(tmux_command())
        .args(["has-session", "-t", name])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Writes to the handle, retrying while the spawned shell is still wiring
/// itself up (tmux create-or-attach takes a beat before input is consumed).
fn write_retry(handle: &mut Box<dyn PtyHandle>, data: &str) {
    let mut last_err = None;
    for _ in 0..50 {
        match handle.write(data) {
            Ok(()) => return,
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    panic!("write never succeeded: {last_err:?}");
}

fn spawn_attach(mgr: &PortablePtyManager, line: &str, cwd: &Path) -> Box<dyn PtyHandle> {
    mgr.spawn(
        "/bin/sh",
        &["-c".to_string(), line.to_string()],
        cwd,
        &[("TERM".to_string(), "xterm-256color".to_string())],
        PtySize { rows: 24, cols: 80 },
        EnvPolicy::Inherit,
    )
    .expect("attach PTY spawns")
}

#[test]
fn terminal_session_survives_attach_client_death_and_reattaches() {
    if resolve_tmux_binary().is_none() {
        eprintln!("tmux not installed — skipping durability test");
        return;
    }

    let pid = std::process::id();
    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("MARKER");
    // Prefix unique to THIS test process: parallel tests (and the user's
    // own sessions) never see our sweep.
    let session_id = format!("duraproj:{pid}:dura:terminal:0");
    let prefix = format!("duraproj:{pid}:dura:terminal:");
    let _guard = SweepGuard {
        prefix: prefix.clone(),
    };
    let name = make_tmux_session_name(&session_id);
    let inner = build_terminal_session_command(dir.path(), "/bin/sh");
    let line = build_tmux_shell_line(&name, &inner);

    let mgr = PortablePtyManager;

    // 1+2: first attach — touch a marker and cd away from the start dir.
    // Readiness is gated on the MARKER FILE existing, NOT on output text:
    // the PTY echoes the typed command (which contains the echo token), so
    // matching output races the actual `touch`. A file can only exist once
    // the command has executed.
    let mut first = spawn_attach(&mgr, &line, dir.path());
    write_retry(
        &mut first,
        &format!(
            "touch {} && cd /var && echo DONE\n",
            ade_core::shell_escape::single_quote(&marker.to_string_lossy())
        ),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while !marker.exists() && Instant::now() < deadline {
        // Drain output so the PTY reader buffer doesn't stall the child.
        let mut sink = Vec::new();
        let _ = first.try_read(&mut sink);
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(marker.exists(), "touch never ran in the tmux shell");
    // Give tmux a beat to settle before we kill the client.
    std::thread::sleep(Duration::from_millis(200));
    assert!(session_exists(&name), "tmux session missing while attached");

    // 3: kill the attach client (simulated app crash).
    first.kill().expect("attach client killed");
    std::thread::sleep(Duration::from_millis(300));

    // 4: the tmux server still owns the session.
    assert!(
        session_exists(&name),
        "session died with the attach client — durability broken"
    );

    // 5: reattach with a fresh PTY on the SAME line → SAME shell. Proof of
    // identity is file-based (output matching races the echoed command
    // history that contains "/var"): write `pwd` to a proof file and read
    // it. A fresh session would start in the worktree dir, not /var.
    let proof = dir.path().join("CWDPROOF");
    let mut second = spawn_attach(&mgr, &line, dir.path());
    write_retry(
        &mut second,
        &format!(
            "pwd > {}\n",
            ade_core::shell_escape::single_quote(&proof.to_string_lossy())
        ),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while !proof.exists() && Instant::now() < deadline {
        let mut sink = Vec::new();
        let _ = second.try_read(&mut sink);
        std::thread::sleep(Duration::from_millis(50));
    }
    let cwd_seen = std::fs::read_to_string(&proof).unwrap_or_default();
    let cwd_seen = cwd_seen.trim();
    // macOS: /var is a symlink to /private/var — accept either rendering.
    assert!(
        cwd_seen == "/var" || cwd_seen == "/private/var",
        "reattached shell lost its cwd (fresh session?): {cwd_seen:?}"
    );

    // 6: the prefix sweep kills exactly this session.
    drop(second); // drop the client; the session itself is still alive.
    std::thread::sleep(Duration::from_millis(200));
    assert!(session_exists(&name), "session vanished before sweep");
    let killed = kill_tmux_sessions_by_prefix(&prefix);
    assert!(killed >= 1, "sweep killed {killed}, expected >= 1");
    std::thread::sleep(Duration::from_millis(200));
    assert!(!session_exists(&name), "session survived the sweep");
}

#[test]
fn sweep_never_touches_foreign_sessions() {
    if resolve_tmux_binary().is_none() {
        eprintln!("tmux not installed — skipping sweep test");
        return;
    }
    let pid = std::process::id();
    let prefix = format!("sweepcheck:{pid}:terminal:");
    let _guard = SweepGuard {
        prefix: prefix.clone(),
    };
    // Positive control: an ade session matching the prefix MUST die.
    let ours = make_tmux_session_name(&format!("{prefix}0"));
    // Foreign: a NON-ade-named live session must never be matched — even
    // though its name embeds the prefix text.
    let foreign = format!("ade-sweepcheck-{pid}-lookalike");
    for (name, cmd) in [(&ours, "sleep"), (&foreign, "sleep")] {
        let status = std::process::Command::new(tmux_command())
            .args(["new-session", "-d", "-s", name, cmd, "30"])
            .status()
            .expect("test session created");
        assert!(status.success(), "failed creating session {name}");
    }
    assert!(session_exists(&ours));
    assert!(session_exists(&foreign));

    let killed = kill_tmux_sessions_by_prefix(&prefix);
    assert_eq!(killed, 1, "sweep killed {killed}, expected exactly 1");
    assert!(!session_exists(&ours), "our session survived the sweep");
    assert!(
        session_exists(&foreign),
        "foreign session was destroyed by the sweep"
    );
    // Cleanup the foreign session.
    let _ = std::process::Command::new(tmux_command())
        .args(["kill-session", "-t", &foreign])
        .status();
}

#[test]
fn list_by_prefix_reports_survivors_with_attach_state() {
    // ADR-0028 depends on `list_tmux_sessions_by_prefix` reporting BOTH the
    // decoded ids and their attach state (slot reuse must skip attached
    // sessions). Needs a real tmux server — skip when absent.
    if resolve_tmux_binary().is_none() {
        eprintln!("tmux not installed — skipping list test");
        return;
    }
    let pid = std::process::id();
    let prefix = format!("listproj:{pid}:terminal:");
    let _guard = SweepGuard {
        prefix: prefix.clone(),
    };

    // Slot 0: detached survivor. Slot 1: gets attached via a live PTY client.
    let dir = tempfile::tempdir().expect("tempdir");
    for slot in [0u32, 1] {
        let name = make_tmux_session_name(&format!("{prefix}{slot}"));
        let status = std::process::Command::new(tmux_command())
            .args(["new-session", "-d", "-s", &name, "sleep", "30"])
            .status()
            .expect("test session created");
        assert!(status.success(), "failed creating session for slot {slot}");
    }

    // Both decode under our prefix; foreign prefixes never match.
    let live = list_tmux_sessions_by_prefix(&prefix);
    assert_eq!(live.len(), 2, "expected both slots listed, got {live:?}");
    assert!(
        live.iter().all(|s| !s.attached),
        "fresh sessions are detached: {live:?}"
    );
    let foreign = list_tmux_sessions_by_prefix(&format!("listproj:{pid}:OTHER:"));
    assert!(foreign.is_empty(), "{foreign:?}");

    // Attach slot 1 through the same PTY path the app uses; its attach
    // state must flip so slot reuse can skip it. Poll — client
    // registration is asynchronous.
    let attached_name = make_tmux_session_name(&format!("{prefix}1"));
    let line = format!("tmux -u attach-session -t \"{attached_name}\"");
    let mgr = PortablePtyManager;
    let _client = spawn_attach(&mgr, &line, dir.path());
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut slot1_attached = false;
    while Instant::now() < deadline {
        slot1_attached = list_tmux_sessions_by_prefix(&prefix)
            .iter()
            .find(|s| s.session_id == format!("{prefix}1"))
            .is_some_and(|s| s.attached);
        if slot1_attached {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(slot1_attached, "attached slot never reported attached");
    let slot0 = list_tmux_sessions_by_prefix(&prefix)
        .into_iter()
        .find(|s| s.session_id == format!("{prefix}0"))
        .expect("slot 0 still listed");
    assert!(!slot0.attached, "untouched slot flipped to attached");
}
