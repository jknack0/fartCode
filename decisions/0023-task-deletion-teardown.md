# ADR-0023 — Task deletion / teardown (E2-09)

Status: accepted (ticket E2-09)

## Context

Deleting a task (⌘Backspace) must clean up processes, sessions, worktrees,
and view state — safely (never the project root, never a sibling's shared
worktree) and idempotently (double-delete, delete racing provision).

Reference: `deleteTask.ts` (order of operations), `task-lifecycle-utils.ts`
(project-root guard, `deleteWorkspaceIfUnused`, `workspaceHasRemainingTasks`,
fs-fallback removal, bounded prune), `task-session-manager.ts`
(teardown modes), `getProvisionedWorkspaceBranch`.

## Decision

1. **`ade-core::tasks::deletion::TaskDeletionService`** — the operation
   service wired once in `App` (ARCHITECTURE §7):
   `(db, tasks, conversations, projects, worktrees, sessions)`.
2. **Order of operations** (reference-faithful):
   fetch (vanished row → clean no-op) → workspace snapshot → session
   teardown (registry cancel + tmux kill) → view-state keys → rows
   (task + unused workspace + FK cascade + `task:deleted`) → worktree
   removal → branch deletion. Process/git failures are warnings — the
   rows are the contract (reference catches and proceeds).
3. **Cooperative session teardown** (`pty::sessions::SessionRegistry`):
   the launcher owns each `PtyHandle` on its own thread, so the registry
   never touches handles — it hands the launcher a cancel flag to poll
   and an exited flag to set (`SessionGuard` drop covers every exit path,
   including errors). `terminate` sets cancel, waits bounded (5s; the
   reference's 600s covers teardown SCRIPTS, which Phase 0 lacks), then
   removes the entry. Cancelled launches never respawn. Boot rehydration
   registers via the same registry (one `Arc` shared in `App::init`);
   the session key is the reference `make_pty_session_id(project, task,
   conversation)`, and `RehydrateTarget` gains `project_id` for it.
4. **Workspace-row rules** (`DbTaskStore::delete` hardened):
   `kind='project-root'` rows are never deleted (shared by every
   no-worktree task); siblings block row deletion; unused rows also drop
   their `workspace_file_index` entries (reference `deleteIndex`).
   Missing-row delete is `Ok(())` (reference early return) — the store is
   now safe for double-delete on its own.
5. **Worktree removal** (`WorktreeManager::remove_worktree` extended):
   returns `bool` (removed vs kept-for-siblings, reference
   `removeWorktreeIfUnused`), gains `force` — deletion is user-confirmed
   so the E2-07 dirty-check is bypassed on that path (non-deletion flows
   still refuse dirty worktrees), and prunes with
   `GitOps::worktree_prune_timed` (5s bound; reference
   `pruneGitWorktrees` timeout + non-interactive env) so a wedged git
   cannot hang teardown. Guards unchanged: never the project root, never
   outside the pool.
6. **Branch deletion gate** (reference parity): only when the worktree
   was actually removed AND `delete_branch` AND the provisioned branch
   came from a `create-branch` intent AND differs from its source.
   Safe delete (`git branch -d` — new `GitOps::branch_delete`), failures
   warn.
7. **Frontend**: `delete_task` Tauri command (options default to the
   reference `{deleteWorktree: true, deleteBranch: false}`); sidebar ⌘/
   Ctrl+Backspace on the selected task + a × button on task rows, both
   behind the confirmation dialog (generalized from the project one).
   The backend `task:deleted` event already re-syncs the tree.

## Consequences

- `teardown_sessions` covers conversation AND terminal leaf ids
  (reference `getTaskSessionLeafIds`); tmux kills are best-effort (no
  tmux server → warning, deletion continues).
- The registry is the Phase-0 subset of the reference registry —
  attach/resize/ring-buffer arrive with the interactive terminal UI.
- `Error::GitTimeout` + `SessionCancelled` added; `GitOps` grows
  `branch_delete` and `worktree_prune_timed` (CliGit implements,
  Git2Ops delegates).
- 6 integration tests pin all three acceptance criteria plus the
  project-root and branch gates; store-level idempotence and the
  registry contract have their own unit coverage.
