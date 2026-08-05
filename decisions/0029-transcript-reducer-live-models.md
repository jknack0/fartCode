# ADR-0029 — Transcript reducer + live models live in ade-acp; events bypass the internal bus

Status: accepted (issue #31)

## Context

E2-11-4 replaces E2-11-3's raw `session/update` streams per turn with the
reference's transcript reducer + live models: reduced turns (message /
thinking / tool-call lifecycle / plan items), session config, usage, title,
agents, plan — plus a typed event stream to the frontend and a raw-log
debug artifact.

Three layering questions carried over from ADR-0027:

1. Where do the reducer and live-model types live? They speak ACP wire
   types and only `SessionCell` consumes them.
2. How do updates reach the frontend without pulling Tauri into `ade-acp`?
3. Should ACP events flow over the internal `InternalEvent` bus like git
   and task events?

## Decision

1. **`ade-acp::transcript` owns the reducer** — pure
   `(ParserState, ReducerInput) → ParserState` fold (`reducer::reduce`),
   the stateful `TranscriptParser` wrapper, the `SessionUpdate →
   NormalizedEvent` decoder, id synthesis (reference string formats kept
   verbatim), and the serde-camelCase live-model structs. Same rationale as
   ADR-0027: `ade-acp` is the leaf crate for everything ACP; `ade-core`
   stays untouched.

2. **Event seams are a trait; Tauri is the app layer.** `ade-acp` defines
   `SessionEvents` (`update` / `transcript_changed` /
   `permission_requested`); `ade-app::acp_events::TauriAcpEvents`
   implements it over `AppHandle::emit`, producing `acp:update`,
   `acp:transcript`, `acp:permission_request` keyed by `conversationId`.
   The cell fires `transcript_changed` with a FULL `LiveModels` snapshot on
   every state change — the frontend replaces its store from it (diffing /
   throttling is #33's concern if Phase-2 traffic warrants it).

3. **ACP events bypass the internal bus**, matching the `terminal:output`
   precedent (also emitted directly from `ade-app`): ACP streams are
   high-frequency per-conversation traffic with no domain-service
   consumers; routing them through `BroadcastEventBus` would add a hop and
   a serialization without a buyer. `InternalEvent` keeps the lifecycle
   events it already carries.

4. **Scoped-down from the reference** (documented in module docs):
   - No `EnrichHook` in Phase 2, therefore no provider-enriched event
     kinds (`subagent`, `search`, `mcp_tool`, `web_fetch`,
     `subagent_update`) — the agent slice updates from baseline events
     only, and the terminal live models (`terminals` + live log) stay
     empty until the Phase-4 `terminal` client capability lands.
   - `ToolCallItem` is one struct with a `kind` discriminator instead of
     the reference's 11-way interface union; the serialized JSON is
     identical.
   - No 250ms quiescence timer (inherited from E2-11-3: v1 adapters stream
     inside turns).

5. **Raw log is in-memory** — `RawAcpLog` per conversation (50k-entry cap,
   append-only, oldest evicted), exported as pretty JSON via
   `SessionManager::export_raw_log`. Persistence to disk is deferred —
   the debug artifact requirement is "export", not "survive restart".

## Consequences

- `SessionCell` now owns the parser + raw log; the old raw `Turn.updates`
  stream is gone (E2-11-3's manager tests were migrated to reduced
  assertions; `prompt_text` is now the synthetic user-message item).
- `StartInput` gained `provider_id` and `events`; `update_sink` was
  removed (its only consumer was the test rig passing `None`).
- #32 wires `TauriAcpEvents` into the manager and adds the Tauri commands;
  #33 renders the snapshot; neither needs further `ade-acp` changes.
- Acceptance pinned by `tests/reducer_golden.rs` (six deterministic golden
  folds) and `tests/acp_events_integration.rs` (in-order event delivery +
  live-model snapshot + replay settlement + raw-log export against the
  fake adapter's `rich` behavior).
