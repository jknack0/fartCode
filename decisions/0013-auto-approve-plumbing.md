# ADR-0013 — Auto-approve flag plumbing

Status: accepted (ticket E3-04)

## Context

Each provider gates "run without permission prompts" differently. E3-01
captured an `autoApprove` capability flag and an `autoApproveFlag` argv
string per provider, but nothing consumed them end to end, and two providers
(mimocode, opencode) gate auto-approve via env vars (`MIMOCODE_PERMISSION`,
`OPENCODE_PERMISSION` — reference `extraEnv`), which the argv-only model
silently dropped.

## Decision

1. `build_command` adds the auto-approve mechanism only when
   `ctx.auto_approve` **and** `provider.capabilities.auto_approve` are both
   true (capability-gated, per the reference), honoring
   `omitAutoApproveOnResume` (kimi).
2. `PromptDescriptor` gained `auto_approve_env: Option<Vec<(String, String)>>`
   for env-gated providers; `build_command` merges it into `AgentCommand.env`.
   Exactly one mechanism per provider is enforced by a registry test
   (`has_flag XOR has_env`), so auto-approve can never silently drop.
3. `resolve_auto_approve(conversation_auto_approve, auto_approve_by_default,
   auto_trust_worktrees)` computes `ctx.auto_approve`: trust gating
   (`autoTrustWorktrees`, default true → implicitly on), the conversation's
   own toggle, and the `tasks.autoApproveByDefault` force — OR'd, matching the
   ticket's "trusted worktree ⇒ auto-approve on" rule. The workspace-trust
   write itself stays E2-04's `should_auto_trust` (E2-06 consumer).

## Consequences

- Env-gated providers (mimocode, opencode) now actually honor auto-approve.
- The three-way resolver gives E2-06 one pure function to feed the launcher.
- Regeneration keeps flag/env in sync via the XOR invariant test.
