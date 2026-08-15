//! #134 (grill AC3, live arm): `persisted_sessions` reports live tmux
//! sessions by prefix — including orphans no process shows — and consults
//! no project setting (there is no settings object anywhere near this
//! path). Runs against the REAL tmux binary; skips when absent.
//!
//! The prefix embeds this pid: the tmux server is a process-wide shared
//! resource and a parallel test run must not see (or sweep) our sessions.

use fartcode_app_lib::terminals::TerminalManager;
use fartcode_core::pty::tmux::{
    kill_tmux_sessions_by_prefix, make_tmux_session_name, resolve_tmux_binary,
};

/// Kills the test sessions no matter how the test ends (they live in the
/// user's real tmux server).
struct SweepGuard {
    prefix: String,
}

impl Drop for SweepGuard {
    fn drop(&mut self) {
        let _ = kill_tmux_sessions_by_prefix(&self.prefix);
    }
}

#[test]
fn persisted_sessions_reports_live_sessions_this_process_never_opened() {
    let Some(tmux) = resolve_tmux_binary() else {
        eprintln!("tmux absent — skipping");
        return;
    };
    let project = format!("p134-{}", std::process::id());
    let task = "t1";
    let prefix = format!("{project}:{task}:terminal:");
    let _guard = SweepGuard {
        prefix: prefix.clone(),
    };
    // Orphan sessions created OUTSIDE the manager (a crashed previous app
    // instance). Slot 1 before slot 0 — the listing must sort, not echo.
    for slot in [1_u32, 0] {
        let name = make_tmux_session_name(&format!("{prefix}{slot}"));
        let status = std::process::Command::new(&tmux.command)
            .args(["new-session", "-d", "-s", &name, "sleep", "30"])
            .status()
            .expect("tmux new-session spawns");
        assert!(status.success(), "tmux new-session failed for {name}");
    }

    let manager: TerminalManager<tauri::test::MockRuntime> =
        TerminalManager::new(tauri::test::mock_app().handle().clone());

    // No settings read, no gate: the orphans are listed, slot-ordered.
    let listed = manager.persisted_sessions(&project, task);
    assert_eq!(listed, vec![format!("{prefix}0"), format!("{prefix}1")]);
}

/// #134 adversarial finding 1 (mutation-killer): the COMMAND path must
/// ignore the project's tmux setting. Seeds a tmux-OFF project (defaults:
/// `tmux = None` → off — exactly what a gated impl would read as "skip")
/// with a REAL live session under the task's prefix. A setting-gated
/// `terminal_list_persisted_blocking` returns `[]` here and FAILS; the
/// shipped no-gate implementation returns the session.
#[test]
fn list_persisted_command_ignores_the_tmux_setting() {
    let Some(tmux) = resolve_tmux_binary() else {
        eprintln!("tmux absent — skipping");
        return;
    };
    let app = fartcode_app_lib::app::App::init(Some(":memory:")).unwrap();
    let project = format!("p134cmd-{}", std::process::id());
    {
        let conn = app.db.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO projects (id, name, path) VALUES (?1, 'p', '/tmp/p134cmd')",
            [&project],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (id, project_id, name, status) VALUES ('t1', ?1, 'demo', 'in_progress')",
            [&project],
        )
        .unwrap();
    }
    let prefix = format!("{project}:t1:terminal:");
    let _guard = SweepGuard {
        prefix: prefix.clone(),
    };
    let name = make_tmux_session_name(&format!("{prefix}0"));
    let status = std::process::Command::new(&tmux.command)
        .args(["new-session", "-d", "-s", &name, "sleep", "30"])
        .status()
        .expect("tmux new-session spawns");
    assert!(status.success(), "tmux new-session failed for {name}");

    let manager: TerminalManager<tauri::test::MockRuntime> =
        TerminalManager::new(tauri::test::mock_app().handle().clone());
    let listed = fartcode_app_lib::commands::terminals::terminal_list_persisted_blocking(
        &manager, &app, "t1",
    )
    .expect("command succeeds");
    assert_eq!(listed, vec![format!("{prefix}0")]);
}

