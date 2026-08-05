# ADR-0025 — Durable interactive terminals (tmux-backed)

Status: accepted (issue #36; completes ADR-0021's deferred "tmux attach/kill
wiring into the terminal UI")

## Context

E2-12's interactive task terminals spawned `$SHELL` as a direct PTY child of
the ade process: app crash/restart killed every shell and its scrollback.
ADR-0021 shipped the full tmux durability machinery for AGENT sessions
(naming, create-or-attach shell line, kill, boot rehydration) but explicitly
deferred wiring it into the interactive terminal UI — and the "Use tmux for
terminals" project setting (`ProjectSettings.tmux`, seeded from
`tmux_by_default`) was consumed by nothing.

## Decision

Interactive terminals run under tmux when the project's `tmux` setting is on
AND a tmux binary resolves; otherwise the plain-PTY spawn is unchanged.

1. **Deterministic slot sessions:** session id
   `{project_id}:{task_id}:terminal:{slot}` → `make_tmux_session_name`
   (`ade-` + base64url). Slots are process-local: after a boot/crash-restart
   the slot table is empty, so the task's first `terminal_open` claims slot
   0 and the create-if-missing shell line REATTACHES the surviving session.
   Each further ⌘⇧T allocates the next free slot. No DB rows or
   rehydration orchestration needed — durability falls out of the
   deterministic name + create-or-attach line.
2. **Launch shape:** `sh -c build_tmux_shell_line(name,
   build_terminal_session_command(cwd, $SHELL))` through the same
   `PortablePtyManager` pump as plain terminals (events, resize, write all
   unchanged). The inner command is `cd <cwd> && exec <shell>` (both values
   via `shell_escape::single_quote`; `exec` makes the user shell the session
   foreground so `exit` ends it). `TERM=xterm-256color` is overlaid
   (portable-pty sets none; Dock-launched apps may inherit none) plus a
   `PATH` overlay when the binary resolved outside the inherited PATH
   (Homebrew is absent from the minimal Dock PATH).
3. **Binary resolution:** `resolve_tmux_binary` — PATH probe, then
   `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`; cached
   (`LazyLock<Option<TmuxBinary>>`). Absent → plain fallback; direct tmux
   calls (`kill-session`, `list-sessions`) route through the resolved
   command.
4. **Close semantics:** closing a terminal tab kills only the ATTACH client
   (PTY kill); the tmux session detaches and survives (reopen later gets the
   same shell). Deleting the task sweeps every session whose decoded id
   starts with `{project}:{task}:terminal:`
   (`kill_tmux_sessions_by_prefix`), including orphans from crashed app
   instances; foreign/malformed session names never match (they must decode
   via `parse_tmux_session_name` first).

## Consequences

- Crash/quit/restart survival is real (tmux server owns the shell); reboot
  is NOT — the tmux server dies with the box (reference parity).
- `tmux_by_default` is `false`, so default behavior is byte-identical to
  E2-12 until the user enables the setting (which the settings UI already
  exposes).
- Scrollback on reattach comes from tmux (100k lines), not xterm.
- tmux's status bar renders inside xterm (cosmetic); mouse mode is on per
  the shared shell line.
- Verified live 2026-08-04: boot opened slot 0 (attached) → typed `cd /tmp`
  in the UI → `kill -9` the app → session survived detached → relaunch
  reattached → `pwd` returned `/tmp`. Pinned in
  `ade-terminal/tests/tmux_durability_integration.rs` (real tmux binary,
  skip-if-absent; both readiness and reattach-cwd proofs are file-based —
  output matching races the PTY's echo of the typed command).
