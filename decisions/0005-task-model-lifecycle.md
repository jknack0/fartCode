# ADR-0005: Task model + lifecycle (atomic create, no status allowlist)

- **Status:** Accepted
- **Date:** 2026-08-03
- **Ticket:** E2-01
- **Relates to:** ARCHITECTURE.md §18 D12–D14

## Context

Tasks must exist as durable rows with well-defined statuses and lifecycle
events the rest of the app (UI, telemetry, automations) can rely on. The
reference splits creation into prepare (async) → commit (one DB transaction).

## Decision

- **No status-transition allowlist.** The reference `updateTaskStatus` accepts
  any lifecycle status (`todo | in_progress | review | done | cancelled |
  backlog | duplicate | triage`); the only guards are same-status no-op and
  not-found. ARCHITECTURE.md §3's `InvalidStatusTransition` variant stays for a
  future state machine — nothing enforces one today.
- **Create is atomic:** task row + workspace row (kind `'worktree'`) + optional
  initial conversation row in a single transaction; any failure rolls back
  everything. Events (`task:created`, `conversation:created`) fire post-commit
  and are non-fatal.
- **Provision fast-path contract:** `provision(task_id)` is idempotent —
  re-provision re-fires `task:provisioned` and touches `last_interacted_at`
  (reference `provisionWorkspace` fast path). Real workspace/worktree bootstrap
  is E2-02; the event + idempotency contract land here.
- Delete is a **hard delete** (conversations/terminals cascade via FK);
  archive is the non-destructive alternative (`archived_at` set, restore
  clears). Session teardown in 'archive'/'terminate' modes lands with E2-05.
- `tasks.workspace_intent` is never written (legacy column — the reference
  reads it only as a fallback; intent lives in `workspaces.config`).

## Consequences

- Any status change is allowed — simple and flexible; a future state machine
  can be added without a migration (the `InvalidStatusTransition` variant is
  already reserved).
- Atomic create makes partial-task states impossible; the prepare/commit split
  the reference uses (for async worktree setup) is deferred until E2-02 needs
  async steps.
- The provision contract gives E2-05/E2-04 a stable surface to build on.
