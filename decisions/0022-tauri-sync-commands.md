# ADR-0022 — Tauri command execution model (sync for Phase 0)

Status: accepted (post-E2-07 review follow-up)

## Context

ARCHITECTURE.md §4 says "Tauri commands must be synchronous" and mandates
`Handle::current().block_on(…)` for any async work. In Tauri 2, async
commands are fully supported via `#[tauri::command] async fn` — the premise
in §4 is incorrect. The mandated `block_on` pattern has two real costs:

- It **panics** if called from a runtime worker thread (no Tokio runtime
  context in the worker pool).
- It **blocks the main thread** during multi-second operations (git clone,
  agent launch, dependency install).

## Decision

We stay **synchronous for Phase 0** for reasons that are correct even if
§4's premise was wrong:

1. **All of ade-core is synchronous.** Domain methods return
   `Result<T, Error>` — no `async`, no `spawn_blocking`, no
   `tokio::spawn`. A Tauri async command would only add `spawn_blocking`
   wrappers that don't exist yet. The real async boundary is E2-06's
   agent launcher (blocking PTY) + E2-07's boot rehydration (background
   thread), both of which use `std::thread::spawn`, not tokio.
2. **The current sync commands complete in ≤5 ms.** list_projects,
   create_task, get_settings — these are SQLite reads/writes inside the
   same thread. Switching to async now adds ceremony with zero latency
   benefit.
3. **The agent-launch path does not go through Tauri commands at all.**
   `AgentLauncher::run()` blocks on the PTY; the app spawns it on a
   `std::thread`, and the boot rehydrator also uses `std::thread::spawn`.
   Neither hits the command boundary.

## What changes at Phase 2 / E2-08+

When the terminal UI arrives (E2-08) and Tauri commands need to **read PTY
output** or **wait for agent exit**, we'll:

1. Mark the long-running commands `async` and `spawn_blocking` the domain
   calls inside them.
2. Switch the launcher's thread to a `tokio::task::spawn_blocking` for
   consistency (the PTY reader thread stays `std::thread` — portable-pty is
   blocking-IO by design).
3. Update §4 of ARCHITECTURE.md to reflect the actual async boundary.

## Consequences

- Phase 0 Tauri commands remain sync. No panics from worker-thread context
  because no async command exists yet.
- The git-config reading in `list_projects` (the only git op on the command
  path) completes in <1ms — the "block the main thread" concern doesn't
  manifest until E2-06's longer operations arrive, and those don't use
  Tauri commands.
- The ADR-0022 note in ARCHITECTURE.md §18 replaces the incorrect §4
  premise with this decision.
