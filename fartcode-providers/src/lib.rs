//! Provider registry + capability descriptors (ticket E3-01).
//!
//! The single source of truth for the 35 coding-agent CLIs — what the rest
//! of the app queries for detection (E3-02), launch (E2-06), model selection
//! (E2-04), and feature gating. Ports the reference
//! `packages/plugins/src/agents/` registry + capability types.
//!
//! Phase 0 carries: metadata, the 12 capability flags, the prompt behavior
//! descriptor (E2-06's launch command builder), session resume flags, binary
//! names, default model, and auth env var names. The heavy plugin machinery
//! (ACP adapters, hook configs, MCP adapters, install runners, full model
//! option lists) arrives with E3-02..04 + Phase 2.
//!
//! **Adding a provider** (acceptance 3): one entry in `providers_data.rs`
//! (regenerated from the extraction JSONs) — no other code changes.

pub mod types;

mod providers_data;

use providers_data::PROVIDERS;

pub use types::{Capabilities, Capability, PromptDescriptor, PromptStrategy, ProviderDescriptor};

/// Lookup + filtering API (reference `createPluginRegistry`).
pub fn get(id: &str) -> Option<&ProviderDescriptor> {
    PROVIDERS.iter().find(|p| p.id == id)
}
pub fn list() -> &'static [ProviderDescriptor] {
    // LazyLock is 'static; the borrow is tied to the static.
    &PROVIDERS
}

pub fn filter_by_capability(capability: Capability) -> Vec<&'static ProviderDescriptor> {
    filter_by_capability_in(&PROVIDERS, capability)
}

fn filter_by_capability_in(
    providers: &[ProviderDescriptor],
    capability: Capability,
) -> Vec<&ProviderDescriptor> {
    providers
        .iter()
        .filter(|p| p.capabilities.has(capability))
        .collect()
}

/// `resolve_executable(name)`: matches the binary name (or the provider id).
pub fn resolve_executable(name: &str) -> Option<&'static ProviderDescriptor> {
    resolve_executable_in(&PROVIDERS, name)
}

fn resolve_executable_in<'a>(
    providers: &'a [ProviderDescriptor],
    name: &str,
) -> Option<&'a ProviderDescriptor> {
    providers
        .iter()
        .find(|p| p.id == name || p.binaries.iter().any(|b| b == name))
}

/// Renderer-facing DTO — typed JSON, no secrets (nothing here is secret).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderDto {
    pub id: String,
    pub name: String,
    pub description: String,
    pub website_url: Option<String>,
    pub capabilities: Vec<&'static str>,
    /// Model list for the picker: always starts with the "Default model"
    /// sentinel (reference renderer model selector).
    pub models: Vec<String>,
    pub default_model: Option<String>,
    pub binaries: Vec<String>,
    pub prompt_strategy: &'static str,
}

impl ProviderDto {
    pub fn from_descriptor(p: &ProviderDescriptor) -> Self {
        let mut capabilities: Vec<&'static str> = p
            .capabilities
            .iter()
            .into_iter()
            .filter(|(_, on)| *on)
            .map(|(name, _)| name)
            .collect();
        capabilities.sort_unstable();
        let mut models = vec!["Default model".to_string()];
        if let Some(m) = &p.default_model {
            models.push(m.clone());
        }
        ProviderDto {
            id: p.id.clone(),
            name: p.name.clone(),
            description: p.description.clone(),
            website_url: p.website_url.clone(),
            capabilities,
            models,
            default_model: p.default_model.clone(),
            binaries: p.binaries.clone(),
            prompt_strategy: match &p.prompt.strategy {
                PromptStrategy::Argv { .. } => "argv",
                PromptStrategy::StdinPipe => "stdin-pipe",
                PromptStrategy::Keystroke => "keystroke",
            },
        }
    }
}

/// All providers as renderer-facing DTOs.
pub fn list_dtos() -> Vec<ProviderDto> {
    PROVIDERS.iter().map(ProviderDto::from_descriptor).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_35_providers_registered() {
        assert_eq!(list().len(), 35, "the ticket requires exactly 35 providers");
        let ids: Vec<&str> = list().iter().map(|p| p.id.as_str()).collect();
        let expected = [
            "claude",
            "codex",
            "cursor",
            "copilot",
            "amp",
            "commandcode",
            "opencode",
            "grok",
            "devin",
            "qwen",
            "qoder",
            "droid",
            "antigravity",
            "auggie",
            "goose",
            "kimi",
            "kilocode",
            "kiro",
            "cline",
            "codebuddy",
            "continue",
            "codebuff",
            "freebuff",
            "mistral",
            "jules",
            "junie",
            "oh-my-pi",
            "pi",
            "letta",
            "autohand",
            "rovo",
            "charm",
            "hermes",
            "mimocode",
            "zero",
        ];
        for id in expected {
            assert!(ids.contains(&id), "missing provider: {id}");
        }
        let mut sorted = ids.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 35, "no duplicate ids");
    }

    #[test]
    fn get_claude_returns_correct_metadata_and_capabilities() {
        let claude = get("claude").expect("claude registered");
        assert_eq!(claude.name, "Claude Code");
        assert!(claude.website_url.as_deref().unwrap().contains("claude"));
        assert!(claude.capabilities.acp);
        assert!(claude.capabilities.auto_approve);
        assert!(claude.capabilities.models);
        assert!(claude.capabilities.sessions);
        assert!(
            claude.capabilities.trust,
            "claude is the trust-capable provider"
        );
        assert_eq!(claude.binaries, vec!["claude"]);
        assert_eq!(
            claude.prompt.strategy,
            PromptStrategy::Argv {
                flag: Some(String::new())
            }
        );
        assert_eq!(
            claude.prompt.auto_approve_flag.as_deref(),
            Some("--dangerously-skip-permissions")
        );
        assert_eq!(claude.prompt.resume_flag.as_deref(), Some("--resume"));
        assert_eq!(claude.prompt.model_flag.as_deref(), Some("--model"));
        assert!(claude.env_vars.iter().any(|v| v == "ANTHROPIC_API_KEY"));
    }

    #[test]
    fn capability_filter_returns_22_acp_providers() {
        let acp = filter_by_capability(Capability::Acp);
        assert_eq!(acp.len(), 22, "the ticket requires exactly 22 ACP-capable");
        let ids: Vec<&str> = acp.iter().map(|p| p.id.as_str()).collect();
        for id in ["claude", "codex", "cursor", "copilot", "junie", "mimocode"] {
            assert!(ids.contains(&id), "expected {id} ACP-capable");
        }
        assert!(!ids.contains(&"rovo"), "rovo is not ACP-capable");
        assert!(!ids.contains(&"jules"), "jules is not ACP-capable");
    }

    #[test]
    fn capability_filters_for_other_flags() {
        // Every provider declares a sessions descriptor (resumable OR
        // stateless); resume-ability is expressed via prompt resume flags.
        assert_eq!(filter_by_capability(Capability::Sessions).len(), 35);
        assert_eq!(
            filter_by_capability(Capability::Models).len(),
            5,
            "selectable models"
        );
        assert_eq!(
            filter_by_capability(Capability::Trust).len(),
            3,
            "claude, copilot, cursor"
        );
    }

    #[test]
    fn resolve_executable_matches_binary_names() {
        assert_eq!(resolve_executable("claude").unwrap().id, "claude");
        assert_eq!(resolve_executable("cmdc").unwrap().id, "commandcode");
        assert_eq!(resolve_executable("kilo").unwrap().id, "kilocode");
        assert_eq!(resolve_executable("crush").unwrap().id, "charm");
        assert_eq!(resolve_executable("agy").unwrap().id, "antigravity");
        assert!(resolve_executable("not-a-real-agent").is_none());
    }

    #[test]
    fn every_provider_has_at_least_one_binary() {
        // Reference npmDependency defaults binaryNames to [id]; E3-02's
        // detection relies on every provider resolving to a binary.
        // (charm is a legitimate exception to "contains the id" — its
        // hostDependency names the `crush` binary.)
        for p in list() {
            assert!(
                !p.binaries.is_empty(),
                "provider '{}' must declare a binary",
                p.id
            );
        }
    }

    #[test]
    fn dto_has_default_model_sentinel_and_no_secrets() {
        let dtos = list_dtos();
        assert_eq!(dtos.len(), 35);
        for dto in &dtos {
            assert_eq!(
                dto.models.first().map(String::as_str),
                Some("Default model")
            );
            assert!(
                dto.models.len() <= 2,
                "Phase 0: sentinel + optional default"
            );
        }
        let claude = dtos.iter().find(|d| d.id == "claude").unwrap();
        assert!(claude.capabilities.contains(&"acp"));
        assert_eq!(claude.prompt_strategy, "argv");
        if let Some(default) = &claude.default_model {
            assert!(claude.models.iter().any(|m| m == default));
        }
    }

    #[test]
    fn prompt_strategies_span_argv_keystroke_and_stdin_pipe() {
        assert_eq!(
            get("claude").unwrap().prompt.strategy,
            PromptStrategy::Argv {
                flag: Some(String::new())
            }
        );
        assert_eq!(
            get("amp").unwrap().prompt.strategy,
            PromptStrategy::StdinPipe
        );
        assert_eq!(
            get("hermes").unwrap().prompt.strategy,
            PromptStrategy::Keystroke
        );
        assert_eq!(
            get("kimi").unwrap().prompt.strategy,
            PromptStrategy::Keystroke
        );
        assert_eq!(
            get("zero").unwrap().prompt.strategy,
            PromptStrategy::Keystroke
        );
        // devin uses a positional prompt (flag "--").
        assert_eq!(
            get("devin").unwrap().prompt.strategy,
            PromptStrategy::Argv {
                flag: Some("--".into())
            }
        );
    }
}
