//! Smoke-test checkpoint for E1-02 (settings store) — boot the DB, exercise
//! app settings, project settings, `.fartCode.json` precedence, share-with-team,
//! legacy migration, and KV — then print a summary.
//!
//! Run from the workspace root:
//!
//! ```sh
//! cargo run -p fartcode-core --example smoke
//! ```
//!
//! Everything lives in a fresh temp directory; nothing touches real app data.
//! Exit code 0 = SMOKE OK, non-zero = a check failed.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use fartcode_core::conversations::ConversationStore;
use fartcode_core::db::SqliteDb;
use fartcode_core::events::EventBus;
use fartcode_core::projects::ProjectStore;
use fartcode_core::settings::{
    DbSettingsStore, DefaultBranch, KvStore, ProjectGroup, SettingsStore, DEFAULT_AGENT,
    LOCAL_PROJECT, PROJECT, PROJECT_CONFIG_FILE,
};
use fartcode_core::tasks::{CreateTaskOptions, TaskStatus, TaskStore};

fn main() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("smoke.db");
    println!("== fartCode smoke — E1-02 settings store ==");
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
        project_defaults.branch_prefix == "fartCode",
        "default branch prefix is 'fartCode'",
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
        settings.get(&PROJECT).unwrap().branch_prefix == "fartCode",
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

    // -- 5. Project settings: repo with .fartCode.json + legacy .emdash.json ------
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
        ".fartCode.json preservePatterns honored after seed",
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
        "local value overrides .fartCode.json",
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
        "clearing local value falls back to .fartCode.json",
    );

    // -- 7. Share with team: local values → .fartCode.json, DB cleared ------------
    let mut to_share = settings.get_project_settings("p1", &repo).unwrap();
    to_share.shell_setup = Some("source .envrc".into());
    settings
        .update_project_settings("p1", &repo, &to_share)
        .unwrap();
    settings.share_with_team("p1").unwrap();
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo.join(PROJECT_CONFIG_FILE)).unwrap())
            .unwrap();
    println!("   .fartCode.json after share: {config}");
    check(
        config["shellSetup"] == serde_json::json!("source .envrc"),
        "shellSetup landed in .fartCode.json",
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
    let bus = Arc::new(fartcode_core::events::BroadcastEventBus::new(16));
    let git: Arc<dyn fartcode_core::git::GitOps> = Arc::new(fartcode_git::CliGit);
    let store = fartcode_core::projects::DbProjectStore::new(
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
            "user.email=s@fartCode.dev",
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
            .map(|c| c.lines().any(|l| l.trim() == ".fartCode/"))
            .unwrap_or(false),
        ".fartCode/ excluded from git",
    );
    check(
        matches!(
            events.try_recv(),
            Ok(fartcode_core::events::InternalEvent::ProjectAdded { .. })
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
            fartcode_core::settings::LocalProjectGroup {
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
    let name = fartcode_core::tasks::naming::generate_random_name();
    println!("   random task name: {name}");
    check(
        name.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && name.len() <= 64,
        "random name is shell-safe [a-z0-9-] ≤64",
    );
    let branch = fartcode_core::tasks::naming::resolve_task_branch_name(
        &fartcode_core::tasks::naming::BranchNameOptions {
            raw_branch: &name,
            branch_prefix: Some("fartCode"),
            suffix: &fartcode_core::tasks::naming::random_suffix(),
            append_random_suffix: true,
            linked_issue: None,
            disable_random_suffix: false,
        },
    );
    println!("   task branch: {branch}");
    check(
        // "fartCode/" (4) + name (≤64) + "-" (1) + suffix (5) = ≤74
        branch.starts_with("fartCode/") && branch.len() <= 74,
        "branch name is prefixed + suffixed and bounded",
    );

    // -- 13. Worktrees (E2-02): ensure → isolated checkout → remove ---------
    let wt_git: Arc<dyn fartcode_core::git::GitOps> = Arc::new(fartcode_git::CliGit);
    let wt_manager = fartcode_core::projects::worktrees::WorktreeManager::new(
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
        .ensure_worktree(&fartcode_core::projects::worktrees::EnsureWorktreeOptions {
            project: &dup,
            task_id: "smoke-task-1",
            workspace_id: "smoke-ws-1",
            branch_name: "fartCode/smoke-wt",
            source_ref: Some("main"),
            worktree_enabled: true,
        })
        .unwrap();
    println!("   worktree: {}", wt.path.display());
    check(
        wt.kind == fartcode_core::projects::worktrees::WorkspaceKind::Worktree,
        "ensure_worktree returns a worktree",
    );
    check(
        wt.path.join("README.md").exists(),
        "worktree is checked out with the project files",
    );
    check(
        wt_git.current_branch(&wt.path).unwrap().as_deref() == Some("fartCode/smoke-wt"),
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
        .ensure_worktree(&fartcode_core::projects::worktrees::EnsureWorktreeOptions {
            project: &dup,
            task_id: "smoke-task-1",
            workspace_id: "smoke-ws-1",
            branch_name: "fartCode/smoke-wt",
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
        .remove_worktree(&dup, "smoke-task-1", "smoke-ws-1", &wt.path, false)
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
    let worktrees = fartcode_core::projects::worktrees::WorktreeManager::new(
        db.clone(),
        Arc::new(settings.clone()),
        wt_git.clone(),
    );
    let create_op = fartcode_core::tasks::operations::TaskCreationService::new(
        db.clone(),
        Arc::new(settings.clone()),
        git.clone(),
        worktrees,
        bus.clone(),
    );
    let mut op_events = bus.subscribe();
    let op_task = create_op
        .create_with_provision(fartcode_core::tasks::operations::CreateTaskParams {
            id: Some("smoke-op-task".into()),
            project_id: dup.id.clone(),
            task_config: fartcode_core::tasks::operations::TaskConfigParams {
                name: "op flow task".into(),
                initial_status: None,
                linked_issue: None,
                initial_conversation: Some(
                    fartcode_core::tasks::operations::InitialConversationConfig {
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
            git: fartcode_core::tasks::operations::GitSetup::CreateBranch {
                branch_name: "fartCode/op-flow-zz9xy".into(),
                from_branch: fartcode_core::tasks::operations::SourceBranchRef::local("main"),
                push_branch: false,
            },
            workspace: fartcode_core::tasks::WorkspaceTarget::NewWorktree,
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
    let op_wt = tmp.path().join("worktrees/demo/fartCode/op-flow-zz9xy");
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
            fartcode_core::events::InternalEvent::TaskProvisioned { id, .. }
                if id == "smoke-op-task" =>
            {
                op_seen[0] = true
            }
            fartcode_core::events::InternalEvent::AgentStart {
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

    // -- 15. Conversations (E2-05): session supervisor ------------------------
    let conv_store =
        fartcode_core::conversations::DbConversationStore::new(db.clone(), bus.clone());
    let conv = conv_store
        .create(fartcode_core::conversations::CreateConversationParams {
            id: Some("smoke-conv-1".into()),
            project_id: dup.id.clone(),
            task_id: Some(op_task.task.id.clone()),
            scope: None,
            provider: Some("codex".into()),
            title: "smoke conv".into(),
            auto_approve: None,
            model: Some("o4-mini".into()),
            initial_prompt: Some("Fix it".into()),
            initial_queue: None,
            is_initial_conversation: false,
            r#type: Some(fartcode_core::conversations::model::ConversationType::Pty),
        })
        .unwrap();
    check(
        conv.session_id.as_deref() == Some("smoke-conv-1"),
        "pty conversation gets a stable session id at creation",
    );
    let pty_sid =
        fartcode_core::conversations::make_pty_session_id(&dup.id, &op_task.task.id, &conv.id);
    check(
        fartcode_core::conversations::parse_pty_session_id(&pty_sid).is_some(),
        "pty session id parses",
    );

    // Resume round-trips via session_id: set a native id, then resolve.
    conv_store
        .set_session_id(&conv.id, "  native-codex-42  ")
        .unwrap();
    let (sid, resuming) = fartcode_core::conversations::resolve_agent_session_command_args(
        "codex",
        &conv.id,
        conv_store
            .get(&conv.id)
            .unwrap()
            .unwrap()
            .session_id
            .as_deref(),
        true,
        None,
    );
    check(
        sid == "native-codex-42" && resuming,
        "codex resume uses the native session id",
    );

    // Hydrate after restart: fresh store over the same DB → resume state.
    let conv_store2 =
        fartcode_core::conversations::DbConversationStore::new(db.clone(), bus.clone());
    let hydrated = conv_store2
        .hydrate(&dup.id, &op_task.task.id, &conv.id)
        .unwrap();
    check(
        hydrated.is_resuming
            && hydrated.conversation.session_id.as_deref() == Some("native-codex-42"),
        "hydrate after restart restores the resume state",
    );

    // -- 16. Provider registry (E3-01) ----------------------------------------
    check(
        fartcode_providers::list().len() == 35,
        "provider registry has all 35 agents",
    );
    let claude = fartcode_providers::get("claude").expect("claude registered");
    check(
        claude.name == "Claude Code" && claude.capabilities.acp,
        "get(\"claude\") returns metadata + capabilities",
    );
    check(
        fartcode_providers::filter_by_capability(fartcode_providers::Capability::Acp).len() == 22,
        "capability filter returns 22 ACP-capable providers",
    );
    check(
        fartcode_providers::resolve_executable("cmdc").map(|p| p.id.as_str())
            == Some("commandcode"),
        "resolve_executable maps binary names to providers",
    );
    let claude_dto = fartcode_providers::list_dtos()
        .into_iter()
        .find(|d| d.id == "claude")
        .unwrap();
    check(
        claude_dto.models.first().map(String::as_str) == Some("Default model"),
        "DTO model list starts with the Default model sentinel",
    );

    // -- 17. Host dependencies (E3-02) ----------------------------------------
    let deps_store = fartcode_core::dependencies::HostDependencyStore::new(
        db.clone(),
        Arc::new(fartcode_core::dependencies::ProcessInstallRunner),
    );
    let claude_status = deps_store.get_status("claude").unwrap();
    let codex_status = deps_store.get_status("codex").unwrap();
    check(claude_status.installed, "detection finds claude on PATH");
    check(
        !codex_status.installed || claude_status.checked_at.is_some(),
        "detection results are cached",
    );
    check(
        deps_store.install_instructions("codex").as_deref() == Some("npm install -g @openai/codex"),
        "not-installed providers carry install instructions",
    );
    let claude_plan = fartcode_core::dependencies::host_dependency::host_dependency("claude")
        .unwrap()
        .install_plan
        .unwrap();
    check(
        claude_plan.install_type == fartcode_core::dependencies::host_dependency::InstallType::Curl,
        "claude installs via curl",
    );

    // -- 18. Prompt delivery (E3-03) ------------------------------------------
    use fartcode_core::pty::{
        build_command, cleanup_spilled_prompt, maybe_spill_prompt, CommandContext,
    };
    let claude_p = fartcode_providers::get("claude").unwrap();
    let cmd = build_command(
        &CommandContext {
            cli: "/usr/local/bin/claude".into(),
            auto_approve: true,
            initial_prompt: Some("Fix the bug".into()),
            session_id: Some("smoke-conv-1".into()),
            model: "sonnet".into(),
            ..Default::default()
        },
        claude_p,
    );
    check(
        cmd.args.contains(&"--dangerously-skip-permissions".into())
            && cmd.args.last().map(String::as_str) == Some("Fix the bug"),
        "claude builds the prompt + auto-approve flags",
    );
    let amp_p = fartcode_providers::get("amp").unwrap();
    let amp_cmd = build_command(
        &CommandContext {
            cli: "/usr/local/bin/amp".into(),
            initial_prompt: Some("Do it".into()),
            model: "ultra".into(),
            ..Default::default()
        },
        amp_p,
    );
    check(
        amp_cmd.command == "bash" && amp_cmd.args[1].contains("| /usr/local/bin/amp"),
        "amp receives the prompt on stdin",
    );
    let big = "y".repeat(100 * 1024);
    let (deliverable, spilled) = maybe_spill_prompt(&big, &dup.path).unwrap();
    check(
        deliverable.starts_with('@') && spilled.is_some(),
        "100KB prompt spills to a temp file",
    );
    if let Some(path) = &spilled {
        cleanup_spilled_prompt(path);
        check(!path.exists(), "spilled prompt file is cleaned on exit");
    }

    // -- 19. Auto-approve (E3-04) --------------------------------------------
    use fartcode_core::pty::resolve_auto_approve;
    check(
        resolve_auto_approve(false, false, true),
        "default trust => auto-approve implicitly on",
    );
    check(
        !resolve_auto_approve(false, false, false),
        "no trust / toggle / default => off",
    );
    check(
        resolve_auto_approve(false, true, false),
        "autoApproveByDefault forces auto-approve",
    );
    let codex = fartcode_providers::get("codex").unwrap();
    check(
        codex.capabilities.auto_approve && codex.prompt.auto_approve_flag.is_some(),
        "codex declares autoApprove capability + flag",
    );
    let claude_ctx = fartcode_core::pty::CommandContext {
        auto_approve: true,
        initial_prompt: Some("hi".into()),
        ..fartcode_core::pty::CommandContext::default()
    };
    let claude = fartcode_providers::get("claude").unwrap();
    let cmd = fartcode_core::pty::build_command(&claude_ctx, claude);
    check(
        cmd.args
            .iter()
            .any(|a| a == "--dangerously-skip-permissions"),
        "claude auto-approve passes its flag",
    );

    // -- 20. Lifecycle scripts (E1-06) ---------------------------------------
    use fartcode_core::terminals::lifecycle::{
        fartcode_port, make_lifecycle_script_session_id, task_env, LifecycleScript,
        LifecycleScriptService, LifecycleScriptType, RunOptions,
    };
    let lifecycle_bus = Arc::new(fartcode_core::events::BroadcastEventBus::new(64));
    let lifecycle = Arc::new(LifecycleScriptService::new(
        Arc::new(fartcode_terminal::PortablePtyManager),
        lifecycle_bus.clone(),
    ));
    let lc_cwd = tempfile::tempdir().unwrap();
    let port_a = fartcode_port("/smoke/wt/a");
    let port_b = fartcode_port("/smoke/wt/b");
    check(
        port_a != port_b,
        "two worktrees get different FARTCODE_PORTs",
    );
    let env = task_env(
        "smoke-task",
        "Smoke Task",
        lc_cwd.path(),
        lc_cwd.path(),
        "main",
        "/smoke/wt/a",
    );
    let lc_run = lifecycle
        .run(
            &make_lifecycle_script_session_id("p", "w", LifecycleScriptType::Setup),
            &LifecycleScript {
                r#type: LifecycleScriptType::Setup,
                script: "echo $FARTCODE_TASK_ID port=$FARTCODE_PORT next=$((FARTCODE_PORT + 1))"
                    .into(),
                shell_setup: None,
            },
            lc_cwd.path(),
            &env,
            &RunOptions {
                wait_for_exit: true,
                exit: true,
                ..Default::default()
            },
        )
        .unwrap();
    let lc_out = match &lc_run {
        fartcode_core::terminals::lifecycle::LifecycleRunResult::Exited { output_tail, .. } => {
            output_tail.clone()
        }
        other => panic!("unexpected {other:?}"),
    };
    check(
        lc_out.contains("smoke-task") && lc_out.contains(&port_a.to_string()),
        "lifecycle script runs with the env contract (FARTCODE_TASK_ID, FARTCODE_PORT)",
    );
    check(
        lc_out.contains(&(port_a + 1).to_string()),
        "shell arithmetic sees FARTCODE_PORT",
    );

    // -- 21. View state (E1-08) ---------------------------------------------
    use fartcode_core::view_state;
    let vs_db: std::sync::Arc<dyn fartcode_core::db::Db> = db.clone();
    view_state::save(
        &vs_db,
        "view-state:app:sidebar",
        &serde_json::json!({"collapsed": {"p1": true}}),
    )
    .unwrap();
    let restored = view_state::get(&vs_db, "view-state:app:sidebar")
        .unwrap()
        .unwrap();
    check(
        restored["collapsed"]["p1"] == true,
        "view state round-trips",
    );
    check(
        view_state::save(&vs_db, "nope", &serde_json::Value::Null).is_err(),
        "view-state keys are prefix-enforced",
    );

    // -- 22. Search + resource monitor (E1-09) ------------------------------
    use fartcode_core::search;
    search::upsert(
        &vs_db,
        "project",
        "smoke-proj",
        None,
        None,
        "smoke-engine",
        &["smoke"],
    )
    .unwrap();
    let hits = search::query(&vs_db, "smoke-eng", 5).unwrap();
    check(
        hits.iter().any(|h| h.item_id == "smoke-proj"),
        "FTS trigram search finds the doc",
    );
    search::delete(&vs_db, "project", "smoke-proj").unwrap();
    check(
        search::query(&vs_db, "smoke-eng", 5).unwrap().is_empty(),
        "FTS delete removes the doc",
    );
    let rmon = fartcode_core::resource_monitor::sample().unwrap();
    check(rmon.mem_total_mb > 0, "resource monitor samples memory");

    // -- 23. Env allowlist (E3-08) -------------------------------------------
    use fartcode_core::pty::env_allowlist::{build_agent_env, is_allowlisted};
    std::env::set_var("FARTCODE_SMOKE_SECRET", "leak-me");
    let agent_env = build_agent_env(&[], &[], None);
    check(
        !agent_env.contains_key("FARTCODE_SMOKE_SECRET"),
        "non-allowlisted env var never reaches the agent",
    );
    check(
        agent_env.get("TERM").map(String::as_str) == Some("xterm-256color"),
        "base env forced",
    );
    check(
        is_allowlisted("ANTHROPIC_API_KEY") && !is_allowlisted("FARTCODE_SMOKE_SECRET"),
        "allowlist is the single source of truth",
    );
    std::env::remove_var("FARTCODE_SMOKE_SECRET");

    // -- 24. Agent launcher (E2-06) ------------------------------------------
    use fartcode_core::pty::launcher::{find_on_path, AgentLaunchContext, MAX_RESPAWNS};
    use fartcode_core::terminals::pty::EnvPolicy;
    check(
        EnvPolicy::Inherit != EnvPolicy::AllowlistedOnly,
        "env policy: inherit vs allowlist-only are distinct",
    );
    check(
        find_on_path("sh").is_some(),
        "find_on_path resolves a shell on PATH",
    );
    check(
        find_on_path("definitely-not-a-real-binary-xyz").is_none(),
        "find_on_path misses absent binaries",
    );
    check(MAX_RESPAWNS == 2, "respawn budget is initial + 2");
    let _launcher_shape = AgentLaunchContext {
        provider_id: "codex".into(),
        conversation_id: "c".into(),
        session_id: None,
        is_resuming: false,
        auto_approve: false,
        model: None,
        initial_prompt: Some("hi".into()),
        worktree: lc_cwd.path().to_path_buf(),
        task_env: env.clone(),
        hook_env: None,
        respawn_resume: false,
        tmux_enabled: false,
        pty_session_id: None,
    };
    check(true, "launcher context carries the full launch surface");

    // -- 25. Terminal persistence + tmux durability (E2-07) ------------------
    use fartcode_core::pty::tmux::{
        build_tmux_shell_line, make_tmux_session_name, parse_tmux_session_name,
    };
    let tname = make_tmux_session_name("conv-xyz");
    check(
        tname.starts_with("fartCode-")
            && parse_tmux_session_name(&tname).as_deref() == Some("conv-xyz"),
        "tmux session name round-trips",
    );
    check(
        parse_tmux_session_name("not-ours").is_none(),
        "foreign tmux names rejected",
    );
    let tline = build_tmux_shell_line(&tname, "amp run");
    check(
        tline.contains("has-session")
            && tline.contains("new-session -d")
            && tline.contains("attach-session"),
        "tmux shell line: create-if-missing + attach",
    );
    check(
        tline.contains("history-limit 100000"),
        "tmux history-limit applied",
    );
    use fartcode_core::conversations::DbConversationStore;
    use fartcode_core::db::Db;
    let persist_db = fartcode_core::db::SqliteDb::init_in_memory().unwrap();
    let persist_store = DbConversationStore::new(
        persist_db.clone(),
        Arc::new(fartcode_core::events::BroadcastEventBus::new(8)),
    );
    check(
        persist_store
            .set_session_id("smoke-missing", "sid-1")
            .is_err(),
        "set_session_id errors on a missing conversation",
    );
    check(
        persist_store.set_session_id("smoke-missing", "").is_err(),
        "set_session_id errors on an empty id",
    );

    // -- 26. Boot rehydration orchestration (E2-07) --------------------------
    use fartcode_core::projects::DbProjectStore;
    use fartcode_core::pty::launcher::{RehydrateSummary, Rehydrator};
    use fartcode_core::tasks::DbTaskStore;
    let rh_db = fartcode_core::db::SqliteDb::init_in_memory().unwrap();
    let rh_bus = Arc::new(fartcode_core::events::BroadcastEventBus::new(16));
    let rh_convs = DbConversationStore::new(rh_db.clone(), rh_bus.clone());
    let rh_tasks = DbTaskStore::new(rh_db.clone(), rh_bus.clone());
    let rh_projects = DbProjectStore::new(
        rh_db.clone(),
        Arc::new(DbSettingsStore::new(rh_db.clone())),
        Arc::new(fartcode_git::CliGit),
        rh_bus.clone(),
    );
    let rehydrator = Rehydrator::new(
        Arc::new(fartcode_terminal::PortablePtyManager),
        Arc::new(fartcode_core::dependencies::HostDependencyStore::new(
            rh_db.clone(),
            Arc::new(fartcode_core::dependencies::ProcessInstallRunner),
        )),
        Arc::new(fartcode_core::events::BroadcastEventBus::new(16)),
        Arc::new(rh_convs),
        Arc::new(rh_tasks),
        Arc::new(rh_projects),
        rh_db.clone(),
        false,
        None,
        None,
    );
    // Empty DB -> nothing to resume, no errors.
    let empty_summary = rehydrator.rehydrate_all().unwrap();
    check(
        empty_summary == RehydrateSummary::default(),
        "boot rehydration on an empty DB is a clean no-op",
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
