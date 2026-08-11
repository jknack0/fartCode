# ADR-0043: Connection lifecycle lives in the pool, not in the callers

- **Status:** Accepted
- **Date:** 2026-08-11
- **Issue:** #90 (E12-06)
- **Relates to:** ADR-0041 (russh client), ADR-0042 (remote projects), ADR-0025 (tmux durability), ARCHITECTURE.md §6.6

## Context

E12-04 and E12-05 left two different lifetimes for the same host. Terminals
and tmux shared a cached `SshPtyManager` per connection; every remote-project
command opened and dropped its own `SshClient`. Neither noticed a session
dying: the cache handed out a corpse, the commands paid a handshake per
keystroke-sized operation, and the UI had one boolean (`is_connected`) to
describe five possible situations.

E12-06 needs states, bounded reconnect, rehydrate, and a MaxSessions signal.
The open question was where that logic belongs, and how far "rehydrate" goes.

## Decision

- **One pool owns every session.** `RemotePtyRegistry` holds one `SshClient`
  per connection id, plus the `SshPtyManager` and `RemoteTmux` bound to it.
  Commands borrow it (`client_for`); nothing else constructs a client. A
  cached entry whose `SshClient::is_closed()` is true is dropped on read, so
  no caller receives a dead session.
- **Liveness is polled, not subscribed.** russh exposes a closed flag but no
  close future on `Handle`. A 2 s watchdog per dialed session reads that flag;
  a close-notification channel would mean forking the handler for a signal
  that costs an atomic load to sample.
- **Generations, not cancellation.** Each dial bumps a per-connection counter
  and stamps the entry. A watchdog holding a stale generation exits instead of
  racing the dial that replaced it — the same guard the reference
  (`ssh-connection-manager.ts`) uses, minus the promise bookkeeping.
- **The ladder is fixed and finite**: 1/2/5/10/20 s, one attempt per rung,
  then `Error`. No jitter, no infinite retry — a host that ignored five rungs
  is the user's problem to see, not the app's to hammer.
- **Manual disconnect is intent, and it outranks everything.** The
  disconnect set suppresses the watchdog, the ladder, terminal opens and boot
  rehydration until an explicit `connect` clears it (E12-05 AC13 kept intact).
- **MaxSessions is health, not state.** A refused channel on a live session
  cannot be fixed by reconnecting, so it publishes
  `SshConnectionHealthChanged { degraded }` instead of moving the state
  machine. Classification is message-based (`is_channel_open_failure`):
  russh flattens the channel-open reason code into its error text.

## Consequences

- **Rehydrate rebinds the pool, not the terminals.** After a reconnect the
  next route, browse or clone reaches the new session; existing terminal PTYs
  stay dead. Their process and scrollback live in the host's tmux server
  (ADR-0025, E12-05 AC12), so reopening the terminal attaches back — respawning
  them from the pool would duplicate that path and race the user's own reopen.
- A dropped connection is noticed up to 2 s late, and only while the process
  is running: the watchdog is not a keepalive and does not hold the session
  open by itself.
- `ssh_connection_state` now returns a status object (`state`, `connected`,
  `degraded`) instead of a bool. No frontend consumed the bool yet.
- Health is per connection, not per channel: one refusal marks the host
  degraded until a channel opens again.
