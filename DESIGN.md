---
name: fartCode
description: Near-black terminal chassis, four meaningful colours, boxless cards, mono for machine text — the design_handoff_left_nav / design_handoff_v2 system.
colors:
  window: "#101012"
  rail: "#0b0b0d"
  flyout: "#0e0e10"
  overlay: "#17171b"
  card-inset: "#14141a"
  tile-active: "#1e1e22"
  drag-lift: "#1a1a1e"
  bubble: "#1a1a1e"
  hairline: "rgba(255,255,255,.06)"
  hairline-strong: "rgba(255,255,255,.12)"
  hairline-mid: "rgba(255,255,255,.07)"
  hairline-tile: "rgba(255,255,255,.14)"
  hover: "rgba(255,255,255,.035)"
  focus-bg: "rgba(255,255,255,.05)"
  focus-row: "rgba(255,255,255,.04)"
  text: "#e8e8ea"
  text-card: "#dcdce1"
  text-secondary: "#a4a4ab"
  text-muted: "#9a9aa1"
  text-mid: "#7c7c83"
  text-key: "#66666d"
  meta: "#5f5f66"
  disabled: "#4e4e55"
  dot-idle: "#46464d"
  accent: "oklch(.78 .15 155)"
  working: "oklch(.8 .13 80)"
  bad: "#c96b6b"
  bad-text: "#c98d8d"
  link: "#7c8fd0"
  brand-green: "#45d68a"
  brand-amber: "#dfa94d"
  brand-tile: "#0d0d10"
  brand-mono: "#c2c2c8"
  diff-add: "#8fd6ae"
  diff-selection: "rgba(110,231,168,.14)"
  xterm-bg: "#101012"
  xterm-fg: "#e8e8ea"
  xterm-cursor: "#e8e8ea"
typography:
  human:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, sans-serif (Inter Variable acceptable)"
    fontSize: "13px"
    lineHeight: 1.45
    fontWeight: 400
  machine:
    fontFamily: "JetBrains Mono Variable, JetBrainsMono Nerd Font Mono, ui-monospace, SFMono-Regular, Menlo, monospace"
    fontSize: "11px"
    lineHeight: 1.5
    fontWeight: 400
  group-label:
    fontFamily: "JetBrains Mono"
    fontSize: "10px"
    letterSpacing: ".14em"
    textTransform: uppercase
  meta:
    fontFamily: "JetBrains Mono"
    fontSize: "10.5px"
    lineHeight: 1.4
  mono-body:
    fontFamily: "JetBrains Mono"
    fontSize: "11.5px"
    lineHeight: 1.65
  terminal:
    fontFamily: "JetBrains Mono"
    fontSize: "12px"
    lineHeight: 1.6
  row-title:
    fontSize: "12.5px"
    lineHeight: 1.4
  body-lg:
    fontSize: "13.5px"
    lineHeight: 1.65
  panel-name:
    fontSize: "14px"
    fontWeight: 600
    letterSpacing: "-0.01em"
  pane-title:
    fontSize: "15px"
    fontWeight: 600
  glyph:
    fontSize: "18px"
    fontWeight: 400
  title:
    fontSize: "20px"
    fontWeight: 600
    lineHeight: 1.3
    letterSpacing: "-0.02em"
rounded:
  bubble-corner: "3px"
  chip: "4px"
  mark: "5px"
  card-hover: "6px"
  new-task: "7px"
  row: "8px"
  tile: "10px"
  overlay: "10px"
  window: "12px"
  active-bar: "2px"
spacing:
  scale: "4 / 6 / 9 / 14 / 16 / 18 / 22 / 26 / 28 / 34"
  rail-width: "56px"
  flyout-width: "244px"
  header-row: "46px"
  right-panel: "400px"
  settings-nav: "170px"
  drawer: "210px"
components:
  status-dot-running:
    backgroundColor: "{colors.working}"
    size: "7px"
    animation: "fc-pulse 1.8s ease-in-out infinite"
  status-dot-needs-you:
    border: "1.5px {colors.working}"
    backgroundColor: "transparent"
    size: "8px"
  status-dot-failed:
    backgroundColor: "{colors.bad}"
    size: "7px"
  status-dot-passed:
    backgroundColor: "{colors.accent}"
    size: "7px"
  status-dot-queued:
    border: "1px dashed #6e6e75"
    backgroundColor: "transparent"
    size: "7px"
  status-dot-idle:
    backgroundColor: "{colors.dot-idle}"
    size: "7px"
  card-rest:
    backgroundColor: "transparent"
    border: "none"
    borderLeft: "2px transparent"
    padding: "8px 12px"
  card-hover:
    backgroundColor: "{colors.hover}"
    rounded: "0 6px 6px 0"
  card-focused:
    backgroundColor: "{colors.focus-bg}"
    borderLeft: "2px {colors.accent}"
    rounded: "0 6px 6px 0"
  card-dragging:
    backgroundColor: "{colors.drag-lift}"
    rounded: "6px"
    boxShadow: "0 12px 28px rgba(0,0,0,.55)"
    opacity: 0.92
  overlay-card:
    backgroundColor: "{colors.overlay}"
    border: "1px {colors.hairline-strong}"
    rounded: "10px"
    boxShadow: "0 24px 60px rgba(0,0,0,.65)"
  inset-card:
    backgroundColor: "{colors.card-inset}"
    border: "1px {colors.hairline-strong}"
    rounded: "10px"
  commit-card:
    backgroundColor: "{colors.card-inset}"
    border: "1px rgba(255,255,255,.1)"
    rounded: "8px"
    padding: "12px 14px"
  popover:
    backgroundColor: "{colors.overlay}"
    border: "1px {colors.hairline-strong}"
    rounded: "8px"
    boxShadow: "0 16px 40px rgba(0,0,0,.5)"
  user-bubble:
    backgroundColor: "{colors.bubble}"
    rounded: "10px 10px 3px 10px"
    padding: "10px 13px"
    maxWidth: "85%"
  key-footer:
    fontFamily: "JetBrains Mono"
    fontSize: "11px"
    textColor: "{colors.text-key}"
    borderTop: "1px {colors.hairline}"
---

# Design System: fartCode

Supersedes the 2026-08-05 "emdash world" decision (charcoal `#111111`, blue
selection, Inter-first). Binding sources: `design_handoff_left_nav/README.md`
(rail, flyout, board, tokens) and `design_handoff_v2/README.md` + `FLOWS.md`
(task view, PM chat, ship loop, drawer, settings, logo). CSS variables in
`app-frontend/src/styles.css` `:root` are the single token home.

## Overview

**Creative North Star: "The Quiet Terminal"**

Near-black chassis (`#101012` window, `#0b0b0d` rail, `#0e0e10`
flyout/panels), hairlines instead of borders, no boxes at rest, and colour
reserved for meaning. The app's voice is typographic: system sans for human
text, JetBrains Mono for everything the machine produced — paths, chords,
ids, counts, elapsed, logs. Icons are glyphs (`+`, `⌘`, `‹`, `›`, `>_`,
`⌄`); there is no icon set.

**Key Characteristics:**
- Near-black surface ramp `#0b0b0d` → `#101012` → `#17171b`, separated by white-alpha hairlines
- Exactly four meaningful colours: accent, working-amber, bad-red, link-blue
- Boxless cards and rows — hover/focus washes and a 2px accent rail, never borders at rest
- Two type voices: system sans (human), JetBrains Mono (machine)
- Glyph icons only; status dots carry the whole state vocabulary
- Two animations total: the running-dot pulse and the input caret

## Colors

A near-black neutral ramp with four saturated jobs; everything informative
sits at `#5f5f66` or brighter.

### Primary
- **Accent Emerald** (`oklch(.78 .15 155)`): selection, staged/added lines, focused-card rail, active-tab underline, the app mark. Never decorative.

### Secondary
- **Working Amber** (`oklch(.8 .13 80)`): an agent is working (filled dot, pulsing) or needs you (hollow 1.5px ring). Unstaged `M` status letters.

### Tertiary
- **Ended-Badly Red** (`#c96b6b`): failed runs, conflicts, failing checks — dots and underlines; **Readable Red** (`#c98d8d`) is its text form. The only red *action label* in the app is `⌘⌫ delete` inside the delete confirm.
- **Link-Out Blue** (`#7c8fd0`): a link out (GitHub, files, logs, `install`) and NOTHING else.

### Neutral
- **Window** (`#101012`), **Rail** (`#0b0b0d`), **Flyout/Panel** (`#0e0e10`), **Overlay** (`#17171b`), **Inset card** (`#14141a`), **Active tile / drag lift** (`#1e1e22` / `#1a1a1e`).
- **Text**: primary `#e8e8ea` · card `#dcdce1` · secondary `#a4a4ab` · muted `#9a9aa1` · mid `#7c7c83` · key `#66666d` · meta `#5f5f66` · disabled `#4e4e55` · idle dot `#46464d`.
- **Hairlines**: `rgba(255,255,255,.06)` default, `.07` inside cards, `.12` overlay borders, `.14` active tiles.
- **Washes**: hover `rgba(255,255,255,.035)`, focus `rgba(255,255,255,.05)`.
- **Brand** (`#45d68a` green tile, `#dfa94d` amber, `#0d0d10` tile, `#c2c2c8` menu-bar mono): the fC mark, wordmark, and `shared`/`default` settings tags only — never UI chrome.

### Named Rules
**The Four Colours Rule.** Meaningful colour is exactly accent, working,
bad, and link — each with one fixed job. A fifth saturated colour, or one of
these four doing a different job, is wrong.

**The Meta Floor Rule.** `#5f5f66` is the legibility floor — nothing
informative goes dimmer. `#4e4e55` is reserved for disabled/empty text.

## Typography

**UI Font:** system sans (-apple-system stack; Inter Variable acceptable) — every human word.
**Machine Font:** JetBrains Mono Variable (Nerd Font stacks first inside terminals) — paths, chords, ids, counts, elapsed, logs, meta lines.

### Hierarchy
Sizes are the ramp: **10 / 10.5 / 11 / 11.5 / 12 / 12.5 / 13 / 13.5 / 14 /
15 / 20**. Mono sits 10–13; human text 12.5–20.
- **Title** (600, 20px/1.3): the task title, `max-width: 30ch`.
- **Panel titles** (600, 13–15px): header rows, settings pane names.
- **Body** (400, 13–13.5px/1.45–1.65): card titles, dialog copy.
- **Row title** (400, 12.5px/1.4, `#dcdce1`): flyout and list rows.
- **Meta** (mono, 10.5–11px, `#5f5f66`): the machine line under every title.
- **Group label** (mono, 10px, `.14em`, uppercase, `#5f5f66`): section labels.
- **Key hints** (mono, 11px, `#66666d`; the key itself `#a4a4ab`): footers and trailing hints.

### Named Rules
**The Two Voices Rule.** The machine speaks mono; the operator reads sans.
Never a path, chord, id, count, or elapsed time in sans; never body copy in
mono.

## Layout

56px icon rail → 244px project flyout (⌘\ / ⌘B, pinned not hover) → main
surface → optional 400px right panel (Changes / PM chat / card detail — one
slot). Header rows are 46px with a hairline below. Drawer (⌘J) is a 210px
bottom sheet. Settings nav is 170px. Under ~900px the board collapses to one
column and the rail narrows to 48px.

## Elevation & Depth

Surfaces are flat tints separated by hairlines. Only things that leave the
chassis cast shadows, and motion is exactly two keyframes: `fc-pulse`
(opacity 1→.35→1, 1.8s ease-in-out, infinite) on running dots, and
`fc-caret` (1.1s step-end, infinite) on input carets. No entrance
animations, no card/column transitions, no glow, gradient, or blur.

### Shadow Vocabulary
- **Overlay** (`0 24px 60px rgba(0,0,0,.65)`): composer, confirms, palette.
- **Popover** (`0 16px 40px rgba(0,0,0,.5)`): line-comment popover, inline confirms.
- **Dragged card** (`0 12px 28px rgba(0,0,0,.55)`): the lifted board card.
- **Scrim** (`rgba(0,0,0,.55)`, token `--backdrop`): the dim behind modal
  overlays. These four alpha-blacks are the only shadow/scrim values.

### Named Rules
**The Two Keyframes Rule.** `fc-pulse` and `fc-caret` are the app's entire
motion vocabulary. Any other animation is wrong.

## Shapes

Radii: 2px (active bar) · 3px (bubble tail corner) · 4px (chips) · 5px (app
mark) · 6px (hover/drag card) · 7px (dashed new-task) · 8px (rows, commit
card, popovers) · 10px (tiles, overlays, proposal cards) · 12px (window).
Circles belong to dots only (7px filled, 8px hollow). Borders are 1px
white-alpha hairlines; the only dashed strokes are the queued dot ring and
the add-project tile.

## Components

- **Cards/rows are boxless at rest**: meta line (mono 10.5px `--meta`) over
  title (13px/1.45 `--text-card`), 2px transparent left rail. Hover paints
  `--hover-bg`; keyboard focus is `--focus-bg` + 2px accent left rail; drag
  lifts to `#1a1a1e` radius 6 with the drag shadow, no rotation or scale.
  Drop target is a 1px accent line, never a ghost box.
- **Status dots** are the whole state vocabulary: filled amber pulsing =
  running; hollow amber = needs you; red = failed; accent = passed/additions;
  dashed ring = queued; `#46464d` = idle/stopped. 7px in rows, 8px hollow.
- **Overlay cards** (confirms, composer): `#17171b`, `.12` hairline, radius
  10, mono key footers (`esc cancel` left, primary key right, keys in
  `#a4a4ab`).
- **Inset cards** (proposal, commit): `#14141a` on panel background.
- **Key-first**: every action has a key, every button is labelled with its
  key, mono 11px `#66666d`.
- **User bubbles** (PM chat): right-aligned `#1a1a1e`, radius 10/10/3/10;
  agent turns are plain left text, no bubble.
- **Terminals** sit on the window colour with 12px mono.

## Pipeline board (handoff v3, E18/E19)

- **Step-done dot**: accent-filled 7px dot on a card = the step settled and
  the column holds (`on_settle: hold`), awaiting a human drag. Same dot as a
  passed check; distinct from hollow needs-you. Card hint: `↵ read
  <artifact> · drag on` where the step declares an artifact.
- **Header kind subline**: every column header carries a mono 10.5px subline
  — `shelf` / `human gate` / `counts as done` in `--disabled`; agent steps
  show `<provider> · <model> · <effort> — <trigger>[ → <advance_to>]`.
- **Confirm-free spend is brighter**: an `on_enter: run` subline renders
  `--text-muted` `#9a9aa1` (queue-mode stays `--meta`). Presence + brightness
  is the visibility; no new colour.
- **`counts_as_done` = dimmed**: the flag drives the dimmed header +
  50%-opacity cards; multiple terminal columns dim identically. Never key
  dimming on a column's name.
- **Landing tag**: the `is_landing` column shows a mono 10px `landing` tag
  in `--meta` after its name — information, not consent, so never green.
- **Delete-with-issues is a disabled label, not a dialog**: `delete column`
  sits in `--disabled` with the reason beside it in `--meta`; it activates
  the moment the column empties.
- Run-state derives from the live session, never from column identity. The
  flyout's in-flight contract: card in an `agent_step` column with a live
  session, or needs-you.

## Do's and Don'ts

### Do:
- **Do** put every token in `styles.css :root`; never a second styling system.
- **Do** derive elapsed times from timestamps on a tick; never store them.
- **Do** keep the flyout to in-flight work — the board owns everything else.
- **Do** give every action a key first, and label its button with the key (mono 11px).

### Don't:
- **Don't** go dimmer than the meta floor `#5f5f66` for informative text.
- **Don't** add icons, badges, tinted washes, chips — glyphs, dots and
  hairlines carry all meaning. GitHub links live inside the mono meta line.
- **Don't** use blue for anything but links out; green beyond
  selection/additions; red beyond ended-badly.
- **Don't** animate anything beyond the two keyframes.
