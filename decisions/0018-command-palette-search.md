# ADR-0018 — Command palette + FTS search + resource monitor

Status: accepted (ticket E1-09)

## Context

E1-09 adds ⌘K (command palette over commands + projects/tasks/conversations),
an FTS-backed search index that stays current as rows change, and a resource
monitor (CPU/memory) disabled by default. The `search_index` FTS5 virtual
table (trigram tokenizer) already existed from E1-01.

## Decision

1. **`ade_core::search` owns the index**: `upsert`/`delete` use a
   deterministic rowid (`hash32(item_type:item_id)`) so upserts dedupe
   (FTS5 has no unique columns); `query` wraps the input in double quotes
   (trigram phrase matching) with quote-escapes; `backfill` repopulates from
   projects/tasks tables. Event-driven writes keep it current.
2. **Indexer runs in `ade-app`**: `spawn_search_indexer` backfills on boot,
   then subscribes to the event bus (project/task/conversation
   added/deleted → upsert/delete). Lagged events are dropped (bridge
   survives), matching the event forwarder.
3. **Resource monitor**: `sysinfo` (0.30 — `global_cpu_info().cpu_usage()`,
   memory in bytes → MB) behind a lazy `Mutex<Option<System>>` singleton so
   samples are cheap; gated by the new `resourceMonitor.enabled` setting
   (default false per the ticket).
4. **Frontend**: ⌘K overlay (commands registry + debounced FTS results,
   keyboard nav, enter-to-run); resource panel polls `resource_sample` every
   1s while open; the palette's "toggle resource monitor" flips the setting
   and opens the panel. The create-project dialog is driven via a shared ui
   store so both the palette and ⌘⇧N open it.

## Consequences

- The index is event-consistent (boot backfill + live updates) and the
  acceptance criteria are covered by core tests (upsert/query/delete/
  backfill, trigram substrings) + smoke.
- sysinfo is a new ade-core dependency (already in the workspace manifest).
- The palette's command registry is a static list today; E2-10 task-switch
  and E2-06 agent commands plug into the same registry later.
