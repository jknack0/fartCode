//! E1-06 — Lifecycle scripts: env contract, input transform, status events,
//! dedupe, timeout, and output tail.

use std::path::PathBuf;
use std::sync::Arc;

use fartcode_core::events::{BroadcastEventBus, EventBus, InternalEvent};
use fartcode_core::terminals::lifecycle::{
    append_output_tail, create_lifecycle_script_terminal_id, fartcode_port, hash32,
    make_lifecycle_script_session_id, manual_run_options, strip_terminal_controls, task_env,
    terminal_input_for_script, LifecycleRunResult, LifecycleScript, LifecycleScriptService,
    LifecycleScriptType, RunOptions, FARTCODE_PORT_BASE, OUTPUT_TAIL_CAP,
};
use fartcode_terminal::PortablePtyManager;

fn env_of(run: &LifecycleRunResult) -> String {
    match run {
        LifecycleRunResult::Exited { output_tail, .. } => output_tail.clone(),
        _ => panic!("expected Exited, got {run:?}"),
    }
}

fn script(ty: LifecycleScriptType, body: &str) -> LifecycleScript {
    LifecycleScript {
        r#type: ty,
        script: body.to_string(),
        shell_setup: None,
    }
}

// ---------------------------------------------------------------------------
// Pure helpers (reference parity)
// ---------------------------------------------------------------------------

#[test]
fn terminal_input_normalizes_newlines_and_appends_exit() {
    // \r\n and \n both become \r; no exit → single trailing \r.
    assert_eq!(
        terminal_input_for_script("a\nb\r\nc", false, false),
        "a\rb\rc\r"
    );
    // exit → `; exit\r` on posix.
    assert_eq!(
        terminal_input_for_script("echo hi", true, false),
        "echo hi; exit\r"
    );
    // exit → `\rexit\r` on cmd.exe.
    assert_eq!(
        terminal_input_for_script("echo hi", true, true),
        "echo hi\rexit\r"
    );
    // Trailing \r(s) are trimmed before `; exit`.
    assert_eq!(terminal_input_for_script("a\r\r", true, false), "a; exit\r");
    // Empty script with exit.
    assert_eq!(terminal_input_for_script("", true, false), "; exit\r");
}

#[test]
fn strip_controls_and_tail_cap() {
    assert_eq!(strip_terminal_controls("hi\x1b[31mred\x1b[0m"), "hired");
    assert_eq!(
        strip_terminal_controls("a\x1b]0;title\x07b\x1b]8;;http://x\x1b\\c"),
        "abc"
    );
    assert_eq!(strip_terminal_controls("a\r\nb"), "a\nb");

    let tail = append_output_tail("", &"x".repeat(OUTPUT_TAIL_CAP + 100));
    assert_eq!(tail.len(), OUTPUT_TAIL_CAP, "tail capped at 16 KiB");
    assert!(tail.ends_with(&"x".repeat(100)), "keeps the NEWEST bytes");

    // Multi-byte UTF-8 output must never panic the cap (byte cut snapped to
    // a char boundary).
    let tail = append_output_tail("", &"🚀".repeat(OUTPUT_TAIL_CAP + 10));
    assert!(
        tail.len() <= OUTPUT_TAIL_CAP,
        "emoji tail capped: {}",
        tail.len()
    );
    assert!(
        tail.is_char_boundary(tail.len()),
        "cut lands on a char boundary"
    );
}

#[test]
fn fartcode_port_is_deterministic_and_isolated() {
    // Range: 50000..=59990, step 10 (ticket formula).
    for seed in ["", "a", "hello world", "/some/path"] {
        let p = fartcode_port(seed);
        assert!(
            (FARTCODE_PORT_BASE..=FARTCODE_PORT_BASE + 9990).contains(&p),
            "{p}"
        );
        assert_eq!((p - FARTCODE_PORT_BASE) % 10, 0, "step 10: {p}");
        assert_eq!(fartcode_port(seed), fartcode_port(seed), "deterministic");
    }
    // Port isolation: two tasks in the same project have different
    // worktree paths → different ports (acceptance 2).
    let p1 = fartcode_port("/proj/worktrees/a");
    let p2 = fartcode_port("/proj/worktrees/b");
    assert_ne!(p1, p2, "different worktrees → different ports");
    // hash32 sanity: FNV-1a known vector.
    assert_eq!(hash32(""), 0x811c_9dc5);
}

#[test]
fn task_env_contract_is_exact() {
    let env = task_env(
        "task-1",
        "Fix the Bug",
        &PathBuf::from("/wt/fix-the-bug"),
        &PathBuf::from("/proj"),
        "main",
        "/wt/fix-the-bug",
    );
    let map: std::collections::HashMap<_, _> = env.into_iter().collect();
    assert_eq!(map.get("FARTCODE_TASK_ID").unwrap(), "task-1");
    assert_eq!(
        map.get("FARTCODE_TASK_NAME").unwrap(),
        "fix-the-bug",
        "slugified"
    );
    assert_eq!(map.get("FARTCODE_TASK_PATH").unwrap(), "/wt/fix-the-bug");
    assert_eq!(map.get("FARTCODE_ROOT_PATH").unwrap(), "/proj");
    assert_eq!(map.get("FARTCODE_DEFAULT_BRANCH").unwrap(), "main");
    assert_eq!(
        map.get("FARTCODE_PORT").unwrap().parse::<u16>().unwrap(),
        fartcode_port("/wt/fix-the-bug")
    );

    // Fallbacks: empty slug → "task"; default branch.
    let env = task_env(
        "t",
        "!!!",
        PathBuf::from("/w").as_path(),
        PathBuf::from("/p").as_path(),
        "",
        "/w",
    );
    let map: std::collections::HashMap<_, _> = env.into_iter().collect();
    assert_eq!(map.get("FARTCODE_TASK_NAME").unwrap(), "task");
    assert_eq!(
        map.get("FARTCODE_DEFAULT_BRANCH").unwrap(),
        "main",
        "empty -> main fallback"
    );
}

#[test]
fn session_and_terminal_ids() {
    assert_eq!(
        create_lifecycle_script_terminal_id(LifecycleScriptType::Setup),
        "script-lifecycle-setup"
    );
    assert_eq!(
        create_lifecycle_script_terminal_id(LifecycleScriptType::Run),
        "script-lifecycle-run"
    );
    assert_eq!(
        create_lifecycle_script_terminal_id(LifecycleScriptType::Teardown),
        "script-lifecycle-teardown"
    );
    assert_eq!(
        make_lifecycle_script_session_id("proj", "ws", LifecycleScriptType::Run),
        "proj:ws:script-lifecycle-run"
    );
}

// ---------------------------------------------------------------------------
// PTY execution (acceptance: scripts execute with correct env; output visible;
// port isolation; failure + timeout semantics)
// ---------------------------------------------------------------------------

struct Fixture {
    service: Arc<LifecycleScriptService>,
    cwd: tempfile::TempDir,
    /// Subscribed at creation so status events emitted during a run are seen.
    events: tokio::sync::broadcast::Receiver<InternalEvent>,
}

impl Fixture {
    fn new() -> Self {
        let bus = Arc::new(BroadcastEventBus::new(64));
        let service = Arc::new(LifecycleScriptService::new(
            Arc::new(PortablePtyManager),
            bus.clone(),
        ));
        let cwd = tempfile::tempdir().unwrap();
        let events = bus.subscribe();
        Self {
            service,
            cwd,
            events,
        }
    }

    fn env(&self, task_id: &str, port_seed: &str) -> Vec<(String, String)> {
        task_env(
            task_id,
            "Lifecycle Test",
            self.cwd.path(),
            self.cwd.path(),
            "main",
            port_seed,
        )
    }

    fn events_for(&mut self, session_id: &str) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(ev) = self.events.try_recv() {
            if let InternalEvent::LifecycleScriptStatusChanged {
                session_id: sid,
                status,
                ..
            } = ev
            {
                if sid == session_id {
                    out.push(status);
                }
            }
        }
        out
    }

    /// Pumps events until `status` is seen for the session (bounded) — used
    /// instead of sleeps so timing-sensitive tests don't race CI load.
    fn wait_for_status(&mut self, session_id: &str, status: &str) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            while let Ok(ev) = self.events.try_recv() {
                if let InternalEvent::LifecycleScriptStatusChanged {
                    session_id: sid,
                    status: st,
                    ..
                } = ev
                {
                    if sid == session_id && st == status {
                        return true;
                    }
                }
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

#[test]
fn setup_script_runs_with_env_and_emits_statuses() {
    let mut fx = Fixture::new();
    let sid = make_lifecycle_script_session_id("p", "w", LifecycleScriptType::Setup);
    let run = fx
        .service
        .run(
            &sid,
            &script(
                LifecycleScriptType::Setup,
                "echo $FARTCODE_TASK_ID:$FARTCODE_PORT",
            ),
            fx.cwd.path(),
            &fx.env("task-42", "/wt/a"),
            &RunOptions {
                wait_for_exit: true,
                exit: true,
                ..Default::default()
            },
        )
        .unwrap();
    let out = env_of(&run);
    assert!(
        out.contains("task-42"),
        "FARTCODE_TASK_ID in output: {out:?}"
    );
    let port = fartcode_port("/wt/a").to_string();
    assert!(out.contains(&port), "FARTCODE_PORT in output: {out:?}");
    assert_eq!(
        fx.events_for(&sid),
        vec!["running", "succeeded"],
        "status event order"
    );
}

#[test]
fn shell_arithmetic_on_ade_port_works() {
    let fx = Fixture::new();
    let sid = make_lifecycle_script_session_id("p", "w", LifecycleScriptType::Setup);
    let port = fartcode_port("/wt/arith");
    let run = fx
        .service
        .run(
            &sid,
            &script(LifecycleScriptType::Setup, "echo $((FARTCODE_PORT + 1))"),
            fx.cwd.path(),
            &fx.env("t", "/wt/arith"),
            &RunOptions {
                wait_for_exit: true,
                exit: true,
                ..Default::default()
            },
        )
        .unwrap();
    let out = env_of(&run);
    assert!(
        out.contains(&(port + 1).to_string()),
        "arithmetic sees FARTCODE_PORT ({port}): {out:?}"
    );
}

#[test]
fn failure_surfaces_unless_continue_on_failure() {
    let mut fx = Fixture::new();
    let sid = make_lifecycle_script_session_id("p", "w", LifecycleScriptType::Setup);
    let opts = RunOptions {
        wait_for_exit: true,
        exit: true,
        surface_failure: true,
        ..Default::default()
    };
    let err = fx
        .service
        .run(
            &sid,
            &script(LifecycleScriptType::Setup, "exit 3"),
            fx.cwd.path(),
            &fx.env("t", "/w"),
            &opts,
        )
        .unwrap_err();
    assert!(
        matches!(
            err,
            fartcode_core::Error::LifecycleScriptFailed {
                exit_code: Some(3),
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(fx.events_for(&sid), vec!["running", "failed"]);

    // continue_on_failure → result, not error.
    let sid2 = make_lifecycle_script_session_id("p", "w2", LifecycleScriptType::Setup);
    let run = fx
        .service
        .run(
            &sid2,
            &script(LifecycleScriptType::Setup, "exit 3"),
            fx.cwd.path(),
            &fx.env("t", "/w"),
            &RunOptions {
                wait_for_exit: true,
                exit: true,
                surface_failure: true,
                continue_on_failure: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(matches!(
        run,
        LifecycleRunResult::Exited {
            exit_code: Some(3),
            ..
        }
    ));
}

#[test]
fn timeout_stops_the_script() {
    let mut fx = Fixture::new();
    let sid = make_lifecycle_script_session_id("p", "w", LifecycleScriptType::Run);
    let err = fx
        .service
        .run(
            &sid,
            &script(LifecycleScriptType::Run, "sleep 30"),
            fx.cwd.path(),
            &fx.env("t", "/w"),
            &RunOptions {
                wait_for_exit: true,
                exit: false,
                timeout_ms: Some(200),
                surface_failure: true,
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(
        matches!(err, fartcode_core::Error::LifecycleScriptTimeout(_)),
        "{err:?}"
    );
    assert_eq!(fx.events_for(&sid), vec!["running", "stopped"]);

    // Without surface_failure the result is a distinct TimedOut variant,
    // not a misleading clean exit.
    let sid2 = make_lifecycle_script_session_id("p", "w", LifecycleScriptType::Run);
    let run = fx
        .service
        .run(
            &sid2,
            &script(LifecycleScriptType::Run, "sleep 30"),
            fx.cwd.path(),
            &fx.env("t", "/w"),
            &RunOptions {
                wait_for_exit: true,
                exit: false,
                timeout_ms: Some(150),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        matches!(run, LifecycleRunResult::TimedOut { .. }),
        "{run:?}"
    );
}

#[test]
fn fire_and_forget_returns_started_and_reports_status() {
    let mut fx = Fixture::new();
    let sid = make_lifecycle_script_session_id("p", "w", LifecycleScriptType::Setup);
    let run = fx
        .service
        .run(
            &sid,
            &script(LifecycleScriptType::Setup, "echo bg-done"),
            fx.cwd.path(),
            &fx.env("t", "/w"),
            &RunOptions {
                wait_for_exit: false,
                exit: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(matches!(run, LifecycleRunResult::Started), "{run:?}");
    assert!(
        fx.wait_for_status(&sid, "succeeded"),
        "background run reports succeeded"
    );
}

#[test]
fn active_sessions_dedupe() {
    let fx = Fixture::new();
    let sid = make_lifecycle_script_session_id("p", "w", LifecycleScriptType::Setup);

    // First run holds the session slot for ~1s (no `exit` typed, so the
    // shell stays alive until the runner's 5s timeout).
    let svc = fx.service.clone();
    let cwd = fx.cwd.path().to_path_buf();
    let env = fx.env("t", "/w");
    let sid1 = sid.clone();
    let handle = std::thread::spawn(move || {
        svc.run(
            &sid1,
            &script(LifecycleScriptType::Setup, "sleep 1"),
            &cwd,
            &env,
            &RunOptions {
                wait_for_exit: true,
                exit: true,
                timeout_ms: Some(5000),
                ..Default::default()
            },
        )
    });

    // Let the first run grab the slot, then probe for the duplicate.
    std::thread::sleep(std::time::Duration::from_millis(200));
    let dup = fx.service.run(
        &sid,
        &script(LifecycleScriptType::Setup, "echo dup"),
        fx.cwd.path(),
        &fx.env("t", "/w"),
        &RunOptions {
            ..Default::default()
        },
    );
    assert!(matches!(dup.unwrap(), LifecycleRunResult::AlreadyRunning));
    let hold_result = handle.join().unwrap().unwrap();
    assert!(
        matches!(hold_result, LifecycleRunResult::Exited { .. }),
        "hold run should exit cleanly, got: {hold_result:?}"
    );

    // Session released after the first run completes.
    let again = fx.service.run(
        &sid,
        &script(LifecycleScriptType::Setup, "echo again"),
        fx.cwd.path(),
        &fx.env("t", "/w"),
        &RunOptions {
            wait_for_exit: true,
            exit: true,
            ..Default::default()
        },
    );
    assert!(matches!(again.unwrap(), LifecycleRunResult::Exited { .. }));
}

#[test]
fn manual_run_options_match_the_reference() {
    let m = manual_run_options();
    assert!(m.wait_for_exit && m.respawn_after_exit && m.log_failure && m.surface_failure);
    assert!(!m.exit && !m.continue_on_failure);
    assert_eq!(m.timeout_ms, None);
}

#[test]
fn shell_setup_is_prepended() {
    let fx = Fixture::new();
    let sid = make_lifecycle_script_session_id("p", "w", LifecycleScriptType::Setup);
    let s = LifecycleScript {
        r#type: LifecycleScriptType::Setup,
        script: "echo BODY".to_string(),
        shell_setup: Some("echo SHELLSETUP".to_string()),
    };
    let run = fx
        .service
        .run(
            &sid,
            &s,
            fx.cwd.path(),
            &fx.env("t", "/w"),
            &RunOptions {
                wait_for_exit: true,
                exit: true,
                ..Default::default()
            },
        )
        .unwrap();
    let out = env_of(&run);
    assert!(
        out.contains("SHELLSETUP") && out.contains("BODY"),
        "{out:?}"
    );
}
