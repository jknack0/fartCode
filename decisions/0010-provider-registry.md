# ADR-0010: Provider registry + capability descriptors (E3-01)

- **Status:** Accepted
- **Date:** 2026-08-03
- **Ticket:** E3-01
- **Relates to:** E2-04 (model picker), E2-06 (agent launch), E3-02 (host
  dependency detection), E3-03 (prompt delivery)

## Context

E3-01 is the single source of truth for the 35 coding-agent CLIs: metadata,
capability flags, and behavioral descriptors the rest of the app queries for
detection, launch, model selection, and feature gating. The reference keeps
one plugin file per provider (`packages/plugins/src/agents/impl/*/index.ts`)
with deep behavior configs (ACP adapters, hook configs, MCP adapters,
install runners, full model option lists).

## Decision

- **`fartcode-providers` crate** (pure data — only `serde` for the DTO; no
  dependency on `fartcode-core`, respecting the leaf rule). Empty scaffold was
  filled with `types.rs` (pure types), `lib.rs` (registry API + DTO + tests),
  and `providers_data.rs` (the 35-entry table).
- **Data extraction**: 5 parallel read-only subagents extracted each
  provider's fields from the reference impl files into strict JSON under
  `fartcode-providers/extracted/t1..t5/`; `fartcode-providers/tools/generate_providers.py`
  regenerates `providers_data.rs` from them. The extraction JSONs + generator
  are committed as the data source of record (like E2-03's word lists), so
  **adding a provider = one JSON + regenerate** (acceptance 3).
- **Types**: `Capabilities` = the 12 flags (`acp auth autoApprove effort
  hooks hostDependency mcp models plugins prompt sessions trust`), kind →
  bool; `PromptDescriptor` (strategy argv|stdin-pipe|keystroke + the
  `buildStandardCommand` flags: autoApprove/initialPrompt/resume/sessionId/
  model, `sessionIdOnResumeOnly`, `resumeWithoutSession`, defaultArgs);
  `ProviderDescriptor` (metadata + capabilities + prompt + binaries +
  default_model + env_vars).
- **Phase-0 scope cut**: full model option lists, auth method configs, hook
  configs, MCP adapters, install runners, and ACP transform machinery are
  Phase 2 / E3-02..04 consumers — the registry carries the *descriptors*
  they need (`default_model`, env var names, binaries, prompt flags).
- **`sessions` semantics**: every provider declares a sessions descriptor
  (`resumable` 26 | `stateless` 9) — the capability filter returns 35;
  resume-ability is expressed via the prompt `resume_flag` /
  `sessionIdOnResumeOnly` descriptors (E2-06 consumes those).
- **Registry API**: `get(id)`, `list()`, `filter_by_capability(cap)`,
  `resolve_executable(name)` (binary-name OR id match), `list_dtos()`.
- **DTO**: `ProviderDto` (serde) with capability name list + `models`
  starting with the **"Default model" sentinel** (reference renderer model
  selector). No secrets exist in the data; the DTO omits nothing sensitive.
- **Static storage**: the 35 entries live in a `LazyLock<Vec<ProviderDescriptor>>`
  (`.to_string()` calls aren't const, so a plain `static` table isn't
  possible); API returns borrows tied to the static.

## Consequences

- E2-06 builds launch commands from `PromptDescriptor`; E3-02 detects
  installs via `binaries`; E2-04's model picker gets `models` + the sentinel;
  feature gating uses `filter_by_capability`.
- The 22-ACP count and the 35-provider set are asserted in tests (verbatim
  id list), so a regeneration drift fails CI.
- 7 unit tests; smoke section 16 exercises list/get/filter/resolve/DTO.
