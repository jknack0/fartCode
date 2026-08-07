# Phase 0 Cross-Cutting Checklists

> Formerly the Appendix of `tickets-phase0.md` (retired 2026-08-04 — work is now
> tracked as GitHub issues in `jknack0/fartCode`). Kept because it is process guidance,
> not ticket work.

## Restart-survival tests (required for E2-07, sanity for all)

1. Create task → quit app (kill process) → relaunch → task/conversation restored; agent resumes (or documented degradation for non-resume providers).
2. With tmux enabled: process survives quit; relaunch reattaches.
3. Editor buffers (E5-03, later phase) unaffected by Phase 0.

## Security review triggers

- PTY env allowlist changes (E2-06) — review against `packages/core/src/agents/agent-env.ts`.
- Shell quoting/escaping helpers — single shared module; no ad-hoc quoting.
- Worktree path validation (E2-02) — realpath containment, never remove project root.

## Telemetry events to emit (E15 later; stub the client now)

`app_started/app_closed (was_crash)`, `project_added/deleted`, `task_created/provisioned/status_changed/archived/deleted`, `conversation_created/deleted`, `agent_run_started/finished (provider, exit_code)`, `terminal_created/deleted`, `setting_changed`, `sidebar_toggled`, `error/$exception`.
