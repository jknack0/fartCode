# ADR-0019 — Agent env allowlist

Status: accepted (ticket E3-08)

## Context

Agent processes must inherit only the env vars they need — never secrets,
never app internals, never shell cruft. E3-08 makes the allowlist a single,
security-reviewed source of truth.

## Decision

1. **One module, one file** (`ade_core::pty::env_allowlist`): the canonical
   lists are `BASE_AGENT_ENV` (forced TERM/COLORTERM/TERM_PROGRAM),
   `BASE_PASSTHROUGH`, `GLOBAL_AGENT_ENV_VARS`, `DISPLAY_ENV_VARS`,
   `AGENT_ENV_VARS` (~95 keys exact per `packages/core/src/agents/agent-env.ts`
   plus the lowercase proxy variants the ticket requires),
   `WINDOWS_ESSENTIAL` (applied only on Windows), and `HOOK_ENV`
   (`ADE_HOOK_*`). Adding a var = touching one list + PR review; the
   `adding_a_var_requires_one_file` test asserts every builder-emitted key
   is reachable via `is_allowlisted`.
2. **`build_agent_env`** (reference `pty-env.ts::buildAgentEnv`): base env →
   allowlisted pass-throughs from the process env → `SSH_AUTH_SOCK`
   (injected via `detect_ssh_auth_sock` when missing — env → macOS
   `launchctl getenv` → common socket locations) → provider vars (E3-01
   registry `env_vars`) → task env (`ADE_*` from E1-06) → hook env.
   The task env overrides provider vars; hook env wins last.
3. **Security posture**: the builder only ever copies from the process env
   keys on the allowlists; `SECRET_TOKEN`-style vars can't reach the agent
   (test). Provider vars are the deliberate E3-01 extension point (the
   registry's `env_vars` are themselves extracted from the reference, not
   free-form).

## Consequences

- `PATHEXT` is present on Windows, absent on macOS/Linux (test).
- E2-06's launcher builds the agent env from this module + the provider
  registry + task env; no other env source is allowed.
- The Windows essential set carries reference defaults when the process env
  lacks them.
