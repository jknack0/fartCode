# ADR-0020 — Agent launcher (E2-06)

Status: accepted (ticket E2-06)

## Context

Launching an agent CLI inside a PTY in its task worktree, with the right
env, prompt delivery, and respawn behavior — the bridge from the domain
layer to a live agent process.

## Decision

1. **`fartcode_core::pty::launcher::AgentLauncher`** owns the flow: resolve binary
   → build command (E3-03 `build_command_with_spill`, session args from
   E2-05) → env (E3-08 allowlist) → spawn → prompt delivery → wait → events →
   respawn.
2. **Env policy (the security core)**: `PtyManager::spawn` gained
   `EnvPolicy` — `AllowlistedOnly` calls portable-pty's `env_clear()` so the
   agent child sees ONLY the allowlist output (without this the parent env —
   secrets included — leaks through, which the E2-06 integration test
   caught). Lifecycle scripts keep `Inherit` (reference parity: they run the
   user's shell env + `FARTCODE_*`).
3. **Respawn**: `MAX_RESPAWNS = 2` respawns AFTER the initial launch (3
   spawns total), 500 ms delay, gated on `respawn_resume` (supervisor
   decision) and disabled under tmux; events `AgentRunStarted` /
   `AgentRunFinished` / `AgentSessionExited` per reference.
4. **Wait is open-ended**: `AGENT_WAIT_TIMEOUT` (24 h) — agent sessions run
   until the CLI exits; interactive close/terminate lands with the E2
   terminal UI.
5. **Binary resolution**: host-dependency detection path (E3-02) first, then
   the registry's `binaries` on PATH; missing → typed `AgentNotFound`.
6. **Spill cleanup**: the E3-03 large-prompt temp file is removed after the
   launch (accepted: the last spawn's spill wins if a respawn respilled).

## Consequences

- Agent children are hermetic: allowlist + task env + hook env only.
- The launcher is blocking; the app layer spawns it on a thread (same model
  as lifecycle scripts).
- `PtyHandle::resize` (cols ≥ 2, rows ≥ 1) added for the terminal UI later.
