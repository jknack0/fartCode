# ADR-0028: Terminal lifecycle — close kills, reopen reattaches, restore surfaces survivors

- **Status:** Accepted
- **Date:** 2026-08-05
- **Ticket:** follow-up to #36 (ADR-0025)
- **Relates to:** ADR-0025, ARCHITECTURE.md §7 (TerminalManager)

## Context

ADR-0025 shipped detach-on-close semantics for tmux-backed terminals: closing
a tab killed only the attach client, so the session survived detached and the
NEXT OPEN in the same process got a fresh slot. Two failure modes showed up
on a real machine:

1. **Sessions accumulate and never resurface.** Each restart/close cycle left
   detached sessions behind; opens kept minting new slots (a single task grew
   to slots 0–10). Survivors were invisible until the user pressed ⌘T, which
   *sometimes* reattached them via create-or-attach — read as "terminals
   don't automatically show when I reopen".
2. **Close ≠ teardown confused the model.** A user closing a tab expects the
   terminal gone, not parked detached for a later open to trip over.

## Decision

Three lifecycle changes (all behind the existing project `tmux` setting):

1. **Close tab kills the session.** `TerminalManager::close` now runs
   `kill-session` on the terminal's tmux session (the attach client dies with
   the PTY as before) and frees the slot. Closing a tab is an intentional
   teardown — nothing survives to reattach. Plain-PTY terminals are
   unchanged (they had nothing to survive into).
2. **Open reattaches survivors instead of minting slots.** `pick_slot` →
   `ade_core::pty::tmux::choose_terminal_slot` (pure, unit-tested): reuse the
   smallest live DETACHED session of the task that this process doesn't
   already own; else the first slot unused locally AND on the tmux server
   (never double-attach a session another client holds). Crash/restart
   recovery now lands back on the surviving shell with its cwd/scrollback.
3. **Restore surfaces every existing terminal.** Window close detaches all
   PTYs but keeps sessions (`WindowEvent::Destroyed` → `detach_all`), so
   reopening reattaches. On task restore the frontend asks
   `terminal_surviving` how many live sessions this process does NOT cover
   and opens extra tabs until every survivor is shown — persisted tabs
   reattach first (slot reuse), survivors fill the rest. Nothing spawns
   fresh while a survivor exists.

## Consequences

- Reopen after quit/crash shows every still-running terminal automatically —
  no ⌘T archaeology.
- Close is final for the session: users who want "park it" should not close
  the tab (tmux durability now means *crash/window-close* durability, not
  close durability).
- `tmux_by_default` off / tmux absent → byte-identical to before (plain PTYs,
  `terminal_surviving` returns 0, restore respawns fresh shells).
- Session numbers no longer climb forever: close frees the slot, reuse
  consumes survivors first.
- Foreign/malformed session names are still safe — listing and kill both go
  through `parse_tmux_session_name` first.
