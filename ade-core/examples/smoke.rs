//! Smoke-test checkpoint for E1-02 (settings store) — boot the DB, exercise
//! app settings, project settings, `.ade.json` precedence, share-with-team,
//! legacy migration, and KV — then print a summary.
//!
//! Run from the workspace root:
//!
//! ```sh
//! cargo run -p ade-core --example smoke
//! ```
//!
//! Everything lives in a fresh temp directory; nothing touches real app data.
//! Exit code 0 = SMOKE OK, non-zero = a check failed.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use ade_core::db::{Db, SqliteDb};
use ade_core::events::EventBus;
use ade_core::projects::ProjectStore;
use ade_core::settings::{
    DbSettingsStore, DefaultBranch, KvStore, ProjectGroup, SettingsStore, DEFAULT_AGENT,
    LOCAL_PROJECT, PROJECT, PROJECT_CONFIG_FILE,
};

fn main() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("smoke.db");
    println!("== ade smoke — E1-02 settings store ==");
    println!("db: {}\n", db_path.display());

    let db = SqliteDb::init(Some(db_path.to_str().unwrap())).expect("db init");
    let settings = DbSettingsStore::new(db.clone());
    let kv = KvStore::new(db.clone());

    let mut failures = 0;
    let mut check = |ok: bool, label: &str| {
        println!("[{}] {label}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            failures += 1;
        }
    };

    // -- 1. App settings defaults ------------------------------------------
    let project_defaults = settings.get(&PROJECT).unwrap();
    println!("   project defaults: {project_defaults:#?}");
    check(
        project_defaults.branch_prefix == "ade",
        "default branch prefix is 'ade'",
    );
    check(
        project_defaults.push_on_create,
        "default pushOnCreate is true",
    );

    // -- 2. Delta write: override one field ---------------------------------
    let override_project = ProjectGroup {
        push_on_create: false,
        ..Default::default()
    };
    settings.set(&PROJECT, override_project).unwrap();
    let stored: String = db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'project'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    println!("   stored delta: {stored}");
    check(
        !settings.get(&PROJECT).unwrap().push_on_create,
        "overridden pushOnCreate is false",
    );
    check(
        settings.get(&PROJECT).unwrap().branch_prefix == "ade",
        "untouched field still deep-merges from defaults",
    );

    // -- 3. Setting back to the default deletes the row ---------------------
    settings.set(&PROJECT, ProjectGroup::default()).unwrap();
    let rows: i64 = db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM app_settings WHERE key = 'project'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    check(rows == 0, "setting to the default deleted the row");

    // -- 4. Scalar setting + reset -------------------------------------------
    settings.set(&DEFAULT_AGENT, "gemini".into()).unwrap();
    check(
        settings.get(&DEFAULT_AGENT).unwrap() == "gemini",
        "defaultAgent override",
    );
    settings.set(&DEFAULT_AGENT, "claude".into()).unwrap();
    check(
        settings.get(&DEFAULT_AGENT).unwrap() == "claude",
        "defaultAgent back to default",
    );

    // -- 5. Project settings: repo with .ade.json + legacy .emdash.json ------
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    db.conn()
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO projects (id, name, path) VALUES ('p1', 'smoke', ?1)",
            [repo.to_str().unwrap()],
        )
        .unwrap();
    write_ade_json(&repo, r#"{"preservePatterns": [".env", "team.env"]}"#);
    // The legacy file must exist before the first settings read: the one-shot
    // migration runs on first access and is marked done even without a file.
    std::fs::write(
        repo.join(".emdash.json"),
        r#"{"remote": "upstream", "defaultBranch": "main", "tmux": true}"#,
    )
    .unwrap();

    settings.seed_project_settings("p1", &repo).unwrap();
    let effective = settings.get_project_settings("p1", &repo).unwrap();
    println!(
        "   seeded effective: preservePatterns={:?} baseRemote={:?} defaultBranch={:?} tmux={:?}",
        effective.preserve_patterns,
        effective.base_remote,
        effective.default_branch,
        effective.tmux
    );
    check(
        effective.preserve_patterns.as_deref()
            == Some(&[".env".to_string(), "team.env".to_string()][..]),
        ".ade.json preservePatterns honored after seed",
    );
    check(
        effective.base_remote.as_deref() == Some("upstream"),
        "legacy remote migrated to baseRemote on first read",
    );
    check(
        effective.default_branch == Some(DefaultBranch::Name("upstream/main".into())),
        "bare legacy branch prefixed with the remote name",
    );
    check(effective.tmux == Some(true), "legacy tmux migrated");

    // -- 6. Local override, then clear → falls back to file -------------------
    let mut local = settings.get_project_settings("p1", &repo).unwrap();
    local.preserve_patterns = Some(vec!["local.env".into()]);
    settings
        .update_project_settings("p1", &repo, &local)
        .unwrap();
    let effective = settings.get_project_settings("p1", &repo).unwrap();
    check(
        effective.preserve_patterns.as_deref() == Some(&["local.env".to_string()][..]),
        "local value overrides .ade.json",
    );

    let mut clear = settings.get_project_settings("p1", &repo).unwrap();
    clear.preserve_patterns = None;
    settings
        .update_project_settings("p1", &repo, &clear)
        .unwrap();
    let effective = settings.get_project_settings("p1", &repo).unwrap();
    check(
        effective.preserve_patterns.as_deref()
            == Some(&[".env".to_string(), "team.env".to_string()][..]),
        "clearing local value falls back to .ade.json",
    );

    // -- 7. Share with team: local values → .ade.json, DB cleared ------------
    let mut to_share = settings.get_project_settings("p1", &repo).unwrap();
    to_share.shell_setup = Some("source .envrc".into());
    settings
        .update_project_settings("p1", &repo, &to_share)
        .unwrap();
    settings.share_with_team("p1").unwrap();
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo.join(PROJECT_CONFIG_FILE)).unwrap())
            .unwrap();
    println!("   .ade.json after share: {config}");
    check(
        config["shellSetup"] == serde_json::json!("source .envrc"),
        "shellSetup landed in .ade.json",
    );
    check(
        settings
            .get_project_settings("p1", &repo)
            .unwrap()
            .shell_setup
            .as_deref()
            == Some("source .envrc"),
        "shared value still effective (now sourced from the file)",
    );

    // -- 8. Legacy migration is a no-op on re-run -----------------------------
    settings
        .migrate_legacy_project_settings("p1", &repo)
        .unwrap();
    check(
        settings
            .get_project_settings("p1", &repo)
            .unwrap()
            .base_remote
            .as_deref()
            == Some("upstream"),
        "legacy migration is a no-op on re-run",
    );

    // -- 9. Namespaced KV ------------------------------------------------------
    kv.set("view", "sidebar", &serde_json::json!({"open": true}))
        .unwrap();
    let read: Option<serde_json::Value> = kv.get("view", "sidebar").unwrap();
    check(
        read == Some(serde_json::json!({"open": true})),
        "kv roundtrip",
    );
    kv.delete("view", "sidebar").unwrap();
    check(
        kv.get::<serde_json::Value>("view", "sidebar")
            .unwrap()
            .is_none(),
        "kv delete",
    );

    // -- 10. Projects (E1-03): create local, duplicate, close/open, clone ----
    let bus = Arc::new(ade_core::events::BroadcastEventBus::new(16));
    let git: Arc<dyn ade_core::git::GitOps> = Arc::new(ade_git::CliGit);
    let store = ade_core::projects::DbProjectStore::new(
        db.clone(),
        Arc::new(settings.clone()),
        git.clone(),
        bus.clone(),
    );
    let mut events = bus.subscribe();

    // A real repo with one commit on main.
    let repo2 = tmp.path().join("demo");
    std::fs::create_dir_all(&repo2).unwrap();
    git.as_ref().init(&repo2).unwrap();
    std::fs::write(repo2.join("README.md"), "# demo\n").unwrap();
    Command::new("git")
        .arg("-C")
        .arg(&repo2)
        .args([
            "-c",
            "user.name=Smoke",
            "-c",
            "user.email=s@ade.dev",
            "commit",
            "-am",
            "init",
        ])
        .status()
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(&repo2)
        .args(["branch", "-M", "main"])
        .status()
        .unwrap();

    let project = store.create_local(&repo2, false).unwrap();
    println!(
        "   project: name={} base_ref={} provider={}",
        project.name,
        project.base_ref(),
        project.workspace_provider.as_str()
    );
    check(project.base_ref() == "main", "base ref resolved on create");
    check(
        std::fs::read_to_string(repo2.join(".git/info/exclude"))
            .map(|c| c.lines().any(|l| l.trim() == ".ade/"))
            .unwrap_or(false),
        ".ade/ excluded from git",
    );
    check(
        matches!(
            events.try_recv(),
            Ok(ade_core::events::InternalEvent::ProjectAdded { .. })
        ),
        "project:added emitted",
    );
    check(
        project.repository_workspace_id.is_some(),
        "repository workspace created",
    );

    let dup = store.create_local(&repo2, false).unwrap();
    check(dup.id == project.id, "duplicate add opens existing project");

    store.close(&project.id).unwrap();
    let opened = store.open(&project.id).unwrap();
    check(
        opened.worktrees.iter().any(|w| w.path == project.path),
        "close/open re-detects the main worktree",
    );

    // Clone flow into an overridden (temp) projects dir.
    settings
        .set(
            &LOCAL_PROJECT,
            ade_core::settings::LocalProjectGroup {
                default_projects_directory: tmp
                    .path()
                    .join("repositories")
                    .to_string_lossy()
                    .into_owned(),
                default_worktree_directory: tmp
                    .path()
                    .join("worktrees")
                    .to_string_lossy()
                    .into_owned(),
                write_agent_config_to_git_ignore: true,
            },
        )
        .unwrap();
    let bare = tmp.path().join("bare.git");
    Command::new("git")
        .args(["init", "--bare", bare.to_str().unwrap()])
        .status()
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(&repo2)
        .args(["remote", "add", "origin", bare.to_str().unwrap()])
        .status()
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(&repo2)
        .args(["push", "-u", "origin", "main"])
        .status()
        .unwrap();
    Command::new("git")
        .args([
            "--git-dir",
            bare.to_str().unwrap(),
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ])
        .status()
        .unwrap();
    let clone = store.create_clone(bare.to_str().unwrap()).unwrap();
    check(
        clone.base_ref() == "origin/main",
        "clone base ref resolves via remote HEAD",
    );
    check(
        clone.path.ends_with("repositories/bare"),
        "clone lands in configured projects dir (named after the URL)",
    );

    println!(
        "\n== SMOKE {} ==",
        if failures == 0 { "OK" } else { "FAILED" }
    );
    std::process::exit(if failures == 0 { 0 } else { 1 });
}

fn write_ade_json(repo: &Path, content: &str) {
    std::fs::write(repo.join(PROJECT_CONFIG_FILE), content).unwrap();
}
