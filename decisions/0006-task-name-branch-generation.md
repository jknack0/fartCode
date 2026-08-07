# ADR-0006: Task name + branch generation

- **Status:** Accepted
- **Date:** 2026-08-03
- **Ticket:** E2-03
- **Relates to:** ARCHITECTURE.md §18 D15–D16

## Context

New tasks get human-friendly unique names and safe branch names automatically.
The reference delegates to two npm packages (`human-id@4.2.0` for random names,
`nbranch@0.1.1` for title slugs) whose sources are not in the reference repo.

## Decision

- **Random names: vendored word lists + direct algorithm.** The reference
  `humanId({ separator: '-', capitalize: false })` produces
  `adjective-noun-verb` (default `adjectiveCount: 1, addAdverb: false`). The
  real `human-id@4.2.0` word lists (200 adjectives / 300 nouns / 250 verbs)
  were fetched from npm and embedded; the combination order matches the
  package exactly.
- **Title slugs implemented directly** (reference `nbranch.generateBranchName`
  with `addRandomSuffix: false`): lowercase, non-alnum → `-`, collapse/trim,
  cap 64 — no npm package needed; `sanitize_name` covers both.
- **Random suffix: 5 base36 `[0-9a-z]` chars** derived from `uuid::Uuid::v4`
  entropy (matches the reference's `Math.random().toString(36).slice(2, 7)`
  format) — no `rand` dependency.
- **Branch resolution is pure and faithful** (reference `resolveTaskBranchName`):
  Linear issue branch names verbatim (no prefix/suffix); Linear without
  `branchName` suppresses the suffix; otherwise `raw-suffix` then
  `prefix/branch`; `normalize_branch_prefix` trims + strips `/`.
- **Pure functions; settings are read by the caller** (E2-04 wires
  `project.branchPrefix='fartCode'`, `project.appendRandomBranchSuffix=true`,
  `tasks.autoGenerateName=true` from E1-02).

## Consequences

- No runtime npm/network dependency; names are deterministic-shaped and
  shell-safe (`[a-z0-9-]`, ≤64).
- Branch names match `fartCode/<name>-<suffix>` with the suffix toggle honored.
- If the exact reference distribution ever matters, the word lists can be
  swapped for the npm tarball contents — they're already verbatim copies.
