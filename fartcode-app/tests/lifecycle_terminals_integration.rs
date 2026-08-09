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
            remove: &[],
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

    // While in flight the listing reports running=true, no exit code yet.
    let info = manager
        .list_for_task("t1")
        .into_iter()
        .find(|t| t.id == id)
        .expect("listed while running");
    assert!(info.running, "in-flight entry must report running");
    assert_eq!(info.exit_code, None, "no exit code before the script ends");

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
    // Retained finished entries carry the exit info (7b drawer: `setup ✗
    // exit 1` needs the code; this clean run exits 0).
    assert!(!info.running, "finished entry must report running=false");
    assert_eq!(info.exit_code, Some(0), "clean exit must surface code 0");

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
    // 7b log body: the run ends with a trailing exit line in the retained
    // tail — dim (ANSI 2) for a clean exit.
    assert!(
        text.contains("\u{1b}[2msetup exited 0 \u{b7} "),
        "tail must end with the dim exit line: {text:?}"
    );

    // Closing the tab drops the entry (a rerun then starts clean).
    manager.close(&id);
    assert!(!manager.list_for_task("t1").iter().any(|t| t.id == id));
}

/// 7b log body: a FAILING lifecycle run appends a RED (ANSI 31) trailing
/// exit line with the non-zero code into the retained tail.
#[test]
fn failed_lifecycle_run_tail_ends_with_a_red_exit_line() {
    let manager = manager();
    let cwd = tempfile::tempdir().unwrap();
    let args = vec!["-c".to_string(), "exit 7".to_string()];

    let id = manager
        .open(TerminalSpec {
            task_id: "t4",
            project_id: "p1",
            agent: None,
            tmux: false,
            program: "/bin/sh",
            args: &args,
            env: &[],
            remove: &[],
            cwd: cwd.path(),
            rows: 24,
            cols: 80,
            lifecycle: Some(LifecycleScriptType::Setup),
        })
        .expect("open");

    eventually(|| {
        manager
            .list_for_task("t4")
            .iter()
            .any(|t| t.id == id && !t.running)
    });
    let info = manager
        .list_for_task("t4")
        .into_iter()
        .find(|t| t.id == id)
        .unwrap();
    assert_eq!(info.exit_code, Some(7));

    let tail = manager.tail(&id).expect("tail retained");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(tail)
        .unwrap();
    let text = String::from_utf8_lossy(&decoded);
    assert!(
        text.contains("\u{1b}[31msetup exited 7 \u{b7} "),
        "failed run must append the red exit line: {text:?}"
    );
    manager.close(&id);
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
            remove: &[],
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
            remove: &[],
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
