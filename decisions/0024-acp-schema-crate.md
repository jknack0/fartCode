# ADR-0024: ACP wire types from the official schema crate; own transport/client

**Status:** Accepted (2026-08-04) · **Ticket:** #28 (E2-11-1)

## Context

PRD §10.1 left open: implement the ACP client types ourselves (public spec)
or use an existing Rust crate. Since the PRD was written, the ACP spec
repository (zed-industries/agent-client-protocol) now ships an official
Rust schema crate, `agent-client-protocol-schema` (v1.6.0, Apache-2.0):
typed, serde-ready wire types for JSON-RPC envelopes and every v1
method/notification, generated alongside the spec itself.

## Decision

- **Wire types:** depend on `agent-client-protocol-schema` (default features,
  stable v1 module). It is the spec's own source of truth; hand-maintained
  serde structs would drift from protocol revisions.
- **Transport + client:** our own (`ade-acp::transport` / `ade-acp::client`).
  The spec ships no Rust client runtime; we need newline-delimited JSON-RPC
  over child-process stdio, our pending-request correlation, update-stream
  fan-out, and client-side `fs/*` + `terminal/*` request handlers.
- **Version policy:** pin the major protocol version we negotiate
  (`PROTOCOL_VERSION` in `client.rs`); reject an `initialize` response that
  disagrees.
- Workspace `rust-version` (1.85) bumped to **1.88** — the schema crate's
  minimum. All crates build on it; CI already uses the stable toolchain.

## Consequences

- Protocol drift is caught at compile time when the crate is bumped; bumps
  are reviewable diffs against generated types.
- We carry the crate's transitive deps (serde_with, schemars, derive_more,
  strum) — acceptable for a desktop app.
- `#[non_exhaustive]` types in the crate mean we must handle unknown enum
  variants defensively (wildcard arms) — the transport already treats
  unparseable frames as warnings, matching the ticket's malformed-frame
  criterion.
