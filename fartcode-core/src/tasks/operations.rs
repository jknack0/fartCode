//! E2-04: Add Task flow — the operation layer that turns a dialog's typed
//! inputs into committed rows + a provisioned workspace.
//!
//! Ports the reference `createTask.ts` split into two phases:
//!
//! 1. **commit** (rows): validate the project + inputs, resolve the workspace
//!    intent, build the versioned conversation config, then atomically insert
//!    task + workspace + conversation rows (through `DbTaskStore::create`).
//! 2. **provision** (workspace): ensure the worktree / project-root workspace
//!    via `WorktreeManager` (E2-02), push non-fatal, auto-trust, and emit
//!    `task:provisioned`.
//!
//! The frontend dialog (name / branch source / provider+model pickers /
//! workspace target) lives in `app-frontend` (deferred — no node toolchain in
//! the Phase-0 env); this module is the core half it drives.

use std::sync::Arc;

use rusqlite::OptionalExtension;

use crate::db::Db;
use crate::events::{EventBus, InternalEvent};
use crate::git::{BranchRef, GitOps};
use crate::projects::model::project_from_row;
use crate::projects::worktrees::{EnsureWorktreeOptions, WorktreeManager};
use crate::settings::SettingsStore;
use crate::tasks::{
    CreateTaskOptions, InitialConversation, Task, TaskStatus, TaskStore, WorkspaceTarget,
};
use crate::Error;

/// Typed task configuration (reference `TaskConfig` v1 minus version).
#[derive(Debug, Clone, Default)]
pub struct TaskConfigParams {
    pub name: String,
    pub initial_status: Option<TaskStatus>,
    pub linked_issue: Option<crate::tasks::LinkedIssue>,
    pub initial_conversation: Option<InitialConversationConfig>,
}

/// Typed initial-conversation config (reference
/// `taskConfig.initialConversation`). `build_config` produces the versioned
/// `{version:'1', type, autoApprove?, initialPrompt?, model?, initialQueue?}`
/// JSON stored in `conversations.config`.
#[derive(Debug, Clone)]
pub struct InitialConversationConfig {
    pub id: String,
    pub provider: Option<String>,
    pub title: String,
    /// `"pty"` (default) or `"acp"` (Phase 2).
    pub r#type: Option<String>,
    pub auto_approve: Option<bool>,
    pub initial_prompt: Option<String>,
    pub model: Option<String>,
    pub initial_queue: Option<Vec<String>>,
}

impl InitialConversationConfig {
    pub fn new(
        id: impl Into<String>,
        provider: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            provider: Some(provider.into()),
            title: title.into(),
            r#type: None,
            auto_approve: None,
            initial_prompt: None,
            model: None,
            initial_queue: None,
        }
    }

    /// Versioned conversation config (reference `prepareCreateTask` configObj):
    /// pty → `{version, type:'pty', autoApprove?, initialPrompt?, model?}`;
    /// acp → `{version, type:'acp', autoApprove?, initialQueue?, model?}`.
    /// Fields are only present when set (delta-shaped).
    pub fn build_config(&self) -> serde_json::Value {
        let ty = self.r#type.as_deref().unwrap_or("pty");
        let mut obj = serde_json::Map::new();
        obj.insert("version".into(), serde_json::json!("1"));
        obj.insert("type".into(), serde_json::json!(ty));
        if let Some(aa) = self.auto_approve {
            obj.insert("autoApprove".into(), serde_json::json!(aa));
        }
        if ty == "acp" {
            if let Some(queue) = self
                .initial_queue
                .as_ref()
                .map(|q| {
                    q.iter()
                        .map(|t| t.trim())
                        .filter(|t| !t.is_empty())
                        .map(|t| serde_json::json!({ "text": t }))
                        .collect::<Vec<_>>()
                })
                .filter(|q| !q.is_empty())
            {
                obj.insert("initialQueue".into(), serde_json::Value::Array(queue));
            }
        } else if let Some(prompt) = self
            .initial_prompt
            .as_ref()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
        {
            obj.insert("initialPrompt".into(), serde_json::json!(prompt));
        }
        if let Some(model) = &self.model {
            obj.insert("model".into(), serde_json::json!(model));
        }
        serde_json::Value::Object(obj)
    }
}

/// A branch ref (reference `GitBranchRef` = `{type, branch, remote?}`):
/// the source a new branch is created from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBranchRef {
    pub r#type: String, // "local" | "remote"
    pub branch: String,
    pub remote: Option<String>,
}

impl SourceBranchRef {
    pub fn local(branch: impl Into<String>) -> Self {
        Self {
            r#type: "local".into(),
            branch: branch.into(),
            remote: None,
        }
    }

    pub fn remote(branch: impl Into<String>, remote: impl Into<String>) -> Self {
        Self {
            r#type: "remote".into(),
            branch: branch.into(),
            remote: Some(remote.into()),
        }
    }

    /// The string form `ensure_worktree`'s source_ref expects:
    /// `"origin/main"` for remote refs, `"main"` for local.
    pub fn as_source_ref(&self) -> String {
        match (self.r#type.as_str(), &self.remote) {
            ("remote", Some(remote)) => format!("{remote}/{}", self.branch),
            _ => self.branch.clone(),
        }
    }
}

/// Git setup intent for the workspace (reference `GitSetup`). The branch
/// names arrive ready-made from the dialog (E2-03 naming).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitSetup {
    None,
    UseBranch {
        branch_name: String,
    },
    CreateBranch {
        branch_name: String,
        from_branch: SourceBranchRef,
        push_branch: bool,
    },
}

impl GitSetup {
    pub fn branch_name(&self) -> Option<&str> {
        match self {
            GitSetup::None => None,
            GitSetup::UseBranch { branch_name } | GitSetup::CreateBranch { branch_name, .. } => {
                Some(branch_name)
            }
        }
    }
}

/// Everything the dialog hands over to create a task (reference
/// `CreateTaskParams`).
#[derive(Debug, Clone)]
pub struct CreateTaskParams {
    /// Override the task id (renderer-optimistic flows). Default: uuid v4.
    pub id: Option<String>,
    pub project_id: String,
    pub task_config: TaskConfigParams,
    pub git: GitSetup,
    pub workspace: WorkspaceTarget,
    pub automation_run_id: Option<String>,
}

/// Result of a successful create (reference `CreateTaskSuccess` minus the
/// full Conversation model — the conversations module lands in E2-05).
#[derive(Debug, Clone)]
pub struct CreateTaskSuccess {
    pub task: Task,
    pub initial_conversation_id: Option<String>,
    /// Isolation warning when the task runs in the project root instead of a
    /// worktree.
    pub warning: Option<String>,
}

/// E2-04 operation service. Wired once in the `App` struct (ARCHITECTURE §7).
pub struct TaskCreationService {
    db: Arc<dyn Db>,
    settings: Arc<dyn SettingsStore>,
    git: Arc<dyn GitOps>,
    worktrees: WorktreeManager,
    event_bus: Arc<dyn EventBus>,
}

impl TaskCreationService {
    pub fn new(
        db: Arc<dyn Db>,
        settings: Arc<dyn SettingsStore>,
        git: Arc<dyn GitOps>,
        worktrees: WorktreeManager,
        event_bus: Arc<dyn EventBus>,
    ) -> Self {
        Self {
            db,
            settings,
            git,
            worktrees,
            event_bus,
        }
    }

    /// Workspace-row access (one `Arc` clone — `new()`'s signature is wired
    /// in `App`, so the store is built per use rather than held).
    fn workspaces(&self) -> crate::workspaces::WorkspaceStore {
        crate::workspaces::WorkspaceStore::new(self.db.clone())
    }

    /// Dialog "start source = branch": list refs the picker offers
    /// (reference `git branch` list in the create-task dialog).
    pub fn list_branches(&self, project_id: &str) -> Result<Vec<BranchRef>, Error> {
        let project = self.project(project_id)?;
        self.git.branches(&project.path)
    }

    /// Trust handling (reference `workspaceTrustService.shouldAutoTrust`):
    /// auto-trust unless `tasks.autoTrustWorktrees` is off; forced when the
    /// conversation runs with auto-approve. The actual provider trust write
    /// lands with E2-06 — this is the decision surface.
    pub fn should_auto_trust(&self, force: bool) -> Result<bool, Error> {
        if force {
            return Ok(true);
        }
        let tasks: crate::settings::TaskGroup =
            serde_json::from_value(self.settings.get_json("tasks")?)?;
        Ok(tasks.auto_trust_worktrees)
    }

    /// Full create: commit rows, then provision the workspace (reference
    /// `createTask` + `provisionWorkspace`). All failures are typed `Error`s,
    /// never panics.
    pub fn create(&self, params: CreateTaskParams) -> Result<CreateTaskSuccess, Error> {
        // Validate the project exists (reference: project-not-found).
        self.project(&params.project_id)?;

        // -- validation (reference zod + workspaceTargetSchema) --------------
        if params.task_config.name.trim().is_empty() {
            return Err(Error::InvalidTaskInput("task name is required".into()));
        }
        if let Some(branch) = params.git.branch_name() {
            if branch.trim().is_empty() {
                return Err(Error::InvalidTaskInput(
                    "branch name is required for the selected start source".into(),
                ));
            }
        }
        if let Some(conv) = &params.task_config.initial_conversation {
            match conv.r#type.as_deref() {
                None | Some("pty") | Some("acp") => {}
                Some(other) => {
                    return Err(Error::InvalidTaskInput(format!(
                        "invalid conversation type: {other}"
                    )));
                }
            }
        }
        if let WorkspaceTarget::RepositoryInstance { workspace_id } = &params.workspace {
            // Typed-error fast path. Known Phase-0 limitation: `tasks.workspace_id`
            // is plain TEXT with no FK (0000_initial.sql), so a workspace deleted
            // between this check and the insert would commit silently and surface
            // as TaskNotFound at provision time. A proper `REFERENCES workspaces(id)`
            // needs an append-only migration (schema is hash-verified) — TODO with
            // the E2-05 schema work.
            let exists = self.workspaces().get(workspace_id)?.is_some();
            if !exists {
                return Err(Error::InvalidTaskInput(format!(
                    "workspace not found: {workspace_id}"
                )));
            }
        }

        // -- commit rows (reference commitCreateTask) ------------------------
        let conversation =
            params
                .task_config
                .initial_conversation
                .as_ref()
                .map(|ic| InitialConversation {
                    id: Some(ic.id.clone()),
                    title: ic.title.clone(),
                    provider: ic.provider.clone(),
                    config: Some(ic.build_config()),
                });
        let workspace_config = build_workspace_config(&params.git, &params.workspace);
        let store = crate::tasks::DbTaskStore::new(self.db.clone(), self.event_bus.clone());
        let task = store.create(CreateTaskOptions {
            project_id: params.project_id.clone(),
            name: params.task_config.name.clone(),
            id: params.id.clone(),
            initial_status: params.task_config.initial_status,
            linked_issue: params.task_config.linked_issue.clone(),
            initial_conversation: conversation,
            automation_run_id: params.automation_run_id.clone(),
            workspace_target: Some(params.workspace.clone()),
            workspace_config: Some(workspace_config),
        })?;

        // -- agent start placeholder (E2-06 launches the real agent) ---------
        let initial_conversation_id = params
            .task_config
            .initial_conversation
            .as_ref()
            .map(|ic| ic.id.clone());
        if let Some(conv) = &params.task_config.initial_conversation {
            let is_pty = conv.r#type.as_deref() != Some("acp");
            let has_prompt = conv
                .initial_prompt
                .as_ref()
                .map(|p| !p.trim().is_empty())
                .unwrap_or(false);
            if is_pty && has_prompt {
                // Reference emits the start event with providerId even when
                // the conversation has no provider (defaults to the project's).
                self.event_bus.send(InternalEvent::AgentStart {
                    provider: conv.provider.clone().unwrap_or_default(),
                    project_id: params.project_id.clone(),
                    task_id: task.id.clone(),
                    conversation_id: conv.id.clone(),
                });
            }
        }

        Ok(CreateTaskSuccess {
            task,
            initial_conversation_id,
            warning: None,
        })
    }

    /// Combined happy path: `create` + `provision`. On provision failure the
    /// rows are already committed — the error is honest and the caller can
    /// retry with `provision(task_id)` (idempotent).
    pub fn create_with_provision(
        &self,
        params: CreateTaskParams,
    ) -> Result<CreateTaskSuccess, Error> {
        let created = self.create(params)?;
        let provisioned = self.provision(&created.task.id)?;
        Ok(CreateTaskSuccess {
            task: created.task,
            initial_conversation_id: created.initial_conversation_id,
            warning: provisioned.warning,
        })
    }

    // -- internals -----------------------------------------------------------

    fn project(&self, project_id: &str) -> Result<crate::projects::model::Project, Error> {
        self.with_conn(|conn| {
            conn.query_row(
                &format!(
                    "SELECT {} FROM projects WHERE id = ?1",
                    crate::projects::model::PROJECT_COLUMNS
                ),
                [project_id],
                project_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::ProjectNotFound(project_id.into()))
        })
    }

    fn with_conn<T>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
        f(&conn)
    }

    /// Reference `provisionWorkspace` → `ensureWorkspaceSetupForTask`: make the
    /// workspace real on disk, then fire `task:provisioned`. Idempotent —
    /// `ensure_worktree` reuses a valid existing worktree. The workspace
    /// intent (git setup + target) is read from the workspace row's versioned
    /// config written at create time, so it survives a restart.
    pub fn provision(&self, task_id: &str) -> Result<ProvisionResult, Error> {
        let store = crate::tasks::DbTaskStore::new(self.db.clone(), self.event_bus.clone());
        let task = store
            .get(task_id)?
            .ok_or_else(|| Error::TaskNotFound(task_id.into()))?;
        let workspace_id = task
            .workspace_id
            .clone()
            .ok_or_else(|| Error::Internal("task has no workspace row".into()))?;
        let project = self.project(&task.project_id)?;

        // The workspace row's `kind` discriminates without touching the
        // config: byoi/project-root rows (incl. repository-instance reuse of
        // the repo workspace, which has no config) never need a worktree.
        let row_kind = self
            .workspaces()
            .kind(&workspace_id)?
            .ok_or_else(|| Error::TaskNotFound(format!("workspace {workspace_id}")))?;

        let (path, warning) = match row_kind.as_str() {
            "byoi" => (None, None),
            "project-root" => {
                // Either target ProjectRoot (ensure the disabled workspace) or
                // a repository-instance reusing the repo workspace (no-op).
                let (_git, target) = self.workspace_intent(&workspace_id)?;
                match target {
                    WorkspaceTarget::ProjectRoot => {
                        let result = self.worktrees.ensure_worktree(&EnsureWorktreeOptions {
                            project: &project,
                            task_id: &task.id,
                            workspace_id: &workspace_id,
                            branch_name: "",
                            source_ref: None,
                            worktree_enabled: false,
                        })?;
                        (Some(result.path), result.warning)
                    }
                    _ => (None, None),
                }
            }
            _ => {
                // worktree-kind rows: the config carries the git setup.
                let (git, target) = match self.workspace_intent(&workspace_id) {
                    Ok(intent) => intent,
                    Err(_) => {
                        // Legacy row (created before the command provisioned
                        // at create time): no config to read, so mint the
                        // default intent — fresh branch off the project's
                        // base ref — and persist it so re-provision and
                        // deletion see a stable setup.
                        let git = self.default_git_setup(&project, &task)?;
                        let target = WorkspaceTarget::NewWorktree;
                        let config = build_workspace_config(&git, &target);
                        self.workspaces()
                            .set_config(&workspace_id, &config.to_string())?;
                        (git, target)
                    }
                };
                match target {
                    WorkspaceTarget::RepositoryInstance { .. } | WorkspaceTarget::Byoi { .. } => {
                        (None, None)
                    }
                    WorkspaceTarget::ProjectRoot => {
                        let result = self.worktrees.ensure_worktree(&EnsureWorktreeOptions {
                            project: &project,
                            task_id: &task.id,
                            workspace_id: &workspace_id,
                            branch_name: "",
                            source_ref: None,
                            worktree_enabled: false,
                        })?;
                        (Some(result.path), result.warning)
                    }
                    WorkspaceTarget::NewWorktree => {
                        let branch = git.branch_name().ok_or_else(|| {
                            Error::InvalidTaskInput(
                                "new-worktree target requires a branch (use-branch/create-branch)"
                                    .into(),
                            )
                        })?;
                        let source_ref = match &git {
                            GitSetup::CreateBranch { from_branch, .. } => {
                                Some(from_branch.as_source_ref())
                            }
                            _ => None, // use-branch: existing-branch flow (fetch + track)
                        };
                        let branch = branch.trim();
                        let source_ref = source_ref.as_deref().map(str::trim);
                        let result = self.worktrees.ensure_worktree(&EnsureWorktreeOptions {
                            project: &project,
                            task_id: &task.id,
                            workspace_id: &workspace_id,
                            branch_name: branch,
                            source_ref,
                            worktree_enabled: true,
                        })?;
                        let trust_force = self.initial_conversation_auto_approve(&task.id)?;
                        self.maybe_auto_trust(&result.path, trust_force);
                        (Some(result.path), result.warning)
                    }
                }
            }
        };

        self.event_bus.send(InternalEvent::TaskProvisioned {
            id: task.id.clone(),
            workspace_id: workspace_id.clone(),
        });
        Ok(ProvisionResult {
            workspace_id,
            path,
            warning,
        })
    }

    /// The default git setup for legacy workspace rows healed by provision:
    /// `fartCode/<task-slug>-<suffix>` off the project's base ref, never pushed
    /// (create-time `push_on_create` applies to the create flow only — a
    /// heal must not surprise-push).
    fn default_git_setup(
        &self,
        project: &crate::projects::model::Project,
        task: &Task,
    ) -> Result<GitSetup, Error> {
        let group: crate::settings::ProjectGroup =
            serde_json::from_value(self.settings.get_json("project")?)?;
        let raw_branch = crate::tasks::naming::generate_task_name(Some(&task.name), None, true);
        let suffix = crate::tasks::naming::random_suffix();
        let branch_name = crate::tasks::naming::resolve_task_branch_name(
            &crate::tasks::naming::BranchNameOptions {
                raw_branch: &raw_branch,
                branch_prefix: Some(&group.branch_prefix),
                suffix: &suffix,
                append_random_suffix: group.append_random_branch_suffix,
                linked_issue: None,
                disable_random_suffix: false,
            },
        );
        let base_ref = project.base_ref();
        let from_branch = match base_ref.split_once('/') {
            Some((remote, branch)) => SourceBranchRef::remote(branch, remote),
            None => SourceBranchRef::local(base_ref),
        };
        Ok(GitSetup::CreateBranch {
            branch_name,
            from_branch,
            push_branch: false,
        })
    }

    /// Reference `workspaceTrustService.maybeAutoTrust`: mark the workspace as
    /// trusted so the agent skips its trust prompt. Phase 0 records the
    /// decision; the actual provider-config write arrives with E2-06.
    fn maybe_auto_trust(&self, workspace_path: &std::path::Path, force: bool) {
        match self.should_auto_trust(force) {
            Ok(true) => {
                tracing::info!(
                    path = %workspace_path.display(),
                    force,
                    "auto-trust worktree (provider trust write lands with E2-06)"
                );
            }
            Ok(false) => tracing::debug!("auto-trust disabled for task"),
            Err(e) => tracing::warn!(error = %e, "auto-trust decision failed (non-fatal)"),
        }
    }

    /// autoApprove on the task's initial conversation forces trust (reference
    /// "forced trust when autoApprove").
    fn initial_conversation_auto_approve(&self, task_id: &str) -> Result<bool, Error> {
        let config: Option<String> = self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT config FROM conversations WHERE task_id = ?1 AND is_initial_conversation = 1 LIMIT 1",
                    [task_id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten())
        })?;
        Ok(config
            .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
            .and_then(|v| v.get("autoApprove").and_then(|a| a.as_bool()))
            .unwrap_or(false))
    }

    /// Reads the workspace intent from the workspace row's versioned config
    /// (written by `build_workspace_config` at create time) — the inverse
    /// shape, so `provision()` survives restarts. Rows created without a
    /// config (pre-E2-04 store.create / the repository workspace) fall back
    /// to their `kind` column.
    fn workspace_intent(&self, workspace_id: &str) -> Result<(GitSetup, WorkspaceTarget), Error> {
        let row = self
            .workspaces()
            .get(workspace_id)?
            .ok_or_else(|| Error::TaskNotFound(format!("workspace {workspace_id}")))?;
        let Some(config) = row.config else {
            // Legacy row: infer the target from its kind. The repository
            // workspace (kind 'project-root', no config) is a no-op reuse;
            // byoi rows are Phase-0 stubs; a worktree-kind row without config
            // cannot be provisioned (no branch known).
            return match row.kind.as_deref() {
                Some("byoi") => Ok((
                    GitSetup::None,
                    WorkspaceTarget::Byoi {
                        remote_workspace_id: None,
                    },
                )),
                Some("project-root") => Ok((
                    GitSetup::None,
                    WorkspaceTarget::RepositoryInstance {
                        workspace_id: workspace_id.into(),
                    },
                )),
                _ => Err(Error::InvalidTaskInput(format!(
                    "workspace {workspace_id} has no versioned config — cannot determine intent"
                ))),
            };
        };
        let value: serde_json::Value = serde_json::from_str(&config).map_err(Error::from)?;
        let git = parse_git_setup(&value["git"])?;
        let workspace = parse_workspace_target(&value["workspace"])?;
        Ok((git, workspace))
    }
}

/// Outcome of `TaskCreationService::provision` (reference `ProvisionResult`).
#[derive(Debug, Clone)]
pub struct ProvisionResult {
    pub workspace_id: String,
    /// Local path when a worktree / project-root workspace was ensured;
    /// `None` for repository-instance reuse and byoi (Phase 0 stub).
    pub path: Option<std::path::PathBuf>,
    pub warning: Option<String>,
}

/// Versioned workspace config stored on the workspace row
/// (`workspaces.config`), mirroring the reference v2 `workspaceConfig`.
pub fn build_workspace_config(git: &GitSetup, workspace: &WorkspaceTarget) -> serde_json::Value {
    let git_obj = match git {
        GitSetup::None => serde_json::json!({ "kind": "none" }),
        GitSetup::UseBranch { branch_name } => serde_json::json!({
            "kind": "use-branch",
            "branchName": branch_name,
        }),
        GitSetup::CreateBranch {
            branch_name,
            from_branch,
            push_branch,
        } => serde_json::json!({
            "kind": "create-branch",
            "branchName": branch_name,
            "fromBranch": {
                "type": from_branch.r#type,
                "branch": from_branch.branch,
                // Phase 0 tracks remote NAMES only; urls arrive with the git
                // remote plumbing (E2-08).
                "remote": from_branch.remote.as_ref().map(|name| {
                    serde_json::json!({ "name": name, "url": serde_json::Value::Null })
                }),
            },
            "pushBranch": push_branch,
        }),
    };
    let ws_obj = match workspace {
        WorkspaceTarget::RepositoryInstance { workspace_id } => serde_json::json!({
            "kind": "repository-instance",
            "workspaceId": workspace_id,
        }),
        WorkspaceTarget::NewWorktree => serde_json::json!({ "kind": "new-worktree" }),
        WorkspaceTarget::ProjectRoot => serde_json::json!({ "kind": "project-root" }),
        WorkspaceTarget::Byoi {
            remote_workspace_id,
        } => serde_json::json!({
            "kind": "byoi",
            "remoteWorkspaceId": remote_workspace_id,
        }),
    };
    serde_json::json!({ "version": "2", "git": git_obj, "workspace": ws_obj })
}

/// Inverse of `build_workspace_config`'s `git` arm.
fn parse_git_setup(v: &serde_json::Value) -> Result<GitSetup, Error> {
    let kind = v.get("kind").and_then(|k| k.as_str());
    match kind {
        None | Some("none") => Ok(GitSetup::None),
        Some("use-branch") => Ok(GitSetup::UseBranch {
            branch_name: v["branchName"].as_str().unwrap_or("").to_string(),
        }),
        Some("create-branch") => {
            let fb = v
                .get("fromBranch")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            // remote is either the reference {name, url} object or (legacy) a
            // bare name string — both parse.
            let remote = fb
                .get("remote")
                .and_then(|r| r.as_str())
                .or_else(|| {
                    fb.get("remote")
                        .and_then(|r| r.get("name"))
                        .and_then(|n| n.as_str())
                })
                .map(String::from);
            Ok(GitSetup::CreateBranch {
                branch_name: v["branchName"].as_str().unwrap_or("").to_string(),
                from_branch: SourceBranchRef {
                    r#type: fb
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("local")
                        .to_string(),
                    branch: fb
                        .get("branch")
                        .and_then(|b| b.as_str())
                        .unwrap_or("")
                        .to_string(),
                    remote,
                },
                push_branch: v["pushBranch"].as_bool().unwrap_or(false),
            })
        }
        Some(other) => Err(Error::InvalidTaskInput(format!(
            "unknown git setup kind: {other}"
        ))),
    }
}

/// Inverse of `build_workspace_config`'s `workspace` arm.
fn parse_workspace_target(v: &serde_json::Value) -> Result<WorkspaceTarget, Error> {
    let kind = v.get("kind").and_then(|k| k.as_str());
    match kind {
        Some("new-worktree") | None => Ok(WorkspaceTarget::NewWorktree),
        Some("repository-instance") => Ok(WorkspaceTarget::RepositoryInstance {
            workspace_id: v["workspaceId"].as_str().unwrap_or("").to_string(),
        }),
        Some("project-root") => Ok(WorkspaceTarget::ProjectRoot),
        Some("byoi") => Ok(WorkspaceTarget::Byoi {
            remote_workspace_id: v
                .get("remoteWorkspaceId")
                .and_then(|r| r.as_str())
                .map(String::from),
        }),
        Some(other) => Err(Error::InvalidTaskInput(format!(
            "unknown workspace target kind: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_config_pty_defaults() {
        let cfg = InitialConversationConfig::new("c1", "claude", "Fix it").build_config();
        assert_eq!(cfg["version"], "1");
        assert_eq!(cfg["type"], "pty");
        // Delta-shaped: absent fields are omitted, not null.
        assert!(cfg.get("autoApprove").is_none());
        assert!(cfg.get("initialPrompt").is_none());
        assert!(cfg.get("model").is_none());
        assert!(cfg.get("initialQueue").is_none());
    }

    #[test]
    fn conversation_config_pty_with_model_and_prompt() {
        let mut ic = InitialConversationConfig::new("c1", "claude", "Fix it");
        ic.initial_prompt = Some("  Fix the bug  ".into());
        ic.model = Some("sonnet".into());
        let cfg = ic.build_config();
        assert_eq!(cfg["version"], "1");
        assert_eq!(cfg["type"], "pty");
        assert_eq!(cfg["initialPrompt"], "Fix the bug"); // trimmed
        assert_eq!(cfg["model"], "sonnet");
        assert!(cfg.get("initialQueue").is_none());
        assert!(cfg.get("autoApprove").is_none());
    }

    #[test]
    fn conversation_config_acp_uses_initial_queue() {
        let mut ic = InitialConversationConfig::new("c1", "codex", "Review");
        ic.r#type = Some("acp".into());
        ic.initial_queue = Some(vec!["do it".into(), "  ".into()]);
        ic.auto_approve = Some(true);
        let cfg = ic.build_config();
        assert_eq!(cfg["type"], "acp");
        assert_eq!(cfg["autoApprove"], true);
        assert_eq!(
            cfg["initialQueue"],
            serde_json::json!([{ "text": "do it" }])
        );
        assert!(cfg.get("initialPrompt").is_none());
    }

    #[test]
    fn workspace_config_matches_reference_v2_shape() {
        let cfg = build_workspace_config(
            &GitSetup::CreateBranch {
                branch_name: "fartCode/fix-bug-abc12".into(),
                from_branch: SourceBranchRef::remote("main", "origin"),
                push_branch: true,
            },
            &WorkspaceTarget::NewWorktree,
        );
        assert_eq!(cfg["version"], "2");
        assert_eq!(cfg["git"]["kind"], "create-branch");
        assert_eq!(cfg["git"]["branchName"], "fartCode/fix-bug-abc12");
        assert_eq!(cfg["git"]["fromBranch"]["type"], "remote");
        assert_eq!(cfg["git"]["fromBranch"]["branch"], "main");
        assert_eq!(cfg["git"]["fromBranch"]["remote"]["name"], "origin");
        assert_eq!(cfg["git"]["pushBranch"], true);
        assert_eq!(cfg["workspace"]["kind"], "new-worktree");

        // Round-trip: parse back to the same intent (provision() reads this).
        let git = parse_git_setup(&cfg["git"]).unwrap();
        let ws = parse_workspace_target(&cfg["workspace"]).unwrap();
        assert_eq!(
            git,
            GitSetup::CreateBranch {
                branch_name: "fartCode/fix-bug-abc12".into(),
                from_branch: SourceBranchRef::remote("main", "origin"),
                push_branch: true,
            }
        );
        assert_eq!(ws, WorkspaceTarget::NewWorktree);
        assert_eq!(git.branch_name(), Some("fartCode/fix-bug-abc12"));
    }
}
