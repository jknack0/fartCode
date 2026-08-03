//! Agent launcher (E2-06): starts a provider CLI inside a PTY in the task
//! worktree with the allowlisted env, prompt delivery (argv/stdin/keystroke),
//! large-prompt spill, and the respawn policy.
//!
//! Phase-0 execution model: each launch runs in a dedicated PTY via
//! `ade_core::terminals::pty::PtyManager`; the interactive-terminal wiring
//! (frontend xterm attach) lands with the E2-series terminal UI. The launch
//! semantics, env, and respawn policy match the reference `local-pty.ts`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ade_providers::{PromptStrategy, ProviderDescriptor};

use crate::conversations::ConversationTypeDto as ConversationType;
use crate::conversations::{
    model::Conversation, resolve_agent_session_command_args, ConversationStore,
};
use crate::db::Db;
use crate::dependencies::HostDependencyStore;
use crate::events::{BroadcastEventBus, EventBus, InternalEvent};
use crate::projects::ProjectStore;
use crate::tasks::TaskStore;
use crate::terminals::lifecycle::task_env;
use crate::terminals::pty::{EnvPolicy, PtyHandle, PtyManager, PtySize};
use crate::Error;
use rusqlite::OptionalExtension;

use super::env_allowlist;
use super::{
    build_command_with_spill, cleanup_spilled_prompt, CommandContext, PromptInjection,
    PromptInjector,
};

/// Reference `MAX_RESPAWNS` + respawn delay.
pub const MAX_RESPAWNS: usize = 2;
pub const RESPAWN_DELAY_MS: u64 = 500;
/// Agent sessions run until the CLI exits — the wait is effectively
/// open-ended (interactive close/terminate arrives with the E2 terminal UI).
pub const AGENT_WAIT_TIMEOUT: Duration = Duration::from_secs(24 * 3600);

/// Everything the launcher needs for one agent session.
#[derive(Debug, Clone)]
pub struct AgentLaunchContext {
    pub provider_id: String,
    pub conversation_id: String,
    /// `conversations.session_id` (provider session id for resume-capable
    /// providers, `None` for stateless ones).
    pub session_id: Option<String>,
    pub is_resuming: bool,
    pub auto_approve: bool,
    pub model: Option<String>,
    pub initial_prompt: Option<String>,
    /// Task worktree — the agent's cwd.
    pub worktree: PathBuf,
    /// `ADE_*` task env (E1-06 contract) — overlays the allowlist.
    pub task_env: Vec<(String, String)>,
    /// (port, pty_id, nonce, token) when the E3-05 hook server is up.
    pub hook_env: Option<(String, String, String, String)>,
    /// Supervisor decision: allow respawn on exit (reference `respawnResume`).
    pub respawn_resume: bool,
    /// Disables respawn (reference: "disabled when tmux enabled").
    pub tmux_enabled: bool,
}

impl AgentLaunchContext {
    /// Convenience for tests: a context with the task env pre-filled from
    /// the E1-06 contract.
    pub fn with_task_env(
        provider_id: &str,
        conversation_id: &str,
        task_id: &str,
        task_name: &str,
        worktree: PathBuf,
        root_path: &Path,
    ) -> Self {
        let task_env = task_env(
            task_id,
            task_name,
            &worktree,
            root_path,
            "main",
            &worktree.to_string_lossy(),
        );
        Self {
            provider_id: provider_id.into(),
            conversation_id: conversation_id.into(),
            session_id: None,
            is_resuming: false,
            auto_approve: false,
            model: None,
            initial_prompt: None,
            worktree,
            task_env,
            hook_env: None,
            respawn_resume: false,
            tmux_enabled: false,
        }
    }
}

/// Outcome of a launch (after the respawn budget is exhausted or the agent
/// exits cleanly).
#[derive(Debug, Clone)]
pub struct AgentLaunchOutcome {
    pub spawns: usize,
    pub last_exit_code: Option<u32>,
    pub last_signal: Option<String>,
}

/// Launches provider CLIs in task worktrees (E2-06).
pub struct AgentLauncher {
    pty: Arc<dyn PtyManager>,
    deps: Arc<HostDependencyStore>,
    events: Arc<BroadcastEventBus>,
    /// When set, every launch persists the resolved session id (E2-07:
    /// "persist conversations.session_id on every start/resume" — the
    /// restart-survival contract).
    conversations: Option<Arc<dyn ConversationStore>>,
}

impl AgentLauncher {
    pub fn new(
        pty: Arc<dyn PtyManager>,
        deps: Arc<HostDependencyStore>,
        events: Arc<BroadcastEventBus>,
    ) -> Self {
        Self {
            pty,
            deps,
            events,
            conversations: None,
        }
    }

    /// Opts into session-id persistence (E2-07).
    pub fn with_conversation_store(mut self, store: Arc<dyn ConversationStore>) -> Self {
        self.conversations = Some(store);
        self
    }

    /// Boot rehydration (E2-07, reference `hydrateConversation` PTY path):
    /// rebuilds the launch context for a stored conversation and runs it.
    /// `is_resuming = session_id.is_some()` — any previously-spawned
    /// conversation resumes (the reference rule); `initial_prompt` is NOT
    /// re-sent on resume (the reference passes it only on first spawn).
    pub fn rehydrate(
        &self,
        conversation: &Conversation,
        worktree: &RehydrateTarget,
    ) -> Result<AgentLaunchOutcome, Error> {
        let is_resuming = conversation.session_id.is_some();
        let ctx = AgentLaunchContext {
            provider_id: worktree.provider_id.clone(),
            conversation_id: conversation.id.clone(),
            session_id: conversation.session_id.clone(),
            is_resuming,
            // Conversation-level toggle wins; the app-provided value is the
            // trust-state fallback (E3-04 resolve_auto_approve semantics).
            auto_approve: conversation
                .config
                .auto_approve
                .unwrap_or(worktree.auto_approve),
            model: conversation.config.model.clone(),
            initial_prompt: None, // resume never re-sends the prompt
            worktree: worktree.worktree.clone(),
            task_env: task_env(
                &worktree.task_id,
                &worktree.task_name,
                &worktree.worktree,
                &worktree.root_path,
                "main",
                &worktree.worktree.to_string_lossy(),
            ),
            hook_env: None,
            respawn_resume: false,
            tmux_enabled: false,
        };
        self.run(&ctx)
    }

    /// Resolves the provider executable: cached host-dependency detection
    /// first, then the registry's binary names on PATH.
    pub fn resolve_binary(&self, provider_id: &str) -> Result<PathBuf, Error> {
        if let Ok(status) = self.deps.get_status(provider_id) {
            if status.installed {
                // `path` IS the detected executable (E3-02 detection).
                if let Some(path) = status.path {
                    return Ok(PathBuf::from(path));
                }
            }
        }
        // Fallback: the registry's binaries on PATH.
        if let Some(provider) = ade_providers::get(provider_id) {
            for bin in &provider.binaries {
                if let Some(found) = find_on_path(bin) {
                    return Ok(found);
                }
            }
        }
        Err(Error::AgentNotFound(provider_id.into()))
    }

    /// Runs the full launch flow (blocking; spawn on a thread from the app
    /// layer): resolve binary → build command + env → spawn → deliver the
    /// prompt (argv/stdin/keystroke) → wait → respawn per policy → events.
    pub fn run(&self, ctx: &AgentLaunchContext) -> Result<AgentLaunchOutcome, Error> {
        let provider = ade_providers::get(&ctx.provider_id)
            .ok_or_else(|| Error::AgentNotFound(ctx.provider_id.clone()))?;
        let binary = self.resolve_binary(&ctx.provider_id)?;

        // Session args (E2-05): native session id when the provider is
        // resume-capable, the conversation id otherwise.
        let (provider_session_id, is_resuming) = resolve_agent_session_command_args(
            &ctx.provider_id,
            &ctx.conversation_id,
            ctx.session_id.as_deref(),
            ctx.is_resuming,
            None,
        );

        // E2-07: persist the session id on EVERY start/resume (the reference
        // setSessionId — single guarded UPDATE). Non-fatal: a vanished
        // conversation logs and continues the launch.
        if let Some(store) = &self.conversations {
            match store.set_session_id(&ctx.conversation_id, &provider_session_id) {
                Ok(()) => {}
                Err(Error::ConversationNotFound(_)) | Err(Error::EmptySessionId) => {
                    tracing::warn!(
                        conversation = %ctx.conversation_id,
                        "could not persist session id (non-fatal)"
                    );
                }
                Err(e) => return Err(e),
            }
        }

        // Provider env vars: the registry names them (E3-01, allowlisted by
        // construction); the launcher pulls their VALUES from the process
        // env — a missing var is skipped, never invented.
        let provider_env: Vec<(String, String)> = provider
            .env_vars
            .iter()
            .filter_map(|key| std::env::var(key).ok().map(|value| (key.clone(), value)))
            .collect();
        let mut env = env_allowlist::build_agent_env(
            &provider_env,
            &ctx.task_env,
            ctx.hook_env
                .as_ref()
                .map(|(p, i, n, t)| (p.as_str(), i.as_str(), n.as_str(), t.as_str())),
        );

        let mut spawns = 0usize;
        let mut outcome = AgentLaunchOutcome {
            spawns: 0,
            last_exit_code: None,
            last_signal: None,
        };

        loop {
            spawns += 1;
            self.events.send(InternalEvent::AgentRunStarted {
                conversation_id: ctx.conversation_id.clone(),
                provider: ctx.provider_id.clone(),
            });

            let (command, this_spill) = build_command_with_spill(
                &CommandContext {
                    cli: binary.to_string_lossy().into_owned(),
                    extra_args: Vec::new(),
                    initial_prompt: ctx.initial_prompt.clone(),
                    session_id: Some(provider_session_id.clone()),
                    provider_session_id: Some(provider_session_id.clone()),
                    is_resuming,
                    auto_approve: ctx.auto_approve,
                    model: ctx.model.clone().unwrap_or_default(),
                },
                provider,
                &ctx.worktree,
            )
            .map_err(|e| Error::Pty(e.to_string()))?;

            // The allowlisted env is the launch env; command.env (from
            // build_command) is merged over it (provider-specific overrides).
            for (k, v) in &command.env {
                env.insert(k.clone(), v.clone());
            }
            // Keep THIS spawn's spill for cleanup when it exits (each
            // respawn writes a fresh uuid file — clean them individually).
            let this_spilled = this_spill;
            let _spill_guard = SpillGuard(&this_spilled);

            let mut handle = self.pty.spawn(
                &command.command,
                &command.args,
                &ctx.worktree,
                &env.iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<Vec<_>>(),
                PtySize::default(),
                EnvPolicy::AllowlistedOnly,
            )?;

            // Keystroke strategy: pump output into the injector and type the
            // prompt once the TUI goes quiet (or max_wait elapses).
            if matches!(provider.prompt.strategy, PromptStrategy::Keystroke) {
                if let Some(prompt) = &ctx.initial_prompt {
                    self.deliver_keystroke_prompt(&mut handle, prompt, provider)?;
                }
            }

            let exit = match handle.wait_exit(AGENT_WAIT_TIMEOUT) {
                Ok(exit) => exit,
                Err(Error::LifecycleScriptTimeout(_)) => {
                    // The agent ran past the fallback window — treat as an
                    // exit with no code so the respawn policy still applies
                    // (the session is genuinely long-running).
                    let _ = handle.kill();
                    crate::terminals::pty::PtyExit {
                        exit_code: None,
                        signal: None,
                    }
                }
                Err(e) => return Err(e),
            };
            let _ = handle.flush();
            // Spill cleanup happens in SpillGuard::drop on every exit path
            // (normal exit, spawn error, wait_exit error).

            outcome.spawns = spawns;
            outcome.last_exit_code = exit.exit_code;
            outcome.last_signal = exit.signal.clone();

            self.events.send(InternalEvent::AgentRunFinished {
                conversation_id: ctx.conversation_id.clone(),
                provider: ctx.provider_id.clone(),
                // -1 = no exit code (force-killed after the timeout window).
                exit_code: exit.exit_code.map(|c| c as i32).unwrap_or(-1),
            });

            // Reference semantics: MAX_RESPAWNS respawns AFTER the initial
            // launch (initial + MAX_RESPAWNS = 3 spawns at the default).
            let should_respawn = ctx.respawn_resume && !ctx.tmux_enabled && spawns <= MAX_RESPAWNS;
            if !should_respawn {
                break;
            }
            std::thread::sleep(Duration::from_millis(RESPAWN_DELAY_MS));
        }

        self.events.send(InternalEvent::AgentSessionExited {
            conversation_id: ctx.conversation_id.clone(),
        });
        Ok(outcome)
    }

    /// Types the prompt into the TUI after a quiet beat (E3-03 injector).
    ///
    /// The pump: feed output to the injector, type what it asks, fire the
    /// delayed submit (when configured), and stop early if the agent exits
    /// before producing output (no 30s stall). Returns the observed exit —
    /// the probe reaps the child (portable-pty `try_wait` caches the reap),
    /// so the caller must use the returned exit instead of `wait_exit`.
    fn deliver_keystroke_prompt(
        &self,
        handle: &mut Box<dyn PtyHandle>,
        prompt: &str,
        provider: &ProviderDescriptor,
    ) -> Result<Option<crate::terminals::pty::PtyExit>, Error> {
        let spec = &provider.prompt;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut injector = match PromptInjector::new(
            prompt,
            spec.submit_sequence.as_deref(),
            spec.submit_delay_ms,
            500, // quiet-period default (no per-provider override)
            30_000,
            now,
        ) {
            Some(injector) => injector,
            None => return Ok(None), // blank prompt — nothing to type
        };

        let mut payload_sent = false;
        let mut submit_sent = injector.submit_sequence().is_empty();
        let mut submit_due: Option<std::time::Instant> = None;
        // With a submit delay, the pump owns the delayed submit (the
        // injector emits Payload then flips to Injected).
        let submit_delay = injector.submit_delay_ms().map(Duration::from_millis);

        let deadline = std::time::Instant::now() + Duration::from_millis(30_000);
        loop {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let mut buf = Vec::new();
            if handle.try_read(&mut buf).unwrap_or(false) && !buf.is_empty() {
                if let Some(injection) = injector.on_data(now) {
                    // A write failure usually means the agent died — non-fatal:
                    // fall through to the exit probe below.
                    let _ = self.write_injection(
                        handle,
                        &injector,
                        injection,
                        &mut payload_sent,
                        &mut submit_sent,
                        &mut submit_due,
                        submit_delay,
                    );
                }
            }
            if let Some(injection) = injector.on_tick(now) {
                let _ = self.write_injection(
                    handle,
                    &injector,
                    injection,
                    &mut payload_sent,
                    &mut submit_sent,
                    &mut submit_due,
                    submit_delay,
                );
            }
            // Delayed submit: fire once the configured delay after the payload
            // has elapsed.
            if !submit_sent && !injector.submit_sequence().is_empty() {
                if let Some(due) = submit_due {
                    if std::time::Instant::now() >= due {
                        let _ = handle.write(injector.submit_sequence());
                        submit_sent = true;
                    }
                }
            }
            // Probe FIRST: a child that died this iteration must be observed
            // here — returning on payload/submit first would leave wait_exit
            // polling a reaped child until AGENT_WAIT_TIMEOUT.
            if let Ok(Some(exit)) = handle.try_wait_exit() {
                if let Some(injection) = injector.on_exit() {
                    let _ = self.write_injection(
                        handle,
                        &injector,
                        injection,
                        &mut payload_sent,
                        &mut submit_sent,
                        &mut submit_due,
                        submit_delay,
                    );
                }
                return Ok(Some(exit));
            }
            if std::time::Instant::now() >= deadline {
                // Give up on quiet-waiting; type whatever is pending.
                if let Some(injection) = injector.on_exit() {
                    let _ = self.write_injection(
                        handle,
                        &injector,
                        injection,
                        &mut payload_sent,
                        &mut submit_sent,
                        &mut submit_due,
                        submit_delay,
                    );
                }
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn write_injection(
        &self,
        handle: &mut Box<dyn PtyHandle>,
        injector: &PromptInjector,
        injection: PromptInjection,
        payload_sent: &mut bool,
        submit_sent: &mut bool,
        submit_due: &mut Option<std::time::Instant>,
        submit_delay: Option<Duration>,
    ) -> Result<(), Error> {
        match injection {
            PromptInjection::Payload => {
                handle.write(injector.payload())?;
                *payload_sent = true;
                if !injector.submit_sequence().is_empty() {
                    // Delayed submit (when configured) is owned by the pump.
                    *submit_due =
                        Some(std::time::Instant::now() + submit_delay.unwrap_or_default());
                }
            }
            PromptInjection::Submit => {
                handle.write(injector.submit_sequence())?;
                *submit_sent = true;
            }
            PromptInjection::PayloadAndSubmit => {
                handle.write(injector.payload())?;
                if !injector.submit_sequence().is_empty() {
                    handle.write(injector.submit_sequence())?;
                }
                *payload_sent = true;
                *submit_sent = true;
            }
        }
        Ok(())
    }
}

/// Phase-3 hook (E2-07): rehydrate a remote terminal session after an SSH
/// reconnect. Phase 0 ships the stub — remote sessions are out of scope until
/// `ade-ssh` lands.
pub trait RemoteRehydrate: Send + Sync {
    fn rehydrate_remote(&self, session_id: &str) -> Result<(), Error>;
}

/// Phase-0 stub: no remote rehydration.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopRemoteRehydrate;
impl RemoteRehydrate for NoopRemoteRehydrate {
    fn rehydrate_remote(&self, _session_id: &str) -> Result<(), Error> {
        Ok(())
    }
}

/// Boot rehydration orchestration (E2-07, reference boot order: rehydrate
/// after DB init): for every project → task → PTY conversation that was
/// previously spawned, relaunch the agent with resume flags in its worktree.
///
/// Blocking (joins all launches); the app shell calls it on a background
/// thread so the window never blocks on agent spawns. Skipped with a warn:
/// conversations without a provider, tasks without a worktree row, and
/// worktree paths that no longer exist on disk (the non-tmux degradation).
#[derive(Clone)]
pub struct Rehydrator {
    launcher: Arc<AgentLauncher>,
    conversations: Arc<dyn ConversationStore>,
    tasks: Arc<dyn TaskStore>,
    projects: Arc<dyn ProjectStore>,
    db: Arc<dyn Db>,
    /// Trust-state fallback for auto-approve (the conversation's own toggle
    /// wins; see `AgentLauncher::rehydrate`).
    default_auto_approve: bool,
    /// Phase-3 remote hook (stub).
    remote: Arc<dyn RemoteRehydrate>,
}

/// Result of a boot rehydration pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RehydrateSummary {
    pub resumed: usize,
    pub skipped: usize,
    pub failed: usize,
}

impl Rehydrator {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pty: Arc<dyn PtyManager>,
        deps: Arc<HostDependencyStore>,
        events: Arc<BroadcastEventBus>,
        conversations: Arc<dyn ConversationStore>,
        tasks: Arc<dyn TaskStore>,
        projects: Arc<dyn ProjectStore>,
        db: Arc<dyn Db>,
        default_auto_approve: bool,
        remote: Arc<dyn RemoteRehydrate>,
    ) -> Self {
        Self {
            launcher: Arc::new(
                AgentLauncher::new(pty, deps, events)
                    .with_conversation_store(conversations.clone()),
            ),
            conversations,
            tasks,
            projects,
            db,
            default_auto_approve,
            remote,
        }
    }

    /// Rehydrates every previously-spawned PTY conversation (blocking).
    pub fn rehydrate_all(&self) -> Result<RehydrateSummary, Error> {
        let mut summary = RehydrateSummary::default();
        let mut handles = Vec::new();

        for project in self.projects.list()? {
            for task in self.tasks.list_by_project(&project.id)? {
                for conv in self.conversations.list_by_task(&task.id)? {
                    if conv.r#type != ConversationType::Pty {
                        continue; // ACP sessions are Phase 2
                    }
                    if conv.session_id.is_none() {
                        summary.skipped += 1; // never spawned
                        continue;
                    }
                    let Some(provider_id) = conv.provider.clone() else {
                        tracing::warn!(conversation = %conv.id, "rehydrate skipped: no provider");
                        summary.skipped += 1;
                        continue;
                    };
                    let Some(workspace_id) = &task.workspace_id else {
                        summary.skipped += 1; // byoi/project-root: no worktree to resume in
                        continue;
                    };
                    let worktree_path: String = self
                        .db
                        .conn()
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .query_row(
                            "SELECT path FROM workspaces WHERE id = ?1",
                            [workspace_id],
                            |row| row.get::<_, String>(0),
                        )
                        .optional()
                        .map_err(Error::from)?
                        .unwrap_or_default();
                    if worktree_path.is_empty() || !Path::new(&worktree_path).is_dir() {
                        tracing::warn!(
                            conversation = %conv.id,
                            path = %worktree_path,
                            "rehydrate skipped: worktree missing on disk"
                        );
                        summary.skipped += 1;
                        continue;
                    }
                    let target = RehydrateTarget {
                        provider_id,
                        task_id: task.id.clone(),
                        task_name: task.name.clone(),
                        worktree: PathBuf::from(&worktree_path),
                        root_path: project.path.clone(),
                        auto_approve: self.default_auto_approve,
                    };
                    let launcher = self.launcher.clone();
                    let conv = conv.clone();
                    let remote = self.remote.clone();
                    handles.push(std::thread::spawn(move || {
                        // Phase-3 hook: after a SUCCESSFUL local resume,
                        // notify the remote side (SSH reconnect re-attaches
                        // the terminal).
                        let result = launcher.rehydrate(&conv, &target);
                        if result.is_ok() {
                            if let Some(sid) = conv.session_id.as_deref() {
                                let _ = remote.rehydrate_remote(sid);
                            }
                        }
                        result
                    }));
                    summary.resumed += 1;
                }
            }
        }
        for handle in handles {
            match handle.join() {
                Ok(Ok(_)) => {}
                Ok(Err(_)) => summary.failed += 1,
                Err(_) => {
                    // One panicked conversation thread must not abort the
                    // pass or discard the rest of the accounting.
                    tracing::warn!("rehydrate thread panicked");
                    summary.failed += 1;
                }
            }
        }
        Ok(summary)
    }
}

/// The per-conversation context the app shell gathers for boot rehydration
/// (E2-07): which provider/task/worktree the stored conversation belongs to.
#[derive(Debug, Clone)]
pub struct RehydrateTarget {
    pub provider_id: String,
    pub task_id: String,
    pub task_name: String,
    pub worktree: PathBuf,
    pub root_path: PathBuf,
    pub auto_approve: bool,
}

/// Cleans a spilled prompt file when the launch-attempt scope exits — on
/// EVERY path (normal exit, spawn error, wait_exit error), so a respawn
/// never leaks its predecessor's temp file.
struct SpillGuard<'a>(&'a Option<std::path::PathBuf>);
impl Drop for SpillGuard<'_> {
    fn drop(&mut self) {
        if let Some(path) = self.0 {
            cleanup_spilled_prompt(path);
        }
    }
}

/// Finds an executable on PATH (names-outer: first listed binary wins).
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if candidate
                    .metadata()
                    .map(|m| m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
                {
                    return Some(candidate);
                }
            }
            #[cfg(not(unix))]
            return Some(candidate);
        }
    }
    None
}
