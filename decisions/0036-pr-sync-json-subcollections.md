# ADR-0036: PR sync cache — one row per PR, versioned-JSON sub-collections

- **Status:** Accepted
- **Date:** 2026-08-07
- **Ticket:** E4-09 (#49)
- **Relates to:** ARCHITECTURE.md §2 (versioned JSON), §11; reference `0005_add_pull_requests.sql`

## Context

E4-09 persists fetched PR data so the Pull Requests tab renders instantly and
offline. The ticket phrases the storage as "`pull_requests` + sub-tables
(files/commits/checks/comments)". The reference normalizes four child tables
(labels, assignees, checks, users) and fetches the rest on demand. We had to
pick a shape for the four review sub-collections.

## Decision

One `pull_requests` row per PR URL. Scalar columns cover the query paths
(`workspace_id`, `owner`/`repo`/`head_ref` for the commit-card branch guard,
`status` for open-first ordering). The full denormalized `PrDto` — files,
commits, checks, comments included — rides in a single versioned-JSON `data`
column, the established §11 pattern for list-shaped JSON.

This deviates from the reference's normalized child tables.

## Consequences

- **Idempotent upserts are trivial:** deserialize the stored blob, compare,
  skip the write (and the `pr:updated` event) when byte-identical. No
  per-row churn, no cascade bookkeeping.
- **Reads are one row:** the tab and the `CachedPrLookup` guard never join.
- **Trade-off:** no SQL-level queries into a sub-collection (e.g. "all failed
  checks across PRs"). None are needed in Phase 1; if one appears, that is the
  signal to normalize that specific collection.
