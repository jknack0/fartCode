# ADR-0017 — View state + onboarding + single-instance

Status: accepted (ticket E1-08)

## Context

E1-08 needs: per-view UI state that survives restarts (sidebar collapse,
selection), boot-time pruning of stale view-state rows, a skip-able
onboarding flow (offline-OK), and a single-instance lock so a second
launch focuses the existing window instead of opening a new one.

## Decision

1. **View state lives in the `kv` table** under the `view-state:` namespace
   (`view-state:<scope>:<id>` — `task:*`, `task:*:tabs`, `project:*`, plus
   app-level keys like `view-state:app:sidebar`), matching the reference
   `view-state-service.ts`. `ade_core::view_state::{save,get,delete}`
   enforce the prefix; `prune_orphans()` is the reference's three DELETE
   statements (orphaned task/project/tabs rows) and runs once on app boot
   (non-fatal on error).
2. **Commands**: `get_view_state` / `set_view_state` — the frontend persists
   the sidebar collapse map + selection on every change and restores it on
   load (invalid selections fall back to the first project).
3. **Onboarding**: a modal flow (welcome → add project → optional agent →
   optional sign-in), every step skip-able, completion recorded in
   `view-state:app:onboarding`. Agent installs and GitHub sign-in are Phase-0
   stubs ("later") — offline-OK per the ticket.
4. **Single instance**: `tauri-plugin-single-instance`; the second launch
   focuses the `main` window. No second window.

## Consequences

- Layout restores after restart; stale rows are pruned at boot.
- The onboarding flag persists; the flow shows once.
- `view-state` is generic — the E1-09 palette and task tabs reuse it.
