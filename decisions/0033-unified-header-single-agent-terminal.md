# ADR-0033: One header row everywhere; one agent terminal per task

**Status:** accepted (dogfood feedback, 2026-08-07)

## Context

Two shapes of the shell diverged as features accreted:

1. **The top chrome changed per scope.** Project scope renders a full-width
   `app-header` row (project name + sheet toggles + GitHub sync). Task scope
   had no header row at all — the tab bar doubled as chrome, carrying the
   lifecycle script launchers (E1-06) and the Changes toggle (E4-03) in its
   trailing edge. Switching between scopes visually rearranged the top of the
   window, and the tab bar mixed two jobs: tab switching and scope controls.
2. **Agent terminals stacked.** `terminal_open_agent` spawned a fresh PTY on
   every call, so the same task could accumulate several agent tabs: board
   dispatch (E17-03), ⌘⇧O (OMP), and the comment-task flow each minted their
   own. A task's agent is its workload — running two copies of an agent CLI
   in one worktree is a collision, not a feature, and directly opposes the
   "one agent, one worktree" model dispatch already assumes (ADR-0032:
   reattach, never spawn a second worktree).

## Decision

1. **The header grid area always renders.** Project scope keeps
   `ProjectHeader`; task scope gets `TaskHeader` in the same grid area with
   the same shape: a title slot (project / task breadcrumb) and an actions
   slot (script launchers + Changes toggle). The tab bars become pure tab
   switching — no trailing chrome.
2. **One agent terminal per task.** `TerminalManager::find_running_agent`
   locates the task's live agent entry; `terminal_open_agent` returns it
   when present instead of spawning (the lifecycle-dedupe pattern, E1-06).
   The check runs before provider resolution so a live session stays
   reachable even if its binary left PATH. An exited agent terminal drops
   its entry (plain-PTY behavior), so the next open mints a fresh one.
3. **Frontend converges on the one tab.** The tabs store's `addTab` focuses
   an existing tab with the same id instead of no-oping, so ⌘⇧O and dispatch
   land on the live agent tab. Tab restore (`ensureTabs`) surfaces live
   agent terminals no persisted tab covers — dispatch spawns the agent
   before navigation, and the task view must show the session it handed
   off to.
4. **Add Task spawns the agent; empty panes stay empty.** `create_task`
   (left nav) launches the default agent in the new worktree — the PRD's
   "Add Task → spawns the chosen agent" — best-effort, same provider
   resolution as dispatch. The frontend never auto-spawns a plain shell on
   task open: with nothing running, the pane shows summon hints (⌘T / ⌘D)
   instead of an unsummoned TTY tab.
5. **No tab bar with nothing to switch.** A pane's tab bar renders only when
   the task has a second tab or a split; the common case (one agent
   terminal, no split) shows the terminal directly under the header. A lone
   "TTY claude" chip restated what the breadcrumb already says and made the
   agent look like one of several tabs rather than the task itself.

## Consequences

- Switching agents means closing the agent tab first (the next open spawns
  the new provider). Acceptable: agent swaps are rare; accidental stacking
  was the real hazard.
- The Changes toggle moves from the tab bar trailing to the header row in
  task scope; project scope's behavior is unchanged (same toggle, same
  key ⌘⇧1).
- `.changes-toggle`, `.tab-bar-trailing`, and `.tab-bar-actions` CSS is
  dead and removed; script launchers keep their styling in the header.
- The dispatch prompt write (`terminal_write` after open) is unchanged: a
  reattach returns the live id, and the caller only writes the packet on
  first dispatch (the `reattached` branch skips it), so a reattached
  session never receives a duplicate prompt.
- With the bar hidden there is no × on the sole tab: ⌘W closes it (and the
  split's active-pane tint still needs the bar, which is why a split keeps
  both bars regardless of tab count).
