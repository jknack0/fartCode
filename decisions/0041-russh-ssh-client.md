# ADR-0041: russh 0.50 for SSH client layer

**Status:** Accepted  
**Date:** 2026-08-09  
**Issue:** #85 (E12-01)

## Context

Phase 3 — Remote requires SSH connectivity. Options evaluated:

| Option | Pros | Cons |
|--------|------|------|
| Shell out to `git` CLI | Simple, proven | No PTY control, no key forwarding, slow |
| `ssh2` crate | Mature | Rust 1.75+ only, sync-heavy |
| russh 0.50 | Async, PTY channels, agent forwarding, active | Steeper API |

## Decision

Use russh 0.50 (workspace declared). Matches our async-first architecture. Provides PTY channels (E12-05), agent forwarding (E12-06), and direct TCP/IP forwarding (E12-09) without extra crates.

## Consequences

- `fartcode-ssh` crate owns all SSH logic
- Handler accepts all server keys (ponytail: `known_hosts` in E12-03)
- ssh-key fork (`internal-russh-forked-ssh-key =0.6.9`) required for key compatibility
- async-trait feature of russh enabled for Handler trait
- Channel<ClientMsg> returned directly; caller owns I/O via `into_stream()`
