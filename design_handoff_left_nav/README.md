# Handoff: left nav redesign + app shell (fartCode)

## Overview
Replaces the current left navigation — Projects tree, worktree paths, per-project "New task",
Recent list, Settings bar — with a 56px icon rail plus a 244px project flyout, and carries that
same restraint through the rest of the app: board, task view, composer, sessions, review,
settings, and the empty/failure states.

The nav's job list, decided with the user: switch project, create a task, see in-flight work,
jump to a task, see agent state, collapse. Everything else moved to the board or to a key.

## About the design files
The files in this bundle are **design references created in HTML** — a prototype showing the
intended look and behaviour, not production code to paste in. `fartCode App.dc.html` opens in a
browser and contains every screen, laid out as labelled frames (2a…4h).

The `react/` folder is different: it is real, typed React written against this design, with no
CSS file and no dependencies beyond React. Treat it as a **reference implementation** — adopt it
wholesale, or lift the values into the app's existing component patterns. Do not introduce a new
styling system to accommodate it; if the codebase already has one, port the token values in
`react/theme.ts` into it.

## Fidelity
**High fidelity.** Colours, type sizes, spacing and states are final and exact. Recreate them
faithfully. The only intentionally loose parts are the board's card copy (sample data) and the
dimmed board slivers used as context in some frames.

---

## Screens

Frame ids below match the badges in `fartCode App.dc.html`.

### Rail (present on every screen)
- **Purpose**: switch project, create, reach sessions and settings.
- **Layout**: 56px wide, `background #0b0b0d`, `border-right 1px solid rgba(255,255,255,.06)`,
  column flex, `align-items: center`, `padding: 14px 0`, `gap: 14px`.
- **Components (top to bottom)**:
  - App mark: 18×18, `border-radius 5px`, `background oklch(.78 .15 155)`, `margin-bottom 6px`.
  - Project tile ×2–5: 34×34, `border-radius 10px`, first letter of the project name,
    13px / 600. Inactive: `border 1px solid rgba(255,255,255,.07)`, `color #7c7c83`, transparent
    background. Active: `background #1e1e22`, `border 1px solid rgba(255,255,255,.14)`,
    `color #e8e8ea`, plus a 2×16 `border-radius 2px` bar in `oklch(.78 .15 155)` at
    `left: -14px; top: 9px` (it sits in the rail's left gutter).
  - Agent dot: 8×8 at `right: -3px; bottom: -3px`, `border 2px solid #0b0b0d`. Filled
    `oklch(.8 .13 80)` + pulse when running; hollow (1.5px ring, transparent centre) when the
    agent needs the user; `#c96b6b` filled when a run failed.
  - `+` new task: 34×34, 18px glyph, `color #55555c`.
  - `>_` sessions: 34×34, mono 13px. Same active/dot treatment as a project tile.
  - Spacer, then `⌘` settings: 34×34, mono 13px, `color #55555c`.

### Flyout (2a)
- 244px, `background #0e0e10`, `border-right 1px solid rgba(255,255,255,.06)`, `padding 18px 14px`.
- Project name 14px / 600 / `-0.01em`, with a `‹` collapse control at 14px `#55555c`.
- Directly under it, one mono line at 11px `#66666d`: `…/Dev/ade · main`. `margin-bottom: 28px`.
- Group label: mono 10px, `letter-spacing .14em`, uppercase, `#5f5f66`, `margin-bottom 14px`.
  Groups in order: **Needs you**, **Running**, **Sessions**. A group is omitted when empty; when
  all are empty the flyout shows mono 11px `#4e4e55` "nothing running".
- Row: `display:flex; gap: 9px`. Dot 7×7 at `margin-top: 5px`. Title 12.5px/1.4 `#dcdce1`.
  Meta mono 10.5px `#5f5f66` at `margin-top: 4px`: `#392 · 4m`. Rows are `gap: 16px` apart.
- **Only in-flight work appears here.** The board owns everything else. This is the single most
  important constraint in the redesign — the old nav felt heavy because it mirrored the board.

### Board (2a)
- Columns: Backlog, Ready, In progress, In review, Done. Each `flex: 1; min-width: 0`,
  `padding: 22px 16px 0 22px`, separated by `border-left 1px solid rgba(255,255,255,.05)`.
- Column header: name 13px `#c9c9cf` (Done uses `#6e6e75`), count mono 11px `#5f5f66`,
  `padding-bottom 12px`, `border-bottom 1px solid rgba(255,255,255,.07)`, `margin-bottom 16px`.
- **Cards have no box.** Meta line (mono 10.5px `#5f5f66`) over title (13px/1.45 `#dcdce1`),
  `gap: 6px` inside, `gap: 18px` between cards. Running cards prefix a 7×7 dot with `gap: 9px`.
- Overflow indicator: mono 11px `#5f5f66`, e.g. `+ 7 more`.

### Task (2b)
Replaces the board area; not a modal. Header mono 11px `#66666d`
(`#392 · steadywith-85 · branch fix/invite-resend`) with `esc` right-aligned. Title 20px/600/1.3,
`max-width: 30ch`. Status row: dot + mono 11px `#9a9aa1`. Body 13.5px/1.65 `#a4a4ab`,
`max-width: 62ch`. Agent transcript: mono 11.5px/1.5, `#7c7c83`, active line `#dcdce1` with a
1px `oklch(.78 .15 155)` caret blinking on a 1.1s step-end loop. Footer keys mono 11px `#66666d`
above a `border-top 1px solid rgba(255,255,255,.06)`, `gap: 20px`.

### Composer (2c / 2e)
440px overlay, `background #17171b`, `border 1px solid rgba(255,255,255,.12)`,
`border-radius 10px`, `box-shadow 0 24px 60px rgba(0,0,0,.65)`. Input row `padding 18px 18px 16px`
with a leading glyph. Footer row `padding 11px 18px`, `border-top 1px solid rgba(255,255,255,.07)`,
mono 11px `#66666d`, project · branch on the left, keys on the right.
**Typing `>` as the first character switches it from a task to a bare TUI session**: the glyph
turns `oklch(.78 .15 155)`, the input switches to mono, the footer becomes `session · ade · main`
and `↵ open`. No second dialog. `⌘⇧↵` from anywhere opens it already in session mode.

### Session (2d) and history (4d)
Transcript only: mono 11.5px/1.6, prompts prefixed `›` in `oklch(.78 .15 155)`, output `#7c7c83`.
Header mono 11px: `session · ade · main · 2m`. Footer: `⌘.` end, `⌘⇧n` keep as a task — the one
bridge from a session to the board. History lists live sessions first, then **Earlier today** at
75% opacity with grey `#3a3a40` dots, `ended · 14m · 11:04`, kept 7 days, `↵` resumes.

### First run (3a)
No projects: rail shows only the mark, a dashed `+` tile, and `⌘`. Centre column,
`padding: 0 56px`, `gap: 26px`: mono label "No projects", then two rows — "Open a folder" `⌘O`
and "Clone from GitHub" `⌘⇧O` — each 14px `#dcdce1` with the key mono 11px `#5f5f66`, separated by
`border-bottom 1px solid rgba(255,255,255,.07)`. Below: "or drop a folder anywhere in this window".

### Jump (3b)
`⌘K`. 440px overlay, same shell as the composer. Input row mono 13px with `⌘K` as the prefix.
Results mix tasks, sessions, projects and actions in one list; selected row
`background #202026; border-radius 6px`. Left side: optional status dot + label truncated with
ellipsis; right side: mono 10.5px `#5f5f66` context (`#392 · ade`, `session · 2m`, `⌘N`).

### Blocked (3c)
The agent's question renders inline in the transcript behind a 2px `oklch(.8 .13 80)` left border
with `padding-left: 14px`. Question 13.5px/1.6 `#dcdce1`, `max-width 46ch`. Numbered answers are
rows with the digit as a mono key on the right. A free-text answer is always allowed.
Flyout gains a **Needs you** group above **Running**.

### Review (3d)
208px file list (mono 11px, name truncated, `+18` in `oklch(.78 .15 155)`, `−4` in `#c96b6b`)
next to the diff. Diff rows: mono 11.5px/1.8, gutter 26px right-aligned `#5f5f66`, added rows
`background rgba(110,231,168,.07)` text `#8fd6ae`, removed `background rgba(201,107,107,.09)`
text `#c98d8d`. Footer: `⌘↵` merge, `⌘⇧r` ask for changes, `j k` files.

### Settings (3e)
A pane, not a modal — the `⌘` rail tile becomes active. `padding: 34px 44px`, `gap: 34px` between
groups. Group label mono 10px uppercase `#5f5f66`. Rows: label 13.5px `#dcdce1` left, value mono
11px `#9a9aa1` right with a `⌄` when it opens a menu, `padding-bottom 11px`,
`border-bottom 1px solid rgba(255,255,255,.06)`. Groups: Projects, Agent (model, ask-before,
run-at-once), Connections.

### Collapsed (3f)
`⌘\` hides the flyout; the rail stays. Board reflows to full width. Rail keeps agent dots so
project state is never hidden by collapsing.

### Run states (4a)
Full vocabulary, all in the same row shape:
| state | dot | meta |
|---|---|---|
| running | filled amber, pulsing | `#392 · writing tests · 4m` |
| needs you | hollow amber ring 1.5px | `#390 · needs you · 30s` |
| failed | filled `#c96b6b` | `#388 · failed · tests exit 1` + hint `↵ read · ⌘r retry` |
| conflict | filled `#c96b6b` | `#384 · conflict with main` + hint `↵ resolve · ⌘⇧r rebase` |
| stopped | filled `#46464d` | `#387 · stopped by you · 8m`, title drops to `#a4a4ab` |
| queued | dashed `#6e6e75` ring | `#386 · queued · 2nd of 3`, whole row `opacity: .55` |
Failure meta text is `#c98d8d`, not the standard `#5f5f66`.

### Card states (4b)
- rest: transparent, `border-left: 2px solid transparent`, title `#dcdce1`
- hover: `background rgba(255,255,255,.035)`, `border-radius 0 6px 6px 0`, title `#fff`, and the
  meta line's right side reveals `↵ open`
- focused (keyboard): `background rgba(255,255,255,.05)` + `border-left 2px solid oklch(.78 .15 155)`
- dragging: `background #1a1a1e`, `border-radius 6px`, `box-shadow 0 12px 28px rgba(0,0,0,.55)`,
  `opacity .92`, **no rotation or scale**
- drop target: a 1px `oklch(.78 .15 155)` line between cards — never a ghost box or outline

### Backgrounded (4e)
Notification only for **needs you** and **failed** — never for started or finished. Dock badge is
amber for needs-you and red for failed; running counts are silent. Menu bar shows the mark, the
running count, and a hollow ring if anything is blocked.

### GitHub (4f)
The chip is gone; the link lives in the mono meta line: `#392 · gh 85`, `#385 · pr 214 · 1 review`,
or nothing at all when the task is local. Link colour `#7c8fd0`, and blue is only ever a link.
`⌘⇧i` imports open issues into Backlog.

### Narrow (4g)
Under ~900px the board becomes one column. Column names collapse to a mono strip
(`backlog 12  ready 2  progress 2  review 1  done 38`); the active one is `#e8e8ea` with a
`border-bottom 1px solid oklch(.78 .15 155)`; a column with running work is `oklch(.8 .13 80)`.
`h` / `l` switch columns. Rail narrows to 48px, tiles to 30×30.

### Keys (4h)
`?` opens the sheet. Two columns, groups Do / Review / Move / Window. Row: label 12.5px `#b6b6bd`
left, key mono 11px `#7c7c83` right.

---

## Interactions & behaviour
- Flyout is pinned by default, not hover-triggered; `⌘\` toggles it and the state persists.
- `⌘1…5` switches project. Rail order is stable — never reorder by recency.
- `⌘N` composer (task) · `⌘⇧↵` composer (session) · `↵` queue · `⌘↵` start now.
- `j k` move card focus, `h l` move column focus, `⇧` + those moves the card itself, `↵` opens.
- `⌘.` stop · `⌘r` retry · `⌘⇧r` rebase or ask-for-changes depending on surface · `⌘⇧i` import
  issues · `⌘,` settings · `?` key sheet · `esc` closes any overlay or focused view.
- Pulse animation: `opacity 1 → .35 → 1`, 1.8s, `ease-in-out`, infinite. Caret: 1.1s `step-end`.
- No other motion. No card entrance animations, no column transitions.
- Every action shown as a key must also be reachable by mouse; every button must be labelled with
  its key. If a new action needs a button, add the key first.

## State management
Per window: `activeProjectId`, `view: 'board' | 'task' | 'session' | 'settings' | 'review'`,
`navOpen`, `focusedCardId`, `focusedColumn`, `overlay: null | 'composer' | 'jump' | 'keys' | 'addProject'`.
Per project: `tasks` (with `state`, `status`, `startedAt`, `gh`), `sessions` (live + 7 days of
history), `branch`, `path`, `concurrencyLimit`. Elapsed times are derived from `startedAt` on a
1s tick, never stored. The flyout must be fed a filtered list (in-flight only), not the full task
array — enforce it at the type level; `react/ProjectFlyout.tsx` does.

## Design tokens
Colours: window `#101012` · rail `#0b0b0d` · flyout `#0e0e10` · overlay `#17171b` · active tile
`#1e1e22` · hairline `rgba(255,255,255,.06)` · strong hairline `rgba(255,255,255,.12)` · hover
`rgba(255,255,255,.035)` · focus `rgba(255,255,255,.05)`.
Text: primary `#e8e8ea` · card `#dcdce1` · secondary `#a4a4ab` · muted `#9a9aa1` · meta `#5f5f66`
(**legibility floor — nothing informative goes dimmer**) · disabled `#4e4e55`.
Meaningful colour, and only these three: `oklch(.78 .15 155)` selection, additions, the app mark;
`oklch(.8 .13 80)` an agent is working (filled) or needs you (hollow); `#c96b6b` a run ended badly.
`#7c8fd0` is a link out and nothing else.
Type: system sans for human text, JetBrains Mono (or any mono) for machine text. Sizes 10 / 10.5 /
11 / 11.5 / 12.5 / 13 / 13.5 / 14 / 15 / 20. Uppercase labels carry `letter-spacing: .14em`.
Radius: 5 (mark) · 6 (hover/drag card) · 8 (rows) · 10 (tiles, overlays) · 12 (window).
Shadow: overlays `0 24px 60px rgba(0,0,0,.65)`; dragged card `0 12px 28px rgba(0,0,0,.55)`.
Spacing: 4 / 6 / 9 / 14 / 16 / 18 / 22 / 26 / 28 / 34.

## Assets
None. Every mark is a plain rounded rect or circle; every icon is a typographic glyph
(`+`, `⌘`, `‹`, `⌄`, `>_`, `›`, `/`). Do not add an icon set for this — the glyphs are the design.

## Files
- `fartCode App.dc.html` — every screen, frames 2a–4h. Open in a browser.
- `Left Nav Redesign.dc.html` — the three original nav directions; 1a is the one chosen.
- `react/theme.ts` — tokens. The only place values live.
- `react/StatusDot.tsx` — the status vocabulary + the two keyframes.
- `react/LeftRail.tsx` — the 56px rail.
- `react/ProjectFlyout.tsx` — the 244px flyout.
- `react/TaskCard.tsx` — boxless card and column header.
- `react/Composer.tsx` — the one-field composer with the `>` session switch.
- `react/README.md` — wiring and the rules the code encodes.

## What to delete
Projects section header and count · per-project disclosure arrows · the worktree path row under
each project · the per-project "New task" row · the Recent list · the Settings bar at the bottom
of the nav.
