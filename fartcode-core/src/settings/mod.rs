//! Settings: typed app settings + project settings with layered precedence
//! (ARCHITECTURE.md §2, §6.2; ticket E1-02).

pub mod kv;
pub mod registry;
pub mod service;
pub mod worktree_directory;

pub use kv::KvStore;
pub use registry::{
    BaseProjectSettings, BrowserPreviewGroup, DefaultBranch, LocalProjectGroup, NotificationGroup,
    ProjectGroup, ProjectSettings, ResourceMonitorGroup, Scripts, SettingKey, SettingValue,
    ShareableProjectSettings, TaskGroup, TerminalGroup, TerminalShell, WorkspaceProvider,
    BROWSER_PREVIEW, DEFAULT_AGENT, DEFAULT_AGENT_ID, DEFAULT_BASE_REMOTE, DEFAULT_BRANCH_FALLBACK,
    DEFAULT_PRESERVE_PATTERNS, DEFAULT_STEP_CHAIN_DEPTH_CAP, LEGACY_CONFIG_FILE, LOCAL_PROJECT,
    NOTIFICATIONS, PROJECT, PROJECT_CONFIG_FILE, RESOURCE_MONITOR, TASKS, TERMINAL,
};
pub use service::{DbSettingsStore, SettingsStore};
