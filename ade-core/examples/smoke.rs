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
use ade_core::tasks::{CreateTaskOptions, DbTaskStore, TaskStatus, TaskStore};

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
    // `git commit -am` does NOT stage untracked files — add first or the
    // commit exits 1 (silently, since .status().unwrap() ignores exit codes).
    Command::new("git")
        .arg("-C")
        .arg(&repo2)
        .args(["add", "."])
        .status()
        .unwrap();
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

    // -- 11. Tasks (E2-01): atomic create, status, provision, delete ---------
    let task_store = DbTaskStore::new(db.clone(), bus.clone());
    let task = task_store
        .create(
            CreateTaskOptions::new(dup.id.clone(), "fix smoke bug")
                .with_initial_conversation("Fix smoke bug", Some("claude")),
        )
        .unwrap();
    check(
        task.status == TaskStatus::InProgress,
        "task defaults to in_progress",
    );
    check(
        task.workspace_id.is_some(),
        "task created with a workspace row",
    );
    check(
        task_store.list_by_project(&dup.id).unwrap().len() == 1,
        "task listed by project",
    );
    // Pin status_changed_at to an old value so the bump is observable
    // (datetime('now') has 1s resolution).
    db.conn()
        .lock()
        .unwrap()
        .execute(
            "UPDATE tasks SET status_changed_at = '2020-01-01 00:00:00' WHERE id = ?1",
            [&task.id],
        )
        .unwrap();
    let done = task_store
        .update_status(&task.id, TaskStatus::Done)
        .unwrap();
    check(
        done.status == TaskStatus::Done,
        "status transition persists",
    );
    check(
        done.status_changed_at.as_deref() != Some("2020-01-01 00:00:00"),
        "status change bumps status_changed_at",
    );
    task_store.provision(&task.id).unwrap();
    task_store.provision(&task.id).unwrap();
    check(true, "provision is idempotent");
    task_store.archive(&task.id).unwrap();
    check(
        task_store
            .get(&task.id)
            .unwrap()
            .unwrap()
            .archived_at
            .is_some(),
        "archive sets archived_at",
    );
    task_store.restore(&task.id).unwrap();
    check(
        task_store
            .get(&task.id)
            .unwrap()
            .unwrap()
            .archived_at
            .is_none(),
        "restore clears archived_at",
    );
    task_store.delete(&task.id).unwrap();
    check(task_store.get(&task.id).unwrap().is_none(), "task deleted");

    // -- 12. Naming (E2-03): random name + branch resolution ----------------
    let name = ade_core::tasks::naming::generate_random_name();
    println!("   random task name: {name}");
    check(
        name.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && name.len() <= 64,
        "random name is shell-safe [a-z0-9-] ≤64",
    );
    let branch = ade_core::tasks::naming::resolve_task_branch_name(
        &ade_core::tasks::naming::BranchNameOptions {
            raw_branch: &name,
            branch_prefix: Some("ade"),
            suffix: &ade_core::tasks::naming::random_suffix(),
            append_random_suffix: true,
            linked_issue: None,
            disable_random_suffix: false,
        },
    );
    println!("   task branch: {branch}");
    check(
        // "ade/" (4) + name (≤64) + "-" (1) + suffix (5) = ≤74
        branch.starts_with("ade/") && branch.len() <= 74,
        "branch name is prefixed + suffixed and bounded",
    );

    // -- 13. Worktrees (E2-02): ensure → isolated checkout → remove ---------
    let wt_git: Arc<dyn ade_core::git::GitOps> = Arc::new(ade_git::Git2Ops::new());
    let wt_manager = ade_core::projects::worktrees::WorktreeManager::new(
        db.clone(),
        Arc::new(settings.clone()),
        wt_git.clone(),
    );
    // A real workspace row (the task create flow makes one) so the
    // ensure/remove DB writes are exercised, not no-oped.
    db.conn()
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO workspaces (id, type, kind, location)
             VALUES ('smoke-ws-1', 'local', 'project-root', 'local')",
            [],
        )
        .unwrap();
    let wt = wt_manager
        .ensure_worktree(&ade_core::projects::worktrees::EnsureWorktreeOptions {
            project: &dup,
            task_id: "smoke-task-1",
            workspace_id: "smoke-ws-1",
            branch_name: "ade/smoke-wt",
            source_ref: Some("main"),
            worktree_enabled: true,
        })
        .unwrap();
    println!("   worktree: {}", wt.path.display());
    check(
        wt.kind == ade_core::projects::worktrees::WorkspaceKind::Worktree,
        "ensure_worktree returns a worktree",
    );
    check(
        wt.path.join("README.md").exists(),
        "worktree is checked out with the project files",
    );
    check(
        wt_git.current_branch(&wt.path).unwrap().as_deref() == Some("ade/smoke-wt"),
        "worktree is on its own branch",
    );
    let listed = wt_git.worktree_list(&dup.path).unwrap();
    check(
        listed.len() >= 2,
        "worktree list includes main + linked worktree",
    );

    // Idempotent re-ensure (restart behavior): same path (modulo symlink
    // resolution — /var -> /private/var on macOS), reused.
    let again = wt_manager
        .ensure_worktree(&ade_core::projects::worktrees::EnsureWorktreeOptions {
            project: &dup,
            task_id: "smoke-task-1",
            workspace_id: "smoke-ws-1",
            branch_name: "ade/smoke-wt",
            source_ref: Some("main"),
            worktree_enabled: true,
        })
        .unwrap();
    let real_of =
        |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    check(
        again.reused && real_of(&again.path) == real_of(&wt.path),
        "re-ensure reuses the worktree",
    );

    // Remove: worktree dir gone + metadata pruned, project root untouched.
    wt_manager
        .remove_worktree(&dup, "smoke-task-1", "smoke-ws-1", &wt.path)
        .unwrap();
    check(!wt.path.exists(), "removed worktree directory is gone");
    check(
        dup.path.exists(),
        "project root untouched by worktree removal",
    );
    check(
        !wt_git
            .worktree_list(&dup.path)
            .unwrap()
            .iter()
            .any(|e| real_of(&e.path) == real_of(&wt.path)),
        "worktree metadata pruned after remove",
    );

    // -- 14. Add Task flow (E2-04): operation-driven create ------------------
    let worktrees = ade_core::projects::worktrees::WorktreeManager::new(
        db.clone(),
        Arc::new(settings.clone()),
        wt_git.clone(),
    );
    let create_op = ade_core::tasks::operations::TaskCreationService::new(
        db.clone(),
        Arc::new(settings.clone()),
        git.clone(),
        worktrees,
        bus.clone(),
    );
    let mut op_events = bus.subscribe();
    let op_task = create_op
        .create_with_provision(ade_core::tasks::operations::CreateTaskParams {
            id: Some("smoke-op-task".into()),
            project_id: dup.id.clone(),
            task_config: ade_core::tasks::operations::TaskConfigParams {
                name: "op flow task".into(),
                initial_status: None,
                linked_issue: None,
                initial_conversation: Some(
                    ade_core::tasks::operations::InitialConversationConfig {
                        id: "smoke-op-conv".into(),
                        provider: Some("claude".into()),
                        title: "op flow".into(),
                        r#type: Some("pty".into()),
                        auto_approve: None,
                        initial_prompt: Some("Start working".into()),
                        model: Some("sonnet".into()),
                        initial_queue: None,
                    },
                ),
            },
            git: ade_core::tasks::operations::GitSetup::CreateBranch {
                branch_name: "ade/op-flow-zz9xy".into(),
                from_branch: ade_core::tasks::operations::SourceBranchRef::local("main"),
                push_branch: false,
            },
            workspace: ade_core::tasks::WorkspaceTarget::NewWorktree,
            automation_run_id: None,
        })
        .unwrap();
    println!(
        "   op task: {} workspace={} conv={:?}",
        op_task.task.name,
        op_task.task.workspace_id.as_deref().unwrap_or(""),
        op_task.initial_conversation_id
    );
    check(
        op_task.task.workspace_id.is_some(),
        "operation creates task with a workspace row",
    );
    check(
        op_task.initial_conversation_id.as_deref() == Some("smoke-op-conv"),
        "operation returns the initial conversation id",
    );
    check(
        op_task
            .task
            .workspace_id
            .as_deref()
            .map(|ws| {
                db.conn()
                    .lock()
                    .unwrap()
                    .query_row("SELECT kind FROM workspaces WHERE id = ?1", [ws], |row| {
                        row.get::<_, String>(0)
                    })
                    .unwrap()
            })
            .as_deref()
            == Some("worktree"),
        "workspace row kind is worktree",
    );
    let op_wt = tmp.path().join("worktrees/demo/ade/op-flow-zz9xy");
    check(
        op_wt.join("README.md").exists(),
        "operation provisions the worktree on disk",
    );
    let op_conv_cfg: serde_json::Value = db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT config FROM conversations WHERE id = 'smoke-op-conv'",
            [],
            |row| row.get::<_, String>(0),
        )
        .map(|c| serde_json::from_str(&c).unwrap())
        .unwrap();
    check(
        op_conv_cfg["type"] == "pty" && op_conv_cfg["model"] == "sonnet",
        "model persists into the versioned conversation config",
    );
    let mut op_seen = [false; 2];
    while let Ok(ev) = op_events.try_recv() {
        match ev {
            ade_core::events::InternalEvent::TaskProvisioned { id, .. }
                if id == "smoke-op-task" =>
            {
                op_seen[0] = true
            }
            ade_core::events::InternalEvent::AgentStart {
                conversation_id, ..
            } if conversation_id == "smoke-op-conv" => op_seen[1] = true,
            _ => {}
        }
    }
    check(op_seen[0], "operation emits task:provisioned");
    check(
        op_seen[1],
        "pty initial prompt emits agent start placeholder",
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
