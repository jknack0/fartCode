# ADR-0030 — ACP runtime runs in-app with the adapter as a direct child; the E2-11-2 worker stays dormant

Status: accepted (issue #32)

## Context

E2-11-5 must connect the conversation path to a session implementation.
Two existed, unconnected, after E2-11-2/3:

1. `fartcode_runtime::SessionHost` — drives the `fartcode-acp-runtime` worker
   process over a bespoke JSON-RPC protocol with its own session
   lifecycle (start/prompt/cancel/stop/permissions). No history, no
   live streaming, no queue/drafts.
2. `fartcode_acp::session::SessionManager` + `SessionCell` — the full
   reference-shaped runtime (state machine, queue, drafts, history,
   transcript parser, live-model events), driven by an `AcpClient`.

ADR-0027 anticipated the seam ("the client handed in by the process
host"), but the worker's bespoke protocol never became an `AcpClient`.

## Decision

1. **The in-app runtime wins (user decision).** `fartcode-app::acp_runtime::
   AcpRuntime` owns the `SessionManager`; each conversation's adapter
   binary is spawned as a direct child process via `AcpClient::spawn`
   (env server-resolved — keyring `resolve_env` with the launcher's
   process-env fallback; the renderer never supplies env). All of
   E2-11-4's wiring (reducer, live models, `acp:*` events) works
   unchanged.

2. **The `fartcode-acp-runtime` worker stays dormant.** Its env-injection
   invariant is preserved by construction (renderer input never touches
   launch env here either — there IS no renderer env input). Retiring or
   repurposing it gets its own ticket if the out-of-process isolation
   returns (candidate: an ACP-stdio proxy). The E2-11-2 tests remain
   green and untouched.

3. **The provider decision is the only routing gate.**
   `resolve_session_path` (E2-11-3) is consumed in exactly two places:
   `create_conversation` decides the row's `type` server-side from
   `capabilities.acp` (the renderer never picks it), and `AcpRuntime::
   start` rejects anything that doesn't resolve to `SessionPath::Acp`.
   There is no persisted `runtime` column — the DTO field is derived
   (type + capability), so capability registry updates re-route rows
   without migration.

4. **Task teardown stops ACP before the row cascade.** `delete_task`
   calls `AcpRuntime::stop_task` BEFORE the domain deletion (the FK
   cascade removes conversation rows), mirroring the PTY reap ordering.

5. **Test seams, dev-only:** `FARTCODE_ACP_ADAPTER` overrides the adapter
   binary (the E2E suite points it at `fake_acp_adapter`); the frontend
   store exposes `window.__conversationsStore` for mocked-backend browser
   verification (the repo has no frontend test runner).

## Consequences

- #33 builds the transcript UI on `acp_history` + `acp:transcript`
  (the `LiveModels` snapshot) and the permission prompts on
  `acp:permission_request` — no further `fartcode-acp` changes needed.
- Boot rehydration of ACP sessions is NOT wired here (PTY rehydration
  stays byte-identical); an ACP rehydrate path is a follow-up when the
  chat UI lands.
- `fartcode-app` now depends on `agent-client-protocol-schema` (SessionId
  type on the runtime surface).
