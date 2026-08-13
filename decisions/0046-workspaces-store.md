# ADR-0046: Workspaces store — one home for workspaces-table SQL

- **Status:** Accepted
- **Date:** 2026-08-12
- **Ticket:** Arch audit 2026-08-12, T2
- **Relates to:** ARCHITECTURE.md §2 (deviation: module shape + Row visibility)

## Context

ARCHITECTURE.md §2 promised `fartcode-core/src/workspaces/` (`mod.rs` +
`model.rs` with a `Workspace` struct) and it was never built. `FROM
workspaces` SQL was hand-rolled at 12+ sites across core and the app shell,
each restating the row conventions — most dangerously the
`COALESCE(location,'local')` default and the "non-empty stored path" filter,
which sites implemented inconsistently (some failed a whole pass on a NULL
path column instead of skipping).

## Decision

- **Single `mod.rs`, no `model.rs`, no `Workspace` struct.** The table is
  pure storage; there is no richer domain object to layer on top. As an
  explicit exception to §2's "Row types are private to the domain module",
  `WorkspaceRow` IS the domain's public read shape. §2 is updated to match.
- **Three access layers, narrowest visibility wins**: public
  `WorkspaceStore` over `Arc<dyn Db>` (one connection guard per call);
  `pub(crate)` fns over `&dyn Db` for core's free-function modules
  (`fs_watch`, `projects::adoption`, pool checks); `pub(crate)` fns over
  `&rusqlite::Connection` for transaction-composed and single-guard RMW
  sites. The outer two layers delegate to the `&Connection` fns, so every
  operation has exactly one SQL string.
- **The `location` default lives in one place**: `row_from`, the single row
  mapper every read goes through. `WorkspaceRow::local_path()` is the shared
  non-empty-path filter (existence checks stay with callers — they disagree
  about the fallback).
- **Specialized joins stay domain-local by design**: the BYOI join
  (`tasks::byoi`), the remote-target join (`projects::remote`), and the
  deletion snapshot (`tasks`) select different shapes and would not collapse
  onto this API.

## Consequences

- New workspaces reads/writes go through this module; a call site restating
  `COALESCE(location,'local')` or a bespoke path filter is a review flag.
- `tasks/mod.rs`'s create/delete sites were left on raw SQL (uncommitted
  user work in flight on that file); they port later onto
  `WorkspaceStore::insert()`/`delete()`.
- Behavior deltas are confined to previously-erroring corners: a NULL-path
  row during boot rehydrate or ACP cwd resolution now takes the documented
  skip/fallback path instead of failing the pass with a column-type error.
- `fs_watch` keeps thin delegates as the watcher's registration surface;
  `WatchTarget` and both target queries live here.
