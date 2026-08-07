# ADR-0004: Project open lifecycle + git exclusion

- **Status:** Accepted
- **Date:** 2026-08-03
- **Ticket:** E1-03
- **Relates to:** ARCHITECTURE.md §18 D10–D11

## Context

Opening a project must leave the repo ready to task: fartCode internals git-excluded,
settings seeded, worktrees re-detected. Closing must eventually tear down
sessions/workspaces/preview servers, but those subsystems don't exist yet.

## Decision

- `.fartCode/` is excluded from git via **`.git/info/exclude`** (local, per-repo —
  never a tracked `.gitignore`). In a linked worktree `--git-dir` resolves to
  the per-worktree dir, so the entry lands there — acceptable; the reference
  writes the common dir, E2-02 can align.
- `open_project` = seed settings → exclude `.fartCode/` → re-detect worktrees via
  `git worktree list` (close/open restores state) → ensure the repository
  workspace row (keyed `sha256("local:<path>")`, race-safe, reuse by key).
- `close_project` is a **Phase 0 stub**: session/workspace/preview teardown
  (tmux `detach` vs `terminate`) lands with E2-05/E2-02/E13; the hook point is
  documented on the function.
- GitHub "new repo" creation is stubbed behind `RepoHostProvider` (E8).

## Consequences

- Repos stay clean of `.fartCode/` without touching tracked files.
- Close/open is cheap and idempotent today; it gains teardown weight as
  subsystems land.
- The `RepoHostProvider` trait means the GitHub API call (E8) slots in without
  touching project creation.
