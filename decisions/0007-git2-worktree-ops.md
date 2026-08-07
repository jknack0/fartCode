# ADR-0007: git2-backed worktree operations (Git2Ops)

- **Status:** Accepted
- **Date:** 2026-08-03
- **Ticket:** E2-02
- **Relates to:** ARCHITECTURE.md §6.4, AGENTS.md "Git strategy", ADR-0003

## Context

E2-02 introduces the git2-backed worktree implementation that AGENTS.md and
ADR-0003 deferred: `Git2Ops` for worktree lifecycle (`worktree()`,
`worktrees()`, `find_worktree()`, prune), with everything else still delegated
to the `CliGit` CLI implementation. git2 must behave like `git worktree`.

## Decision

- **`Git2Ops` in `fartcode-git`** implements the full `GitOps` trait; the five
  worktree methods use git2 directly, the remaining ~18 methods delegate to
  the embedded `CliGit` (per the ticket's "fall back to the CLI").
- **Mutex serialization:** `worktree_list`/`add`/`prune`/`remove` all take a
  `Mutex<()>`; `prune`'s body is factored into a lock-free `prune_locked()`
  so `remove` (which must lock, then delete the dir, then prune) cannot
  double-lock. `git2::Repository` is `!Sync` and never held across calls.
- **Main worktree synthesized:** libgit2's `worktrees()` yields *linked*
  worktrees only, while `git worktree list` and the reference include the
  main repo — `worktree_list` synthesizes the main entry (path/head/branch/
  bare) from the opened repository for parity.
- **Checkout after add:** `git worktree add` checks the branch out into the
  new worktree; libgit2 only creates metadata, so `worktree_add` runs a
  force checkout itself.
- **Add refuses already-checked-out branches** (`git worktree add` parity),
  and **prune skips locked worktrees** and surfaces prune failures instead of
  swallowing them (both match CLI behavior the reference relies on).
- **New trait methods** `config_get`/`config_set`/`push`/`is_tracked`
  (needed by E2-04 worktree reconcile + push): `push` maps exit-1 to
  `Ok(None)`-style "not pushed" only where `git push` actually means that;
  `config_get` maps *only* exit code 1 (key unset) to `None` — all other
  failures are errors.
- **`--` separators** added to `branch_create` / `is_tracked` so dash-leading
  names/paths are never parsed as options.

## Consequences

- Worktree entry parity between `Git2Ops` and `CliGit` (main + linked, branch
  as `refs/heads/*`).
- Tests: roundtrip add/list/remove, prune of stale metadata, prune skips
  locked worktrees, add refuses a checked-out branch.
- `ensure_worktree` (E2-04) can rely on add/prune semantics matching the CLI.
