//! Typed setting keys, defaults, and schemas (ARCHITECTURE.md §2, §6.2).
//!
//! Ports the reference's `settings-registry.ts` + `project-settings.ts`
//! with the product rebrand applied (branch prefix `ade`, paths under
//! `~/ade/`, config file `.ade.json`).
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
            branch_prefix: "ade".into(),
            append_random_branch_suffix: true,
            tmux_by_default: false,
        }
    }
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
                .join("ade/repositories")
                .to_string_lossy()
                .into_owned(),
            default_worktree_directory: home.join("ade/worktrees").to_string_lossy().into_owned(),
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

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ResourceMonitorGroup {
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// Typed keys + defaults
// ---------------------------------------------------------------------------

pub const DEFAULT_AGENT_ID: &str = "claude";

pub static PROJECT: SettingKey<ProjectGroup> = SettingKey::new("project");
pub static TASKS: SettingKey<TaskGroup> = SettingKey::new("tasks");
pub static DEFAULT_AGENT: SettingKey<String> = SettingKey::new("defaultAgent");
pub static LOCAL_PROJECT: SettingKey<LocalProjectGroup> = SettingKey::new("localProject");
pub static TERMINAL: SettingKey<TerminalGroup> = SettingKey::new("terminal");
pub static NOTIFICATIONS: SettingKey<NotificationGroup> = SettingKey::new("notifications");
pub static BROWSER_PREVIEW: SettingKey<BrowserPreviewGroup> = SettingKey::new("browserPreview");
pub static RESOURCE_MONITOR: SettingKey<ResourceMonitorGroup> = SettingKey::new("resourceMonitor");

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
pub const PROJECT_CONFIG_FILE: &str = ".ade.json";

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
    pub workspace_provider: Option<WorkspaceProvider>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Scripts {
    pub setup: Option<String>,
    pub run: Option<String>,
    pub teardown: Option<String>,
}

/// The shareable subset, synced to `.ade.json` (shareable_project_settings_json).
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
/// (reference: `.emdash.json` filtered; we filter `.ade.json`).
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
    pub workspace_provider: Option<WorkspaceProvider>,
    // -- shareable (.ade.json-synced) --
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
            workspace_provider: self.workspace_provider.clone(),
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
