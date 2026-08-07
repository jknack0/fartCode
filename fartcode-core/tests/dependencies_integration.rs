//! Host dependency detection + install/update/uninstall tests (ticket E3-02
//! acceptance criteria).
//!
//! PATH-scoped tests are serialized via `PATH_LOCK` (setting `PATH` is
//! process-wide in Rust). Detection never touches real app data — the kv
//! cache lives in the `:memory:` DB.

use std::path::Path;
use std::sync::{Arc, Mutex};

use fartcode_core::db::{Db, SqliteDb};
use fartcode_core::dependencies::host_dependency::{host_dependency, InstallPlan, InstallType};
use fartcode_core::dependencies::{
    CurlManager, DependencyManager, HostDependencyStore, InstallRunner, NpmManager, PipManager,
};
use fartcode_core::Error;

/// Serializes tests that mutate the process PATH.
static PATH_LOCK: Mutex<()> = Mutex::new(());

struct Fixture {
    db: Arc<SqliteDb>,
    store: HostDependencyStore,
}

impl Fixture {
    fn new() -> Self {
        let db = SqliteDb::init_in_memory().unwrap();
        let store = HostDependencyStore::new(db.clone(), Arc::new(RecordingRunner::default()));
        Self { db, store }
    }
}

/// Records argv instead of spawning anything.
#[derive(Debug, Default, Clone)]
struct RecordingRunner {
    calls: Arc<std::sync::Mutex<Vec<Vec<String>>>>,
}

impl InstallRunner for RecordingRunner {
    fn run(&self, argv: &[String], _sink: &mut dyn FnMut(&str)) -> Result<bool, Error> {
        self.calls.lock().unwrap().push(argv.to_vec());
        Ok(true)
    }
}

/// A fake agent CLI in `dir`: `$name --version` prints a version.
fn install_fake_cli(dir: &Path, name: &str) {
    let script = dir.join(name);
    std::fs::write(
        &script,
        format!("#!/bin/sh\necho \"{name} version 9.9.9\"\n"),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn with_path<T>(path: &Path, f: impl FnOnce() -> T) -> T {
    let _guard = PATH_LOCK.lock().unwrap();
    let old = std::env::var_os("PATH");
    std::env::set_var("PATH", path);
    let result = f();
    match old {
        Some(p) => std::env::set_var("PATH", p),
        None => std::env::remove_var("PATH"),
    }
    result
}

// -- acceptance 1: detection ------------------------------------------------

#[test]
fn claude_on_path_detects_installed_codex_not_installed() {
    let fx = Fixture::new();
    let tmp = tempfile::tempdir().unwrap();
    install_fake_cli(tmp.path(), "claude");

    with_path(tmp.path(), || {
        // claude is on PATH → installed with a version + path.
        let claude = fx.store.detect("claude").unwrap();
        assert!(claude.installed, "claude should be detected: {claude:?}");
        assert_eq!(claude.version.as_deref(), Some("claude version 9.9.9"));
        assert!(claude.path.as_deref().unwrap().contains("claude"));

        // codex is not on PATH → not installed, WITH install instructions.
        let codex = fx.store.detect("codex").unwrap();
        assert!(!codex.installed);
        let instructions = fx.store.install_instructions("codex").unwrap();
        assert_eq!(instructions, "npm install -g @openai/codex");
        assert!(
            codex.install_instructions.is_none(),
            "detect() carries no instructions"
        );
    });
}

#[test]
fn missing_provider_is_a_typed_error() {
    let fx = Fixture::new();
    let err = fx.store.detect("not-a-provider").unwrap_err();
    assert!(matches!(err, Error::AgentNotFound(_)));
}

// -- acceptance 3: re-detect picks up new installs --------------------------

#[test]
fn redetect_picks_up_new_installs() {
    let fx = Fixture::new();
    let tmp = tempfile::tempdir().unwrap();

    with_path(tmp.path(), || {
        let before = fx.store.detect("claude").unwrap();
        assert!(!before.installed);

        install_fake_cli(tmp.path(), "claude");
        let after = fx.store.detect("claude").unwrap();
        assert!(after.installed);
    });
}

// -- cache + TTL ------------------------------------------------------------

#[test]
fn status_is_cached_and_ttl_forces_redetect() {
    let fx = Fixture::new();
    let tmp = tempfile::tempdir().unwrap();
    install_fake_cli(tmp.path(), "claude");

    with_path(tmp.path(), || {
        let first = fx.store.detect("claude").unwrap();
        let checked = first.checked_at.clone();

        // Cached read: same status, no re-probe (checked_at unchanged).
        let cached = fx.store.get_status("claude").unwrap();
        assert_eq!(cached.checked_at, checked);

        // Expire the cache: a stale checked_at forces re-detection.
        let mut stale = first;
        stale.checked_at = Some("1".into());
        let raw = serde_json::to_string(&stale).unwrap();
        fx.db.kv_set("host-dep:claude", &raw).unwrap();
        let fresh = fx.store.get_status("claude").unwrap();
        assert_ne!(fresh.checked_at.as_deref(), Some("1"));
        assert!(fresh.installed);
    });
}

// -- install/update/uninstall run through the manager + runner --------------

#[test]
fn install_runs_the_manager_command_and_revalidates() {
    let calls = Arc::new(std::sync::Mutex::new(Vec::<Vec<String>>::new()));
    let rec = RecordingRunner {
        calls: calls.clone(),
    };
    let db = SqliteDb::init_in_memory().unwrap();
    let store = HostDependencyStore::new(db.clone(), Arc::new(rec));

    let mut sink = |_line: &str| {};
    let status = store.install("codex", &mut sink).unwrap();
    let argv = calls.lock().unwrap().first().cloned().unwrap();
    assert_eq!(argv, vec!["npm", "install", "-g", "@openai/codex"]);
    // The post-install re-detect depends on the ambient PATH (the dev machine
    // may or may not have codex) — assert the cache was written, not the flag.
    assert!(status.checked_at.is_some(), "cache written after install");

    // uninstall reverses and clears the cached status.
    store.uninstall("codex", &mut sink).unwrap();
    let argv = calls.lock().unwrap().last().cloned().unwrap();
    assert_eq!(argv, vec!["npm", "uninstall", "-g", "@openai/codex"]);
    let cached = store.get_status("codex").unwrap();
    assert!(!cached.installed);
}

#[test]
fn curl_managers_run_via_the_shell() {
    // claude's curl installer is a shell one-liner.
    let claude = host_dependency("claude").unwrap();
    let plan = claude.install_plan.unwrap();
    assert_eq!(plan.install_type, InstallType::Curl);

    let argv = CurlManager.install(&plan).unwrap();
    assert_eq!(argv[0], "sh");
    assert_eq!(argv[1], "-c");
    assert!(argv[2].contains("claude.ai/install.sh"));

    let uninstall = CurlManager.uninstall(&plan).unwrap();
    assert_eq!(uninstall, vec!["sh", "-c", "claude uninstall"]);

    // npm + pip builders.
    let codex = host_dependency("codex").unwrap();
    let plan = codex.install_plan.unwrap();
    assert_eq!(
        NpmManager.update(&plan).unwrap(),
        vec!["npm", "update", "-g", "@openai/codex"]
    );
    let pip_plan = InstallPlan {
        install_type: InstallType::Pip,
        install_args: vec!["some-agent".into()],
        shell_command: None,
        uninstall_command: None,
        update_repo: None,
    };
    assert_eq!(
        PipManager.install(&pip_plan).unwrap(),
        vec!["pip", "install", "some-agent"]
    );
}

#[test]
fn process_runner_survives_a_large_stderr_flood() {
    // Regression: the old sequential drain deadlocked when a child wrote
    // >64KB (pipe buffer) to stderr while stdout stayed open. The concurrent
    // drain must complete.
    use fartcode_core::dependencies::{InstallRunner, ProcessInstallRunner};
    let runner = ProcessInstallRunner;
    let script = "for i in $(seq 1 4000); do echo \"err line $i\" >&2; done; echo done";
    let mut output = String::new();
    let ok = runner
        .run(&["sh".into(), "-c".into(), script.into()], &mut |chunk| {
            output.push_str(chunk)
        })
        .unwrap();
    assert!(ok);
    assert!(output.contains("err line 4000"));
    assert!(output.contains("done"));
}

#[test]
fn refresh_detects_all_providers() {
    let fx = Fixture::new();
    let tmp = tempfile::tempdir().unwrap();
    install_fake_cli(tmp.path(), "claude");

    with_path(tmp.path(), || {
        let statuses = fx.store.refresh().unwrap();
        assert_eq!(statuses.len(), 35, "all providers detected");
        let claude = statuses.iter().find(|s| s.provider_id == "claude").unwrap();
        assert!(claude.installed);
        let codex = statuses.iter().find(|s| s.provider_id == "codex").unwrap();
        assert!(!codex.installed);
    });
}
