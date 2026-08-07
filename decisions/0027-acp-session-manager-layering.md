# ADR-0027 — ACP SessionManager/SessionCell live in fartcode-acp behind a persistence trait

Status: accepted (issue #30)

## Context

E2-11-3 needs the session lifecycle from the reference's
`runtime/session-manager.ts` + `session/cell.ts`: one conversation per
cell (state machine, prompt queue, turn quiescence, permission broker),
one manager keyed by conversation id, provider session ids persisted in
`conversations.session_id` so restarts resume the same session.

Two layering questions:

1. Where do the types live? `fartcode-core` owns the conversations domain, but
   ARCHITECTURE.md's crate graph makes `fartcode-core` the leaf — and the
   session machinery speaks ACP wire types (`agent-client-protocol-schema`)
   and drives an `AcpClient`, which belong to `fartcode-acp`.
2. How does the manager persist session ids without `fartcode-acp` depending on
   `fartcode-core` (forbidden direction: `fartcode-acp` is a sibling stub crate, not
   below `fartcode-core`)?

## Decision

1. **`fartcode-acp::session` owns the runtime** — `SessionCell` + `SessionManager`
   live next to the client/transport they drive. No `fartcode-core` dependency is
   added to `fartcode-acp`.
2. **Persistence via a one-method trait** — `SessionIdStore::set_session_id`
   is defined in `fartcode-acp`; the app layer (#32) implements it over
   `DbConversationStore` (which already has the guarded single-UPDATE
   `set_session_id` from E2-05). Errors are logged, never fatal (launcher
   precedent, E2-06).
3. **One `AcpClient` per conversation** instead of the reference's pooled
   `AcpConnectionSource` keyed by `(provider, workspace)`: Phase 2 runs at
   most a handful of conversations per workspace, and the E2-11-2 process
   host already isolates one session per worker. The manager takes the
   client via `StartInput` (spawned by the caller — test rig now, process
   host at #32).
4. **The provider decision hook lives in `fartcode-core::conversations`** —
   `resolve_session_path(conversation)` returns `SessionPath::Acp` only when
   BOTH `config.type == acp` AND the registry reports
   `capabilities.acp`; every other shape falls back to the TUI/PTY path,
   leaving E2-06's launcher untouched (acceptance 3 regression-pinned).
5. **Scoped-down cell** (documented in module docs): no 250ms quiescence
   timer (v1 adapters stream inside turns), no background-agent counting,
   raw `session/update` streams per turn — the transcript reducer and live
   models replace that shape in E2-11-4.

## Consequences

- `fartcode-acp` tests run against the fake adapter with an in-memory
  `SessionIdStore`; the restart-resume acceptance test simulates "restart"
  as a fresh manager + fresh adapter process seeded with the persisted id.
- #32 must wire the real `SessionIdStore` adapter, the Tauri commands, and
  (later) swap the direct-client spawn for the E2-11-2 worker host.
- `conversations.session_id` semantics are unchanged from E2-05 (PTY rows
  carry the conversation id; ACP rows stay null until the manager
  persists the provider id).
