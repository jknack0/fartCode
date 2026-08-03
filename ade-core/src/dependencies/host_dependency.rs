//! Host dependency model (ticket E3-02).
//!
//! `HostDependency` merges the provider's executable names (from
//! `ade-providers`) with per-provider install metadata (from the reference
//! `hostDependency` descriptors, kept in `install_metadata.json`).

use std::sync::LazyLock;

/// How the agent CLI is installed (reference dependency-managers / install
/// method kinds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallType {
    Npm,
    Pip,
    Cargo,
    Brew,
    Go,
    Curl,
    GhRelease,
    Other,
}

impl InstallType {
    pub fn parse(s: &str) -> Option<InstallType> {
        match s {
            "npm" => Some(InstallType::Npm),
            "pip" => Some(InstallType::Pip),
            "cargo" => Some(InstallType::Cargo),
            "brew" => Some(InstallType::Brew),
            "go" => Some(InstallType::Go),
            "curl" => Some(InstallType::Curl),
            "gh-release" => Some(InstallType::GhRelease),
            _ => None,
        }
    }
}

/// Install/update/uninstall metadata for one provider's CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    pub install_type: InstallType,
    /// npm package name, or the shell command pieces for curl/other.
    pub install_args: Vec<String>,
    /// Full shell command for curl-style installs (run via the shell).
    pub shell_command: Option<String>,
    pub uninstall_command: Option<String>,
    /// GitHub `owner/repo` for update checks (None = npm registry / unknown).
    pub update_repo: Option<String>,
}

/// The agent CLI dependency for a provider.
#[derive(Debug, Clone)]
pub struct HostDependency {
    /// Provider id (also the dependency id).
    pub id: String,
    pub name: String,
    /// Executable names searched on PATH (reference `detect_paths`).
    pub detect_paths: Vec<String>,
    /// How to probe the version once the binary is found.
    pub version_command: Vec<String>,
    pub min_version: Option<String>,
    pub install_plan: Option<InstallPlan>,
}

/// Detection result (stored in kv under `host-dep:<id>`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyStatus {
    pub provider_id: String,
    pub installed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    pub checked_at: Option<String>,
    /// Human-readable install instructions when not installed.
    pub install_instructions: Option<String>,
}

impl DependencyStatus {
    pub fn not_installed(provider_id: &str, install_instructions: Option<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            installed: false,
            version: None,
            path: None,
            checked_at: None,
            install_instructions,
        }
    }
}

/// Cached-status TTL: re-detect when the entry is older than this.
pub const DETECTION_TTL_SECS: i64 = 300;

/// Detection cache key prefix (reference KV `host-dep`).
pub const HOST_DEP_KV_PREFIX: &str = "host-dep:";

fn raw_metadata() -> &'static serde_json::Value {
    static RAW: LazyLock<serde_json::Value> = LazyLock::new(|| {
        serde_json::from_str(include_str!("install_metadata.json"))
            .expect("embedded install_metadata.json is valid")
    });
    &RAW
}

/// The install plan for a provider, from the embedded metadata.
fn install_plan(provider_id: &str) -> Option<InstallPlan> {
    let entry = raw_metadata().get(provider_id)?;
    let install_type = entry
        .get("install_type")
        .and_then(|t| t.as_str())
        .and_then(InstallType::parse)?;
    let install_args = entry
        .get("package")
        .and_then(|p| p.as_str())
        .map(|p| vec![p.to_string()])
        .unwrap_or_default();
    let shell_command = entry
        .get("command")
        .and_then(|c| c.as_str())
        .map(String::from);
    let uninstall_command = entry
        .get("uninstall_command")
        .and_then(|c| c.as_str())
        .map(String::from);
    let update_repo = entry
        .get("update_repo")
        .and_then(|r| r.as_str())
        .map(String::from);
    Some(InstallPlan {
        install_type,
        install_args,
        shell_command,
        uninstall_command,
        update_repo,
    })
}

/// Every agent dependency, merged from `ade-providers` + install metadata.
pub fn host_dependencies() -> Vec<HostDependency> {
    ade_providers::list()
        .iter()
        .map(|p| HostDependency {
            id: p.id.clone(),
            name: p.name.clone(),
            detect_paths: p.binaries.clone(),
            version_command: p
                .binaries
                .first()
                .map(|b| vec![b.clone(), "--version".into()])
                .unwrap_or_default(),
            min_version: None,
            install_plan: install_plan(&p.id),
        })
        .collect()
}

pub fn host_dependency(id: &str) -> Option<HostDependency> {
    host_dependencies().into_iter().find(|d| d.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_35_providers_have_dependencies() {
        let deps = host_dependencies();
        assert_eq!(deps.len(), 35);
        for d in &deps {
            assert!(!d.detect_paths.is_empty(), "{}", d.id);
        }
    }

    #[test]
    fn claude_has_curl_plan_and_codex_npm_plan() {
        let claude = host_dependency("claude").unwrap();
        assert_eq!(claude.detect_paths, vec!["claude"]);
        let plan = claude.install_plan.unwrap();
        assert_eq!(plan.install_type, InstallType::Curl);
        assert!(plan.shell_command.unwrap().contains("claude.ai/install.sh"));
        assert_eq!(plan.update_repo.as_deref(), Some("anthropics/claude-code"));

        let codex = host_dependency("codex").unwrap();
        let plan = codex.install_plan.unwrap();
        assert_eq!(plan.install_type, InstallType::Npm);
        assert_eq!(plan.install_args, vec!["@openai/codex"]);
        assert!(plan.shell_command.is_none());
    }

    #[test]
    fn providers_without_installers_have_no_plan() {
        // rovo has no installer (auth-based SaaS) — no install plan.
        let rovo = host_dependency("rovo").unwrap();
        assert_eq!(rovo.install_plan, None);
    }

    #[test]
    fn every_declared_install_type_in_metadata_is_real() {
        // Guards against a typo'd install_type silently disabling installers
        // when install_metadata.json is regenerated.
        let raw: serde_json::Value =
            serde_json::from_str(include_str!("install_metadata.json")).unwrap();
        for (id, entry) in raw.as_object().unwrap() {
            if let Some(kind) = entry.get("install_type").and_then(|t| t.as_str()) {
                assert!(
                    InstallType::parse(kind).is_some(),
                    "metadata for '{id}' has unknown install_type '{kind}'"
                );
            }
        }
    }
}
