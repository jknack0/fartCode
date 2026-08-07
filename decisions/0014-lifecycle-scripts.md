# ADR-0014 — Lifecycle scripts + PTY abstraction location

Status: accepted (ticket E1-06)

## Context

E1-06 runs setup/run/teardown scripts for a task in a PTY with an env
contract (`FARTCODE_*`), status events, output tail, and session dedupe.
ARCHITECTURE §7 wires `pty_manager: Arc<dyn fartcode_terminal::PtyManager>`
wired in `fartcode-app`, implying the trait lives in `fartcode-terminal`. But the
lifecycle service lives in `fartcode-core` (ticket crate), and `fartcode-terminal`
depends on `fartcode-core` — putting the trait in `fartcode-terminal` would create a
dependency cycle.

## Decision

1. **Trait in `fartcode-core`**: `fartcode_core::terminals::pty::{PtyManager, PtyHandle}`
   — the same pattern as `GitOps` (trait in `fartcode-core`, impl in `fartcode-git`).
   `fartcode-terminal` depends on `fartcode-core` + `portable-pty` and provides
   `PortablePtyManager`. §7's field type becomes
   `Arc<dyn fartcode_core::terminals::pty::PtyManager>`; wiring location unchanged.
2. **Phase 0 execution model**: each lifecycle script runs in a *dedicated*
   shell PTY (spawned per run). The reference types into the agent's
   persistent terminal session; that rewiring lands with E2-06, which reuses
   the same `PtyManager` primitive. The input transform, exit semantics,
   statuses, tail cap, and dedupe are reference-identical.
3. **Reader-thread PTY handle**: portable-pty 0.9 has no non-blocking read on
   the master, so the handle spawns a background reader draining into a
   buffer; `try_read` drains the buffer. `wait_exit` polls
   `Child::try_wait()` against a deadline (portable-pty has no timed wait).
4. **Env contract is fartCode-specific** (not in the emdash reference): the ticket
   defines `FARTCODE_TASK_ID`, `FARTCODE_TASK_NAME` (slugified via
   `tasks::naming::sanitize_name`, fallback `task`), `FARTCODE_TASK_PATH`,
   `FARTCODE_ROOT_PATH`, `FARTCODE_DEFAULT_BRANCH` (default `main`), and
   `FARTCODE_PORT = 50000 + (hash32(portSeed) % 1000) * 10` (FNV-1a 32, no extra
   deps). Port seed = workspace path (fallback task id) so two tasks in the
   same project get different ports (acceptance 2).

## Consequences

- `fartcode-core` tests can exercise the full PTY path via a dev-dependency on
  `fartcode-terminal` (dev-dep cycles are allowed, unlike regular deps).
- Status events added to `InternalEvent`
  (`LifecycleScriptStatusChanged`, status ∈ running|succeeded|failed|stopped);
  timeout surfaces `LifecycleScriptTimeout`, failure surfaces
  `LifecycleScriptFailed` unless `continueOnFailure`.
- The terminal drawer (⌘J) consuming the output tail is frontend work that
  rides on E1-04's app wiring; the core captures the tail today.
