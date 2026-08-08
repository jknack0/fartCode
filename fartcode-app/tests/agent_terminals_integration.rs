//! ADR-0033 acceptance: ONE agent terminal per task. A second
//! `terminal_open_agent` while one is live reattaches the existing PTY
//! (same id, no second spawn); a different task still mints its own;
//! plain shells are unaffected (each open spawns).

use std::time::{Duration, Instant};

use fartcode_app_lib::terminals::{TerminalManager, TerminalSpec};

fn manager() -> TerminalManager<tauri::test::MockRuntime> {
    TerminalManager::new(tauri::test::mock_app().handle().clone())
}

fn open_agent(
    manager: &TerminalManager<tauri::test::MockRuntime>,
    task_id: &str,
    agent: &str,
) -> String {
    let cwd = std::env::temp_dir();
    manager
        .open(TerminalSpec {
            task_id,
            project_id: "p1",
            agent: Some(agent),
            tmux: false,
            program: "/bin/sh",
            args: &["-c".into(), "sleep 30".into()],
            env: &[],
            remove: &[],
            cwd: &cwd,
            rows: 24,
            cols: 80,
            lifecycle: None::<fartcode_core::terminals::lifecycle::LifecycleScriptType>,
        })
        .expect("open agent terminal")
}

fn count_agent_kinds(manager: &TerminalManager<tauri::test::MockRuntime>, task_id: &str) -> usize {
    manager
        .list_for_task(task_id)
        .iter()
        .filter(|t| t.kind == "agent")
        .count()
}

#[test]
fn second_agent_open_same_task_reattaches() {
    let manager = manager();
    let first = open_agent(&manager, "t1", "claude");
    // Even under a different provider name — one agent terminal per task.
    let second = open_agent(&manager, "t1", "codex");
    assert_eq!(
        second, first,
        "a live agent terminal must reattach, not respawn"
    );
    assert_eq!(count_agent_kinds(&manager, "t1"), 1);
}

#[test]
fn other_tasks_get_their_own_agent_terminal() {
    let manager = manager();
    let a = open_agent(&manager, "t1", "claude");
    let b = open_agent(&manager, "t2", "claude");
    assert_ne!(a, b);
    assert_eq!(count_agent_kinds(&manager, "t1"), 1);
    assert_eq!(count_agent_kinds(&manager, "t2"), 1);
}

#[test]
fn exited_agent_terminal_allows_a_fresh_spawn() {
    let manager = manager();
    let cwd = std::env::temp_dir();
    let first = manager
        .open(TerminalSpec {
            task_id: "t1",
            project_id: "p1",
            agent: Some("claude"),
            tmux: false,
            program: "/bin/sh",
            args: &["-c".into(), "exit 0".into()],
            env: &[],
            remove: &[],
            cwd: &cwd,
            rows: 24,
            cols: 80,
            lifecycle: None::<fartcode_core::terminals::lifecycle::LifecycleScriptType>,
        })
        .expect("open agent terminal");

    // Wait for the exit pump to mark the entry exited (drops it for plain
    // agent terminals), then a new open mints a fresh id.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if manager.find_running_agent("t1").is_none() {
            break;
        }
        assert!(Instant::now() < deadline, "agent terminal never exited");
        std::thread::sleep(Duration::from_millis(25));
    }
    let second = open_agent(&manager, "t1", "claude");
    assert_ne!(
        second, first,
        "an exited agent terminal must not be reattached"
    );
}

#[test]
fn plain_shells_still_spawn_fresh_each_open() {
    let manager = manager();
    let cwd = std::env::temp_dir();
    let open_shell = || {
        manager
            .open(TerminalSpec {
                task_id: "t1",
                project_id: "p1",
                agent: None,
                tmux: false,
                program: "/bin/sh",
                args: &["-c".into(), "sleep 30".into()],
                env: &[],
                remove: &[],
                cwd: &cwd,
                rows: 24,
                cols: 80,
                lifecycle: None::<fartcode_core::terminals::lifecycle::LifecycleScriptType>,
            })
            .expect("open shell")
    };
    assert_ne!(open_shell(), open_shell());
}
