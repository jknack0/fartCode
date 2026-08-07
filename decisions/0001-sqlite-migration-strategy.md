# ADR-0001: SQLite init + migration runner strategy

- **Status:** Accepted
- **Date:** 2026-08-03
- **Ticket:** E1-01
- **Relates to:** ARCHITECTURE.md §18 D1–D3

## Context

The app needs a consistent local database on first run and every upgrade,
without losing data or corrupting JSON blobs. The reference (Emdash) uses
Drizzle migrations applied through a timestamped journal (`__drizzle_migrations`)
with sha256 recorded but never re-verified.

## Decision

- Migrations are **embedded at build** (`include_str!`, journal JSON + numbered
  SQL files) and applied by a runner that tracks progress via
  `MAX(migrations.created_at)` and splits each file on `--> statement-breakpoint`.
- **Stricter than the reference:** every already-applied migration is
  **hash-verified on every init** (sha256 of the embedded SQL vs the stored
  hash). Hand-editing a numbered migration after apply fails loudly instead of
  silently diverging — "hand-edit of a numbered migration is not possible by
  design".
- **FTS tables live outside migrations**, version-gated via `kv` keys
  (`fts_version='3'`, `file_index_version='4'`) exactly as later tickets read
  them; a gate bump drops and rebuilds.
- Legacy reference DBs (`emdash4.db`/`emdash3.db`) are copied to `fartCode.db` via
  `VACUUM INTO` with `app_secrets` cleared — but a copied reference DB is *not*
  schema-identical to the Phase 0 schema, so init fails loudly rather than
  corrupting; the real data-migration path is a later-phase concern.

## Consequences

- Safer upgrade path (tamper detection) at the cost of a hash check per init.
- New migrations are cheap to add (numbered file + journal entry + one match
  arm) but require a build — migrations can't be shipped as data.
- The FTS gates are the contract for E1-08/E14 search tickets.
