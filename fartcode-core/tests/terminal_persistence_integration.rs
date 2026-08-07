//! E2-07 — Terminal persistence + resume across restarts:
//! session_id persisted on every launch, and boot rehydration resumes a
//! conversation with the provider's resume flags.
//!
//! Uses a fake `amp` binary (amp is not installed on CI hosts) shadowed via
//! a temp PATH dir. amp is argv-strategy with `--resume` + `--session-id`
//! flags — ideal for asserting the resume flag matrix.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fartcode_core::conversations::{
    model::Conversation, ConversationScopeDto, ConversationStore, ConversationTypeDto,
    CreateConversationParams, DbConversationStore,
};
use fartcode_core::db::{Db, SqliteDb};
use fartcode_core::dependencies::HostDependencyStore;
use fartcode_core::events::BroadcastEventBus;
use fartcode_core::projects::DbProjectStore;
use fartcode_core::pty::launcher::{
    AgentLaunchContext, AgentLauncher, NoopRemoteRehydrate, RehydrateTarget, Rehydrator,
};
use fartcode_core::pty::tmux::{make_tmux_session_name, parse_tmux_session_name};
use fartcode_core::tasks::DbTaskStore;
use fartcode_terminal::PortablePtyManager;

/// Restores the process env on drop (PATH mutations).
struct EnvGuard(Vec<(&'static str, Option<String>)>);
impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self(vec![(key, prev)])
    }
}
impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prev) in &self.0 {
            match prev {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Fake amp CLI: records `args=$*` to `$GOOSE_REPORT`, exits 0.
fn write_fake_amp(dir: &Path) -> PathBuf {
    let script = dir.join("amp");
    std::fs::write(
        &script,
        "#!/bin/bash\necho \"args=$*\" > \"$FARTCODE_TASK_PATH/amp-report.txt\"\nexit 0\n",
    )
    .unwrap();
    let mut perms = std::fs::metadata(&script).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    perms.set_mode(0o755);
    std::fs::set_permissions(&script, perms).unwrap();
    script
}

struct Fixture {
    _db: Arc<SqliteDb>,
    conversations: Arc<DbConversationStore>,
    _bin_dir: tempfile::TempDir,
    _worktree: tempfile::TempDir,
    _guard: EnvGuard,
    /// Held for the fixture's LIFETIME (not just construction): parallel
    /// fixtures would otherwise interleave PATH mutations and a dropping
    /// fixture would delete the other's fake binary mid-test.
    _env_lock: std::sync::MutexGuard<'static, ()>,
    report: PathBuf,
    task_id: String,
}

impl Fixture {
    fn new() -> Self {
        let env_lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let bin_dir = tempfile::tempdir().unwrap();
        write_fake_amp(bin_dir.path());
        let path = std::env::var("PATH").unwrap_or_default();
        let guard = EnvGuard::set(
            "PATH",
            &format!("{}:{}", bin_dir.path().to_string_lossy(), path),
        );
        let db = SqliteDb::init_in_memory().unwrap();
        // Minimal project + task rows (FK targets for conversations).
        db.conn()
            .lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO projects (id, name, path) VALUES ('p1', 'demo', '/tmp/demo');
                 INSERT INTO tasks (id, project_id, name, status)
                     VALUES ('task-1', 'p1', 'persist', 'in_progress');",
            )
            .unwrap();
        let bus = Arc::new(BroadcastEventBus::new(16));
        let conversations = Arc::new(DbConversationStore::new(db.clone(), bus));
        let worktree = tempfile::tempdir().unwrap();
        let report = worktree.path().join("amp-report.txt");
        // FARTCODE_TASK_PATH must exist (task_env contract) for the fake's redirect.
        std::fs::create_dir_all(worktree.path()).unwrap();
        Self {
            _db: db,
            conversations,
            _bin_dir: bin_dir,
            _worktree: worktree,
            _guard: guard,
            _env_lock: env_lock,
            report,
            task_id: "task-1".into(),
        }
    }

    fn launcher(&self) -> Arc<AgentLauncher> {
        Arc::new(
            AgentLauncher::new(
                Arc::new(PortablePtyManager),
                Arc::new(HostDependencyStore::new(
                    self._db.clone(),
                    Arc::new(fartcode_core::dependencies::ProcessInstallRunner),
                )),
                Arc::new(BroadcastEventBus::new(16)),
            )
            .with_conversation_store(self.conversations.clone()),
        )
    }

    fn conversation(&self, id: &str, session_id: Option<&str>) -> Conversation {
        let mut c = self
            .conversations
            .create(CreateConversationParams {
                id: Some(id.into()),
                project_id: "p1".into(),
                task_id: Some(self.task_id.clone()),
                scope: Some(ConversationScopeDto::Task),
                provider: Some("amp".into()),
                title: "Persist me".into(),
                auto_approve: None,
                model: None,
                initial_prompt: Some("hi".into()),
                initial_queue: None,
                is_initial_conversation: false,
                r#type: Some(ConversationTypeDto::Pty),
            })
            .unwrap();
        // The fixture drives session_id explicitly (create() sets it to the
        // conversation id for PTY rows — override for the resume scenario).
        if let Some(sid) = session_id {
            c.session_id = Some(sid.into());
        }
        c
    }

    fn fresh_launch_ctx(&self, conversation_id: &str) -> AgentLaunchContext {
        AgentLaunchContext {
            provider_id: "amp".into(),
            conversation_id: conversation_id.into(),
            session_id: None,
            is_resuming: false,
            auto_approve: false,
            model: None,
            initial_prompt: Some("hi".into()),
            worktree: self._worktree.path().to_path_buf(),
            task_env: vec![], // the fake writes via FARTCODE_TASK_PATH (contract)
            hook_env: None,
            respawn_resume: false,
            tmux_enabled: false,
            pty_session_id: None,
        }
    }

    fn report(&self) -> String {
        std::fs::read_to_string(&self.report).unwrap_or_default()
    }
}

/// E2-07 acceptance 1: the resolved session id is persisted on every launch,
/// so a restart can resume.
#[test]
fn session_id_is_persisted_on_launch() {
    let fx = Fixture::new();
    let conv = fx.conversation("conv-persist", None);

    let launcher = fx.launcher();
    launcher.run(&fx.fresh_launch_ctx(&conv.id)).unwrap();

    let stored = fx.conversations.get("conv-persist").unwrap().unwrap();
    assert!(
        stored.session_id.is_some(),
        "session_id must be persisted after launch"
    );
}

/// E2-07 kill-restart acceptance: rehydrating a stored conversation resumes
/// with the provider's resume flags (`--resume <sessionId>` for amp), and a
/// never-spawned one starts fresh (no resume flag).
#[test]
fn kill_restart_rehydrates_with_resume_flags() {
    let fx = Fixture::new();

    // Simulates a prior session that recorded its native id.
    let conv = fx.conversation("conv-resume", Some("amp-native-abc"));

    // "Restart": a fresh launcher over the same DB rehydrates the row.
    let launcher = fx.launcher();
    let target = RehydrateTarget {
        provider_id: "amp".into(),
        project_id: "p1".into(),
        task_id: fx.task_id.clone(),
        task_name: "Persist me".into(),
        worktree: fx._worktree.path().to_path_buf(),
        root_path: fx._worktree.path().to_path_buf(),
        auto_approve: false,
    };
    let outcome = launcher.rehydrate(&conv, &target).unwrap();
    assert_eq!(outcome.spawns, 1, "{outcome:?}");

    let report = fx.report();
    // amp's resume_flag is the space-split "threads continue" — the args
    // are `threads continue <native-session-id>`.
    assert!(
        report.contains("threads continue") && report.contains("amp-native-abc"),
        "resume flag + native session id in args: {report}"
    );

    // A conversation that was never spawned (session_id == its own id, i.e.
    // created but never given a native id) resumes without a resume flag.
    let conv_fresh = fx.conversation("conv-fresh", Some("conv-fresh"));
    std::fs::remove_file(&fx.report).unwrap();
    let _outcome2 = launcher.rehydrate(&conv_fresh, &target).unwrap();
    let report2 = std::fs::read_to_string(&fx.report).unwrap_or_default();
    assert!(
        !report2.contains("threads continue"),
        "fresh conversation must NOT resume: {report2}"
    );
}

/// E2-07 tmux durability: session names round-trip (UTF-8 safe) and foreign
/// names are rejected — the inverse is what boot uses to find sessions.
#[test]
fn tmux_session_name_roundtrip() {
    let name = make_tmux_session_name("conv-123");
    assert!(name.starts_with("fartCode-"));
    assert_eq!(parse_tmux_session_name(&name).as_deref(), Some("conv-123"));
    assert_eq!(parse_tmux_session_name("not-ours"), None);
}

/// E2-07 boot orchestration: `Rehydrator::rehydrate_all` walks
/// projects → tasks → PTY conversations, rebuilds each launch context from
/// the DB (worktree from the workspace row), and resumes with provider
/// flags. Skipped: never-spawned conversations and tasks without a worktree.
#[test]
fn boot_rehydrator_walks_projects_tasks_conversations() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let bin_dir = tempfile::tempdir().unwrap();
    write_fake_amp(bin_dir.path());
    let path = std::env::var("PATH").unwrap_or_default();
    let _guard = EnvGuard::set(
        "PATH",
        &format!("{}:{}", bin_dir.path().to_string_lossy(), path),
    );

    let db = SqliteDb::init_in_memory().unwrap();
    let bus = Arc::new(BroadcastEventBus::new(32));
    let conversations: Arc<DbConversationStore> =
        Arc::new(DbConversationStore::new(db.clone(), bus.clone()));
    let tasks: Arc<DbTaskStore> = Arc::new(DbTaskStore::new(db.clone(), bus.clone()));
    let projects: Arc<DbProjectStore> = Arc::new(DbProjectStore::new(
        db.clone(),
        Arc::new(fartcode_core::settings::DbSettingsStore::new(db.clone())),
        Arc::new(fartcode_git::CliGit),
        bus.clone(),
    ));

    // Project + task + a real worktree dir on disk (the workspace row's path).
    let worktree = tempfile::tempdir().unwrap();
    db.conn()
        .lock()
        .unwrap()
        .execute_batch(&format!(
            "INSERT INTO projects (id, name, path) VALUES ('p1', 'demo', '/tmp/demo');
             INSERT INTO tasks (id, project_id, name, status, workspace_id)
                 VALUES ('task-1', 'p1', 'boot', 'in_progress', 'ws-1');
             INSERT INTO workspaces (id, type, kind, location, path, config)
                 VALUES ('ws-1', 'local', 'worktree', 'local', '{}', '{{}}');",
            worktree.path().display()
        ))
        .unwrap();

    // Resumable conversation: native session id recorded (prior run).
    let conv = conversations
        .create(CreateConversationParams {
            id: Some("conv-boot".into()),
            project_id: "p1".into(),
            task_id: Some("task-1".into()),
            scope: Some(ConversationScopeDto::Task),
            provider: Some("amp".into()),
            title: "Boot me".into(),
            auto_approve: None,
            model: None,
            initial_prompt: Some("hi".into()),
            initial_queue: None,
            is_initial_conversation: false,
            r#type: Some(ConversationTypeDto::Pty),
        })
        .unwrap();
    // A prior run recorded a NATIVE session id (resumable).
    conversations
        .set_session_id(&conv.id, "amp-native-boot")
        .unwrap();

    // A row with a NULL session id (pre-E2-05 rows / ACP-style) is skipped.
    conversations
        .create(CreateConversationParams {
            id: Some("conv-fresh-boot".into()),
            project_id: "p1".into(),
            task_id: Some("task-1".into()),
            scope: Some(ConversationScopeDto::Task),
            provider: Some("amp".into()),
            title: "Never spawned".into(),
            auto_approve: None,
            model: None,
            initial_prompt: None,
            initial_queue: None,
            is_initial_conversation: false,
            r#type: Some(ConversationTypeDto::Pty),
        })
        .unwrap();
    db.conn()
        .lock()
        .unwrap()
        .execute(
            "UPDATE conversations SET session_id = NULL WHERE id = 'conv-fresh-boot'",
            [],
        )
        .unwrap();

    let rehydrator = Rehydrator::new(
        Arc::new(PortablePtyManager),
        Arc::new(HostDependencyStore::new(
            db.clone(),
            Arc::new(fartcode_core::dependencies::ProcessInstallRunner),
        )),
        Arc::new(BroadcastEventBus::new(16)),
        conversations.clone(),
        tasks.clone(),
        projects.clone(),
        db.clone(),
        false,
        Arc::new(NoopRemoteRehydrate),
        None,
    );

    let summary = rehydrator.rehydrate_all().unwrap();
    assert_eq!(summary.resumed, 1, "{summary:?}");
    assert_eq!(
        summary.skipped, 1,
        "NULL-session conversation skipped: {summary:?}"
    );
    assert_eq!(summary.failed, 0, "{summary:?}");

    // The resumed conversation's launch carried the resume flag + native id.
    let report = std::fs::read_to_string(worktree.path().join("amp-report.txt")).unwrap();
    assert!(
        report.contains("threads continue") && report.contains("amp-native-boot"),
        "boot resume flags in args: {report}"
    );
    // Session id was re-persisted by the launch (with_conversation_store).
    let stored = conversations.get("conv-boot").unwrap().unwrap();
    assert_eq!(stored.session_id.as_deref(), Some("amp-native-boot"));
}
