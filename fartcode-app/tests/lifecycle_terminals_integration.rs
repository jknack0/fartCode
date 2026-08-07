//! E1-06 acceptance (app wiring): lifecycle script terminals run through
//! the real PTY layer, dedupe in-flight runs, and are RETAINED after the
//! script exits so a later tab attach still finds the entry + output tail
//! (plain shells keep the drop-on-exit behavior).

use std::time::{Duration, Instant};

use base64::Engine as _;
use fartcode_app_lib::terminals::{TerminalManager, TerminalSpec};
use fartcode_core::terminals::lifecycle::LifecycleScriptType;

fn manager() -> TerminalManager<tauri::test::MockRuntime> {
    TerminalManager::new(tauri::test::mock_app().handle().clone())
}

/// Polls `f` every 25 ms until it returns true (timeout 5 s).
fn eventually(mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("condition not met within 5s");
}

#[test]
fn lifecycle_terminal_runs_dedupes_and_is_retained_after_exit() {
    let manager = manager();
    let cwd = tempfile::tempdir().unwrap();
    let args = vec![
        "-c".to_string(),
        "echo lifecycle-ran; sleep 0.4".to_string(),
    ];
    let env = vec![("FARTCODE_TASK_ID".to_string(), "t1".to_string())];

    let id = manager
        .open(TerminalSpec {
            task_id: "t1",
            project_id: "p1",
            agent: None,
            tmux: false,
            program: "/bin/sh",
            args: &args,
            env: &env,
            cwd: cwd.path(),
            rows: 24,
            cols: 80,
            lifecycle: Some(LifecycleScriptType::Setup),
        })
        .expect("open");

    // In-flight: a rerun of the same type reattaches instead of spawning.
    eventually(|| manager.find_running_lifecycle("t1", "setup").is_some());
    let running = manager
        .find_running_lifecycle("t1", "setup")
        .expect("running");
    assert_eq!(running, id);

    // After the script exits the entry is RETAINED (kind lifecycle) and no
    // longer considered running — a rerun mints a fresh terminal.
    eventually(|| manager.find_running_lifecycle("t1", "setup").is_none());
    let listed = manager.list_for_task("t1");
    assert!(
        listed.iter().any(|t| t.id == id),
        "finished lifecycle terminal must stay listed for tab attach"
    );
    let info = listed.iter().find(|t| t.id == id).unwrap();
    assert_eq!(info.kind, "lifecycle");
    assert_eq!(info.script_type.as_deref(), Some("setup"));
    assert!(info.agent.is_none());

    // The output tail survived the exit (base64 replay for the tab).
    let tail = manager.tail(&id).expect("tail retained");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(tail)
        .unwrap();
    let text = String::from_utf8_lossy(&decoded);
    assert!(
        text.contains("lifecycle-ran"),
        "tail must contain the script output: {text:?}"
    );

    // Closing the tab drops the entry (a rerun then starts clean).
    manager.close(&id);
    assert!(!manager.list_for_task("t1").iter().any(|t| t.id == id));
}

#[test]
fn plain_shell_terminals_still_drop_on_exit() {
    let manager = manager();
    let cwd = tempfile::tempdir().unwrap();
    let args = vec!["-c".to_string(), "true".to_string()];

    let id = manager
        .open(TerminalSpec {
            task_id: "t2",
            project_id: "p1",
            agent: None,
            tmux: false,
            program: "/bin/sh",
            args: &args,
            env: &[],
            cwd: cwd.path(),
            rows: 24,
            cols: 80,
            lifecycle: None,
        })
        .expect("open");

    // Plain entries are removed once the process exits (unchanged
    // behavior — the frontend respawns them on restore).
    eventually(|| !manager.list_for_task("t2").iter().any(|t| t.id == id));
    assert!(manager.list_for_task("t2").is_empty());
}

#[test]
fn lifecycle_kind_surfaces_in_listing() {
    let manager = manager();
    let cwd = tempfile::tempdir().unwrap();
    let args = vec!["-c".to_string(), "sleep 0.2".to_string()];

    let id = manager
        .open(TerminalSpec {
            task_id: "t3",
            project_id: "p1",
            agent: None,
            tmux: false,
            program: "/bin/sh",
            args: &args,
            env: &[],
            cwd: cwd.path(),
            rows: 24,
            cols: 80,
            lifecycle: Some(LifecycleScriptType::Run),
        })
        .expect("open");
    eventually(|| manager.find_running_lifecycle("t3", "run").is_some());
    let listed = manager.list_for_task("t3");
    let info = listed.iter().find(|t| t.id == id).unwrap();
    assert_eq!(info.kind, "lifecycle");
    assert_eq!(info.script_type.as_deref(), Some("run"));
    manager.close(&id);
}
