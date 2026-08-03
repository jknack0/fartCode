# ADR-0015 — App shell wiring + project teardown

Status: accepted (ticket E1-04)

## Context

E1-04 is the first ticket that touches the Tauri shell: the sidebar needs
projects/tasks commands, and the app must finally wire the domain services
per ARCHITECTURE §7. Deleting a project must tear down worktrees + rows.

## Decision

1. **App struct + managed state**: `ade-app::app::App` (db, settings,
   projects, tasks, conversations, event_bus as `Arc`s) built in `App::init`
   (db path from `ADE_DB_FILE`), managed via `app.manage(Arc<App>)`, and
   commands take `State<'_, Arc<App>>` (Tauri 2 keys state by exact `TypeId`
   — no Arc coercion; §7's sketch already used `Arc<dyn ...>` fields).
   `events`/`conversations`/`settings`/`db` are wired but unused until their
   tickets (marked `#[allow(dead_code)]`, kept alive by `App`).
2. **Event bridge**: `spawn_event_forwarder` forwards a whitelist of
   `InternalEvent`s to the frontend channel `ade:event` (serde JSON). The
   forwarder survives `RecvError::Lagged` (drops old events, keeps bridging)
   and only ends on `Closed`.
3. **Project delete teardown**: the project row delete happens in ONE
   transaction with the orphaned-workspace cleanup (workspaces have no FK to
   projects — the project's task worktrees + repository workspace are
   captured before the row vanishes, then deleted). After commit, the
   worktree pool dir is removed best-effort (`pool != project.path` guard —
   never the project root).
4. **Known limitation (documented)**: the worktree pool segment is the
   project NAME (reference parity, `safe_path_segment(name, id)`), so two
   same-named projects share a pool dir and deleting one removes the other's
   on-disk worktrees (rows of the survivor stay, pointing at deleted dirs).
   Fix deferred: switching the segment to the project id changes worktree
   paths for existing projects and needs a migration plan.
5. **Frontend**: zustand sidebar store; `visibleTaskOrder` is the E2-10
   task-switch contract (pinned-first, collapsed-skipped, archived-skipped);
   backend `project:deleted` events update the store locally (no API
   re-invoke); ⌘⇧N create-project, right-click pin/delete.

## Consequences

- The window now shows the real sidebar (projects → tasks, pinned section)
  and drives navigation; every command maps errors to `String` (no panics).
- Same-named projects remain a documented edge until the segment scheme
  changes (tracked with E2-xx worktree work).
- `ade-app` has unit tests for the event mapper; the cascade/teardown is
  covered by `worktrees_integration::delete_project_cascades_rows_and_tears_down_worktrees`.
