//! Typed setting keys, defaults, and schemas (ARCHITECTURE.md §2, §6.2).
//!
//! Ports the reference's `settings-registry.ts` + `project-settings.ts`
//! with the product rebrand applied (branch prefix `fartCode`, paths under
//! `~/fartCode/`, config file `.fartCode.json`).
//!
//! Wire format note: group structs are Rust `snake_case` but serialize to
//! the reference's `camelCase` JSON keys so the frontend can consume them
//! without translation. All group structs tolerate partial JSON (stored
//! values are deltas vs defaults) via `#[serde(default)]`.

use std::marker::PhantomData;
use std::path::PathBuf;

use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// Values storable as app settings: typed, cloneable, JSON-serializable.
pub trait SettingValue:
    Serialize + DeserializeOwned + PartialEq + Clone + std::fmt::Debug + Send + Sync + 'static
{
}
impl<T> SettingValue for T where
    T: Serialize + DeserializeOwned + PartialEq + Clone + std::fmt::Debug + Send + Sync + 'static
{
}

/// A typed app-setting key (e.g. `SETTINGS.project`).
pub struct SettingKey<T: SettingValue> {
    name: &'static str,
    marker: PhantomData<T>,
}

impl<T: SettingValue> SettingKey<T> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            marker: PhantomData,
        }
    }

    /// The key as stored in the `app_settings` table.
    pub fn name(&self) -> &'static str {
        self.name
    }
}

// ---------------------------------------------------------------------------
// App-settings groups (reference `SETTINGS_DEFAULTS`, rebranded)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectGroup {
    pub push_on_create: bool,
    pub branch_prefix: String,
    pub append_random_branch_suffix: bool,
    pub tmux_by_default: bool,
}

impl Default for ProjectGroup {
    fn default() -> Self {
        Self {
            push_on_create: true,
            branch_prefix: "fartCode".into(),
            append_random_branch_suffix: true,
            tmux_by_default: false,
        }
    }
}

/// E1-09: resource monitor toggle (disabled by default — acceptance).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ResourceMonitorGroup {
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskGroup {
    pub auto_generate_name: bool,
    pub auto_approve_by_default: bool,
    pub auto_trust_worktrees: bool,
    pub create_branch_and_worktree: bool,
    pub delete_branch_by_default: bool,
    pub preserve_name_capitalization: bool,
    pub include_issue_context_by_default: bool,
}

impl Default for TaskGroup {
    fn default() -> Self {
        Self {
            auto_generate_name: true,
            auto_approve_by_default: false,
            auto_trust_worktrees: true,
            create_branch_and_worktree: true,
            delete_branch_by_default: false,
            preserve_name_capitalization: false,
            include_issue_context_by_default: true,
        }
    }
}

/// Paths resolve at call time from the user's home directory (function-valued
/// default in the reference), so `Default` is implemented manually.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalProjectGroup {
    pub default_projects_directory: String,
    pub default_worktree_directory: String,
    pub write_agent_config_to_git_ignore: bool,
}

impl Default for LocalProjectGroup {
    fn default() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            default_projects_directory: home
                .join("fartCode/repositories")
                .to_string_lossy()
                .into_owned(),
            default_worktree_directory: home
                .join("fartCode/worktrees")
                .to_string_lossy()
                .into_owned(),
            write_agent_config_to_git_ignore: true,
        }
    }
}

/// Valid shell ids (reference `terminal-settings.ts`).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum TerminalShell {
    #[default]
    System,
    Bash,
    Cmd,
    Fish,
    Powershell,
    Pwsh,
    Wsl,
    Zsh,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct TerminalGroup {
    pub default_shell: TerminalShell,
    pub auto_copy_on_selection: bool,
    pub mac_option_is_meta: bool,
}

impl Default for TerminalGroup {
    fn default() -> Self {
        Self {
            default_shell: TerminalShell::System,
            auto_copy_on_selection: false,
            mac_option_is_meta: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationGroup {
    pub enabled: bool,
    pub sound: bool,
    pub custom_sound_path: String,
    pub os_notifications: bool,
    pub sound_focus_mode: String,
}

impl Default for NotificationGroup {
    fn default() -> Self {
        Self {
            enabled: true,
            sound: true,
            custom_sound_path: String::new(),
            os_notifications: true,
            sound_focus_mode: "always".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct BrowserPreviewGroup {
    pub enabled: bool,
}

impl Default for BrowserPreviewGroup {
    fn default() -> Self {
        Self { enabled: true }
    }
}

// ---------------------------------------------------------------------------
// Typed keys + defaults
// ---------------------------------------------------------------------------

pub const DEFAULT_AGENT_ID: &str = "claude";

pub static PROJECT: SettingKey<ProjectGroup> = SettingKey::new("project");
pub static TASKS: SettingKey<TaskGroup> = SettingKey::new("tasks");
pub static RESOURCE_MONITOR: SettingKey<ResourceMonitorGroup> = SettingKey::new("resourceMonitor");
pub static DEFAULT_AGENT: SettingKey<String> = SettingKey::new("defaultAgent");
pub static LOCAL_PROJECT: SettingKey<LocalProjectGroup> = SettingKey::new("localProject");
pub static TERMINAL: SettingKey<TerminalGroup> = SettingKey::new("terminal");
pub static NOTIFICATIONS: SettingKey<NotificationGroup> = SettingKey::new("notifications");
pub static BROWSER_PREVIEW: SettingKey<BrowserPreviewGroup> = SettingKey::new("browserPreview");

/// All registered app-setting keys, in registry order.
pub fn all_keys() -> &'static [&'static str] {
    &[
        "project",
        "tasks",
        "defaultAgent",
        "localProject",
        "terminal",
        "notifications",
        "browserPreview",
        "resourceMonitor",
    ]
}

/// The default value for `key` (computed on demand — `localProject` paths are
/// dynamic). Returns `None` for unknown keys.
pub fn default_value(key: &str) -> Option<serde_json::Value> {
    let value = match key {
        "project" => serde_json::to_value(ProjectGroup::default()).ok()?,
        "tasks" => serde_json::to_value(TaskGroup::default()).ok()?,
        "defaultAgent" => serde_json::Value::String(DEFAULT_AGENT_ID.into()),
        "localProject" => serde_json::to_value(LocalProjectGroup::default()).ok()?,
        "terminal" => serde_json::to_value(TerminalGroup::default()).ok()?,
        "notifications" => serde_json::to_value(NotificationGroup::default()).ok()?,
        "browserPreview" => serde_json::to_value(BrowserPreviewGroup::default()).ok()?,
        "resourceMonitor" => serde_json::to_value(ResourceMonitorGroup::default()).ok()?,
        _ => return None,
    };
    Some(value)
}

/// Validates and canonically re-serializes `value` for `key`: deserializes to
/// the key's concrete type (rejecting wrong types and unknown enum values) and
/// returns the canonical JSON with unknown keys stripped — the reference's zod
/// `.parse` behavior. Unknown keys must never leak into stored deltas.
pub fn canonical_value(
    key: &str,
    value: &serde_json::Value,
) -> Result<serde_json::Value, crate::Error> {
    let parsed: Result<serde_json::Value, serde_json::Error> = match key {
        "project" => {
            serde_json::from_value::<ProjectGroup>(value.clone()).and_then(serde_json::to_value)
        }
        "tasks" => {
            serde_json::from_value::<TaskGroup>(value.clone()).and_then(serde_json::to_value)
        }
        "defaultAgent" => {
            serde_json::from_value::<String>(value.clone()).and_then(serde_json::to_value)
        }
        "localProject" => serde_json::from_value::<LocalProjectGroup>(value.clone())
            .and_then(serde_json::to_value),
        "terminal" => {
            serde_json::from_value::<TerminalGroup>(value.clone()).and_then(serde_json::to_value)
        }
        "notifications" => serde_json::from_value::<NotificationGroup>(value.clone())
            .and_then(serde_json::to_value),
        "browserPreview" => serde_json::from_value::<BrowserPreviewGroup>(value.clone())
            .and_then(serde_json::to_value),
        "resourceMonitor" => serde_json::from_value::<ResourceMonitorGroup>(value.clone())
            .and_then(serde_json::to_value),
        _ => return Err(crate::Error::InvalidSettingKey(key.into())),
    };
    parsed.map_err(|e| crate::Error::InvalidSettingValue {
        key: key.into(),
        reason: e.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Project settings schemas (reference `project-settings.ts`)
// ---------------------------------------------------------------------------

/// The name of the shareable project config file in a repo.
pub const PROJECT_CONFIG_FILE: &str = ".fartCode.json";

/// The reference app's config file, read by the legacy v1.1.15 migration.
pub const LEGACY_CONFIG_FILE: &str = ".emdash.json";

pub const DEFAULT_BRANCH_FALLBACK: &str = "main";
pub const DEFAULT_BASE_REMOTE: &str = "origin";

pub const DEFAULT_PRESERVE_PATTERNS: &[&str] = &[
    ".env",
    ".env.keys",
    ".env.local",
    ".env.*.local",
    ".envrc",
    "docker-compose.override.yml",
];

/// `defaultBranch` is either a plain name (`main`) or a remote-tracked form
/// (`{ name, remote: true }`).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(untagged)]
pub enum DefaultBranch {
    Name(String),
    Remote { name: String, remote: bool },
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceProvider {
    pub r#type: String,
    pub provision_command: Option<String>,
    pub terminate_command: Option<String>,
}

/// DB-backed project settings fields (project_settings.base_project_settings_json).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BaseProjectSettings {
    pub worktree_directory: Option<String>,
    pub default_branch: Option<DefaultBranch>,
    pub base_remote: Option<String>,
    pub push_remote: Option<String>,
    pub github_account_id: Option<String>,
    pub tmux: Option<bool>,
    pub auto_run_setup_script_on_task_creation: Option<bool>,
    pub auto_run_run_script_on_task_creation: Option<bool>,
    /// Command run INSTEAD of a blank shell when a task terminal opens
    /// (E2-13, #52). `None`/blank = current behavior (`$SHELL`). Local
    /// preference, so deliberately NOT in the shareable subset.
    pub task_startup_command: Option<String>,
    pub workspace_provider: Option<WorkspaceProvider>,
    /// Feature-dossier consent (E19-01, #70; ADR-0038 item 3). Writing
    /// `docs/features/<slug>.md` into someone's repo is never silent, so
    /// this is a per-project opt-in asked at first dispatch (the consent
    /// card is #74) and reversible from project settings in both
    /// directions.
    ///
    /// Three states, and the third is the point:
    /// - `Some(true)`  — write dossiers.
    /// - `Some(false)` — REFUSE. The backend writes nothing, existing
    ///   dossiers stop collecting breadcrumbs, and `dossier_path` stays
    ///   NULL; declining still dispatches (ADR-0038 item 3: "declining
    ///   runs the step without memory").
    /// - `None`        — NOT YET ASKED. **Also refuses**, and today that
    ///   is every project: nothing can set this until #74's consent card
    ///   ships, so unset is the universal state, not an edge case.
    ///
    /// Unset fails CLOSED because the alternative is not "one stray
    /// markdown file on a branch". The dispatch prompt tells the agent to
    /// commit as it goes, so an unrequested dossier rides the feature
    /// branch into the user's pull request — a visible, public artifact
    /// they never asked for, in a repository that is theirs and not ours.
    /// "We have not asked yet" and "we could not read the answer" deserve
    /// the same response: don't. The accepted cost is that the feature is
    /// inert until #74 lands and starts writing `Some(_)` on BOTH answers,
    /// at which point this default stops being reachable in practice.
    ///
    /// DB-backed (base), deliberately NOT in the shareable subset: consent
    /// to write into a checkout is the local user's, not something a
    /// teammate's `.fartCode.json` should decide for them.
    pub feature_dossiers: Option<bool>,
    /// The [`crate::skills::FEATURE_LOG_VERSION`] whose scaffold was last
    /// successfully written into this project (E19-02, #71).
    ///
    /// **This is what makes deleting the scaffold stick.** Seeding runs on
    /// every step launch, so without a memory of "we already did this" the
    /// files would resurrect on the next card — and the removal
    /// instructions printed inside them ("delete this directory to remove
    /// the convention") would be a lie that costs the user a line in every
    /// future pull request. `Some(v) >= FEATURE_LOG_VERSION` means don't
    /// look and don't write; a version bump makes the comparison fail
    /// again, so a genuine format change still self-heals.
    ///
    /// Written only after a write actually succeeded — a refusal (no
    /// consent) or a decline (the paths belong to the user) records
    /// nothing, so neither is mistaken for "done".
    ///
    /// Base, not shareable, for the same reason as `feature_dossiers`: it
    /// describes what happened to this machine's checkout.
    pub feature_log_seeded_version: Option<u32>,
    /// Chain-guard config (#82). `step_chain_depth_cap` caps consecutive
    /// AUTOMATIC agent-step launches (settle-chained, no human confirm)
    /// per card; `None` = the built-in default
    /// [`DEFAULT_STEP_CHAIN_DEPTH_CAP`]. `step_budget_tokens` is an
    /// optional per-project ceiling on total provider-reported token
    /// usage in the step ledger; `None` = no budget. Both are base-only:
    /// spend guardrails on this machine's launches are the local user's,
    /// not something a teammate's `.fartCode.json` should loosen.
    pub step_chain_depth_cap: Option<u32>,
    pub step_budget_tokens: Option<i64>,
}

/// Default per-card chain depth cap (#82): after this many consecutive
/// automatic launches with no human confirm, the chain holds.
pub const DEFAULT_STEP_CHAIN_DEPTH_CAP: u32 = 3;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Scripts {
    pub setup: Option<String>,
    pub run: Option<String>,
    pub teardown: Option<String>,
}

/// The shareable subset, synced to `.fartCode.json` (shareable_project_settings_json).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ShareableProjectSettings {
    pub preserve_patterns: Option<Vec<String>>,
    pub shell_setup: Option<String>,
    pub scripts: Option<Scripts>,
}

/// The default shareable settings (`preservePatterns` only).
pub fn default_shareable() -> ShareableProjectSettings {
    ShareableProjectSettings {
        preserve_patterns: Some(
            DEFAULT_PRESERVE_PATTERNS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        ),
        ..Default::default()
    }
}

/// Filters the project's own config file out of a preserve-patterns list
/// (reference: `.emdash.json` filtered; we filter `.fartCode.json`).
pub fn filter_config_file(patterns: &[String]) -> Vec<String> {
    patterns
        .iter()
        .filter(|p| p.as_str() != PROJECT_CONFIG_FILE)
        .cloned()
        .collect()
}

/// Effective (merged) project settings: base fields + shareable fields.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectSettings {
    // -- base (DB-backed) --
    pub worktree_directory: Option<String>,
    pub default_branch: Option<DefaultBranch>,
    pub base_remote: Option<String>,
    pub push_remote: Option<String>,
    pub github_account_id: Option<String>,
    pub tmux: Option<bool>,
    pub auto_run_setup_script_on_task_creation: Option<bool>,
    pub auto_run_run_script_on_task_creation: Option<bool>,
    /// Command run instead of a blank shell when a task terminal opens
    /// (E2-13, #52).
    pub task_startup_command: Option<String>,
    pub workspace_provider: Option<WorkspaceProvider>,
    /// Feature-dossier consent (E19-01) — see
    /// [`BaseProjectSettings::feature_dossiers`] for the three states and
    /// the interim unset behavior.
    pub feature_dossiers: Option<bool>,
    /// Last feature-log scaffold version written into this project — see
    /// [`BaseProjectSettings::feature_log_seeded_version`]. App-managed
    /// bookkeeping, not a user-facing switch.
    pub feature_log_seeded_version: Option<u32>,
    /// Chain-guard config (#82) — see
    /// [`BaseProjectSettings::step_chain_depth_cap`].
    pub step_chain_depth_cap: Option<u32>,
    pub step_budget_tokens: Option<i64>,
    // -- shareable (.fartCode.json-synced) --
    pub preserve_patterns: Option<Vec<String>>,
    pub shell_setup: Option<String>,
    pub scripts: Option<Scripts>,
}

impl ProjectSettings {
    /// The base (DB-backed) portion.
    pub fn base(&self) -> BaseProjectSettings {
        BaseProjectSettings {
            worktree_directory: self.worktree_directory.clone(),
            default_branch: self.default_branch.clone(),
            base_remote: self.base_remote.clone(),
            push_remote: self.push_remote.clone(),
            github_account_id: self.github_account_id.clone(),
            tmux: self.tmux,
            auto_run_setup_script_on_task_creation: self.auto_run_setup_script_on_task_creation,
            auto_run_run_script_on_task_creation: self.auto_run_run_script_on_task_creation,
            task_startup_command: self.task_startup_command.clone(),
            workspace_provider: self.workspace_provider.clone(),
            feature_dossiers: self.feature_dossiers,
            feature_log_seeded_version: self.feature_log_seeded_version,
            step_chain_depth_cap: self.step_chain_depth_cap,
            step_budget_tokens: self.step_budget_tokens,
        }
    }

    /// The shareable portion.
    pub fn shareable(&self) -> ShareableProjectSettings {
        ShareableProjectSettings {
            preserve_patterns: self.preserve_patterns.clone(),
            shell_setup: self.shell_setup.clone(),
            scripts: self.scripts.clone(),
        }
    }

    /// Effective `baseRemote` (reference `getBaseRemote`).
    pub fn effective_base_remote(&self) -> &str {
        self.base_remote.as_deref().unwrap_or(DEFAULT_BASE_REMOTE)
    }

    /// Effective `pushRemote` (reference `getPushRemote`).
    pub fn effective_push_remote(&self) -> &str {
        self.push_remote
            .as_deref()
            .or(self.base_remote.as_deref())
            .unwrap_or(DEFAULT_BASE_REMOTE)
    }
}

/// Shape of a legacy (v1.1.15-era) `.emdash.json` config: base + shareable
/// fields plus the legacy `remote` field.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct LegacyProjectConfig {
    pub worktree_directory: Option<String>,
    pub remote: Option<String>,
    pub base_remote: Option<String>,
    pub push_remote: Option<String>,
    pub default_branch: Option<String>,
    pub tmux: Option<bool>,
    pub workspace_provider: Option<WorkspaceProvider>,
    pub preserve_patterns: Option<Vec<String>>,
    pub shell_setup: Option<String>,
    pub scripts: Option<Scripts>,
}
