//! Host dependency commands (E3-02 store → 7d "Agents on this machine"):
//! list agent CLIs with detection state, install/update them, and summarize
//! the registry tail (`+ N more in the registry · N acp`).
//!
//! Install/update run the blocking `ProcessInstallRunner` on the blocking
//! pool ([`off_main_thread`]) — an npm/curl install must never stall the main
//! thread. The runner buffers child output and flushes once at the end, so
//! there is NO incremental progress to report — the `installing · 62%` row
//! state has no backend feed until a PTY-backed runner lands (E2-06 seam);
//! callers get the settled DTO when the install finishes.

use std::sync::Arc;

use fartcode_core::settings::DEFAULT_AGENT;
use serde::Serialize;
use tauri::State;

use crate::app::App;
use crate::commands::off_main_thread;

/// One agent CLI row (7d). `latest` is only set when it is cheaply known —
/// the network update check is a Phase-0 stub, so it is currently always
/// `None` ("update available" affordances stay hidden).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostDependencyDto {
    pub provider_id: String,
    pub name: String,
    pub version: Option<String>,
    pub path: Option<String>,
    pub installed: bool,
    pub latest: Option<String>,
    /// The provider registry records ACP capability (the `22 acp` count).
    pub acp: bool,
    /// Resolvable from the `defaultAgent` app setting.
    pub is_default: bool,
}

/// Registry tail counts (7d: `+ 31 more in the registry · 22 acp`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyRegistrySummaryDto {
    pub total: usize,
    pub acp: usize,
}

fn dto_from_status(
    dep: &fartcode_core::dependencies::HostDependency,
    status: fartcode_core::dependencies::HostDependencyStatus,
    latest: Option<String>,
    default_agent: &str,
) -> HostDependencyDto {
    let acp = fartcode_providers::get(&dep.id)
        .map(|p| p.capabilities.acp)
        .unwrap_or(false);
    HostDependencyDto {
        provider_id: dep.id.clone(),
        name: dep.name.clone(),
        version: status.version,
        path: status.path,
        installed: status.installed,
        latest,
        acp,
        is_default: dep.id == default_agent,
    }
}

fn list_blocking(app: &App, refresh: bool) -> Result<Vec<HostDependencyDto>, String> {
    let default_agent = app.settings.get(&DEFAULT_AGENT).unwrap_or_default();
    let mut out = Vec::new();
    for dep in fartcode_core::dependencies::host_dependency::host_dependencies() {
        let status = if refresh {
            app.host_dependencies.detect(&dep.id)
        } else {
            app.host_dependencies.get_status(&dep.id)
        }
        .map_err(|e| e.to_string())?;
        let latest = app.host_dependencies.latest_version(&dep.id);
        out.push(dto_from_status(&dep, status, latest, &default_agent));
    }
    Ok(out)
}

/// Every registered agent CLI with its detection state. `refresh: false`
/// serves the kv cache (TTL 300s); `true` forces a re-detect (PATH scan +
/// version probe for installed binaries). Async: version probes spawn
/// subprocesses.
#[tauri::command]
pub async fn host_dependency_list(
    app: State<'_, Arc<App>>,
    refresh: bool,
) -> Result<Vec<HostDependencyDto>, String> {
    let app = app.inner().clone();
    off_main_thread(move || list_blocking(&app, refresh)).await
}

fn provider_dto(app: &App, provider_id: &str) -> Result<HostDependencyDto, String> {
    let dep = fartcode_core::dependencies::host_dependency::host_dependency(provider_id)
        .ok_or_else(|| format!("unknown provider: {provider_id}"))?;
    let status = app
        .host_dependencies
        .get_status(provider_id)
        .map_err(|e| e.to_string())?;
    let default_agent = app.settings.get(&DEFAULT_AGENT).unwrap_or_default();
    let latest = app.host_dependencies.latest_version(provider_id);
    Ok(dto_from_status(&dep, status, latest, &default_agent))
}

/// Installs the provider's CLI via its registry install plan (npm / curl /
/// brew / …) and returns the re-detected row. Installer output is logged
/// (`tracing`, target visible with RUST_LOG) — no live stream yet, see the
/// module docs.
#[tauri::command]
pub async fn host_dependency_install(
    app: State<'_, Arc<App>>,
    provider_id: String,
) -> Result<HostDependencyDto, String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let mut sink = |chunk: &str| {
            tracing::info!(provider_id = %provider_id, output = %chunk, "dependency install output");
        };
        app.host_dependencies
            .install(&provider_id, &mut sink)
            .map_err(|e| e.to_string())?;
        provider_dto(&app, &provider_id)
    })
    .await
}

/// Updates the provider's CLI (per-manager update command; curl installers
/// re-run idempotently) and returns the re-detected row.
#[tauri::command]
pub async fn host_dependency_update(
    app: State<'_, Arc<App>>,
    provider_id: String,
) -> Result<HostDependencyDto, String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let mut sink = |chunk: &str| {
            tracing::info!(provider_id = %provider_id, output = %chunk, "dependency update output");
        };
        app.host_dependencies
            .update(&provider_id, &mut sink)
            .map_err(|e| e.to_string())?;
        provider_dto(&app, &provider_id)
    })
    .await
}

/// Registry counts for the 7d tail line. Counted from the provider
/// registry — the set `install_metadata.json` annotates (same cardinality;
/// ACP capability only lives on the provider descriptors).
#[tauri::command]
pub fn host_dependency_registry_summary() -> DependencyRegistrySummaryDto {
    let providers = fartcode_providers::list();
    DependencyRegistrySummaryDto {
        total: providers.len(),
        acp: providers.iter().filter(|p| p.capabilities.acp).count(),
    }
}

#[cfg(test)]
mod tests {
    use super::host_dependency_registry_summary;

    #[test]
    fn registry_summary_matches_the_provider_registry() {
        let summary = host_dependency_registry_summary();
        // 35 providers, 22 ACP-capable — pinned by fartcode-providers tests;
        // this guards the DTO mapping, not the registry contents.
        assert_eq!(summary.total, fartcode_providers::list().len());
        assert_eq!(
            summary.acp,
            fartcode_providers::list()
                .iter()
                .filter(|p| p.capabilities.acp)
                .count()
        );
        assert!(summary.acp <= summary.total);
        assert!(summary.total > 0);
    }
}
