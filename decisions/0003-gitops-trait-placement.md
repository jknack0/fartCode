# ADR-0003: GitOps trait placement + CLI implementation

- **Status:** Accepted
- **Date:** 2026-08-03
- **Ticket:** E1-03
- **Relates to:** ARCHITECTURE.md §6.4 / §18 D8–D9

## Context

`fartcode-core::projects` needs git operations, and ARCHITECTURE §6.4 placed the
`GitOps` trait in `fartcode-git`. But the crate graph makes `fartcode-core` the leaf
("depends on nothing internal"), so `projects` cannot depend on `fartcode-git`.

## Decision

- **Deviation (§6.4):** the **`GitOps` trait lives in `fartcode-core::git`**;
  `fartcode-git` depends on `fartcode-core`, provides the implementation (`CliGit`), and
  re-exports the trait. This keeps the leaf rule intact and lets later
  `fartcode-core` domains (`workspaces`) use it too.
- Phase 0 uses the **`git` CLI** via `std::process::Command` with argument
  arrays — no shell, no quoting (AGENTS.md rule). git2 worktree lifecycle
  bindings land with E2-02.
- Base-ref resolution ports the reference `computeBaseRef` `normalize()` exactly:
  slash-containing branches stay bare (`feature/x`), plain branches get the
  remote prefix (`origin/main`), `://` refs are dropped; refinement derives the
  remote from the *detected* ref.
- **`remote_head` is local-only** (`git symbolic-ref`). The reference's
  `git remote show` fallback is a network call that can hang synchronously
  during `create_local`/`create_clone` — dropped in Phase 0.

## Consequences

- `fartcode-core` stays a leaf; the trait/impl split mirrors Db (trait in fartcode-core,
  impl where the machinery lives).
- CLI-arg-array git calls are safe (no shell interpolation) but slower than
  libgit2 — fine for project creation; E2-02 may hot-path worktree ops via git2.
- Slash branches keep their identity as base refs (gitflow-friendly).
- No network calls during project open (origin/HEAD from the clone, local
  candidates otherwise).
