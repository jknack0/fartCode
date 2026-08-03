# ADR-0008: Add Task operation (E2-04) — core service, frontend deferred

- **Status:** Accepted
- **Date:** 2026-08-03
- **Ticket:** E2-04
- **Relates to:** E2-01 (task model), E2-02 (worktrees), E2-03 (naming),
  E2-05 (conversations), E2-06 (agent launch)

## Context

E2-04's story is a dialog that starts a task from a branch with provider and
model chosen, producing a provisioned worktree + seeded conversation. The
reference (`createTask.ts`) splits the flow into `prepareCreateTask` →
`commitCreateTask` (tx inserts) → `finalizeCreateTask` (events), with a
separate `provisionWorkspace` step that ensures the worktree.

## Decision

- **Core operation in `ade-core::tasks::operations`** (`TaskCreationService`):
  a single `create()` that (1) validates project + inputs (typed `Error`s,
  never panics), (2) commits task + workspace + conversation rows through the
  extended `DbTaskStore::create`, (3) provisions the workspace via
  `WorktreeManager::ensure_worktree` (E2-02), pushes non-fatal, and emits
  `task:provisioned`. The dialog-facing branch picker is exposed as
  `list_branches(project_id)`.
- **`WorkspaceTarget` enum** (new-worktree | repository-instance | project-root
  | byoi) added to `ade-core::tasks`; `DbTaskStore::create` honors it
  (workspace row shape + id reuse), with the versioned workspace config
  (`{version:'2', git, workspace}`) stored on `workspaces.config`.
- **Conversation config builder**: `InitialConversationConfig::build_config()`
  produces the reference `{version:'1', type:'pty'|'acp', autoApprove?,
  initialPrompt?, model?, initialQueue?}` JSON; `model` persists into the
  conversation row (acceptance 2). The `conversations` module (E2-05) owns the
  schema; this is the producer.
- **Agent "launch" placeholder**: E2-06 owns the real agent launch; E2-04 emits
  `InternalEvent::AgentStart` when the task has a pty initial prompt, which
  E2-06 will consume. The `Conversation` model returned by the reference is
  deferred — we return the conversation id + config.
- **Trust**: `should_auto_trust(force)` ports
  `workspaceTrustService.shouldAutoTrust` (`tasks.autoTrustWorktrees`, default
  true; forced by auto-approve). The actual provider-config trust write is
  E2-06/providers; Phase 0 logs the decision.
- **Frontend dialog deferred**: the renderer half (name/branch/provider/model
  pickers, workspace target UI) is not implemented — the Phase-0 environment
  has no node toolchain to build/typecheck `app-frontend`, and every prior
  ticket shipped core-first. `app-frontend` E2-04 remains open for the dialog.
- **Phase-0 deviations from the reference**: no `project-not-found`-style
  open/closed project tracking (we validate the project row exists); the
  reference's `git.kind === 'none'` → repository-instance upgrade is replaced
  by an explicit `WorkspaceTarget::ProjectRoot`.

## Consequences

- The dialog can drive a fully-tested core operation; E2-06 consumes the
  `AgentStart` event to launch real agents.
- `DbTaskStore::create` remains the single tx commit path (rows), while
  `TaskCreationService` owns orchestration (validation + provision) — no
  divergence between direct store use and the operation.
- 9 integration tests + 4 unit tests cover the acceptance criteria; smoke
  section 14 exercises the flow end-to-end.
