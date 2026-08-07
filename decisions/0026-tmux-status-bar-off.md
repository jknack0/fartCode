# ADR-0026 — Hide the tmux status bar in fartCode terminals

Status: accepted (follow-up to ADR-0025)

## Context

With the project's `tmux` setting on (ADR-0025), each durable terminal runs
inside a tmux session named `fartCode-<base64url(session id)>`. tmux's default
status bar renders at the top of the pane and shows that opaque session name
(e.g. `fartCode-ODc5N2NhNjQt…`) — noise the user can't act on, since fartCode's own tab
bar already identifies the terminal. The reference keeps the bar; ADR-0025
noted it as cosmetic.

## Decision

`build_tmux_shell_line` sets `status off` **per session** alongside `mouse`
and `history-limit`, in the configure chain that runs on every
create-or-attach. Deviation from the reference's shell line (which sets only
mouse + history-limit).

Per-session (not `-g`), so the user's own tmux server outside fartCode keeps its
status bar; and because the configure chain re-runs on every attach, sessions
created before this shipped pick the option up on their next open — no
migration or one-shot sweep needed.

## Consequences

- Durable terminals render edge-to-edge shell; the bar never appears in new
  or pre-existing fartCode sessions.
- `status off` on tmux versions that ignore or reject the option is
  `2>/dev/null || true`-guarded like the other options — same failure
  contract.
- Pinned in `fartcode-core/src/pty/tmux.rs` (`tmux_shell_line_has_create_configure_attach`).
