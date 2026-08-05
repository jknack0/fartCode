---
name: ade
description: emdash's working surface — neutral charcoal chassis, emerald primary action, blue selection, status hues for agent state.
colors:
  background: "#111111"
  background-1: "#181818"
  background-2: "#201f20"
  background-3: "#282728"
  background-secondary: "#181818"
  background-secondary-1: "#111111"
  background-secondary-2: "#282728"
  background-secondary-3: "#383738"
  background-tertiary: "#201f20"
  background-tertiary-1: "#282728"
  background-tertiary-2: "#302f30"
  background-tertiary-3: "#383738"
  surface-elevated: "#252525"
  surface-elevated-hover: "#2f2f2f"
  surface-elevated-selected: "#393939"
  foreground: "#e9e8e8"
  foreground-body: "#d5d4d5"
  foreground-muted: "#b8b7b8"
  foreground-passive: "#929192"
  border: "#383738"
  border-1: "#626162"
  border-2: "#787778"
  border-primary: "#929192"
  accent: "#00a67b"
  accent-hover: "#00b589"
  accent-border: "#006c50"
  accent-contrast: "#ffffff"
  selection: "#173865"
  selection-foreground: "#d1ebff"
  info: "#82baff"
  info-background: "#18202b"
  info-border: "#173865"
  destructive: "#f27470"
  destructive-background: "#2a1b1a"
  destructive-background-hover: "#36201e"
  destructive-border: "#964441"
  status-in-progress: "#dbae50"
  status-in-review: "#59b358"
  status-neutral: "#929192"
  xterm-bg: "#181818"
  xterm-fg: "#e9e8e8"
  xterm-cursor: "#e9e8e8"
  xterm-selection-bg: "rgba(57, 142, 255, 0.475)"
  xterm-selection-fg: "#82baff"
typography:
  body:
    fontFamily: "Inter Variable, ui-sans-serif, system-ui, -apple-system, sans-serif"
    fontSize: "14px"
    lineHeight: "20px"
    fontWeight: 400
  body-semibold:
    fontFamily: "Inter Variable, ui-sans-serif, system-ui, -apple-system, sans-serif"
    fontSize: "13px"
    fontWeight: 600
  h1:
    fontFamily: "Inter Variable, ui-sans-serif, system-ui, -apple-system, sans-serif"
    fontSize: "20px"
    lineHeight: "28px"
    fontWeight: 600
  micro-label:
    fontFamily: "Inter Variable, ui-sans-serif, system-ui, -apple-system, sans-serif"
    fontSize: "12px"
    fontWeight: 500
  machine:
    fontFamily: "JetBrains Mono Variable, JetBrainsMono Nerd Font Mono, MesloLGS Nerd Font Mono, ui-monospace, SFMono-Regular, Menlo, monospace"
    fontSize: "12px"
    fontWeight: 400
rounded:
  sm: "6px"
  md: "8px"
  lg: "10px"
  xl: "14px"
spacing:
  row-sidebar: "32px project rows / 30px task rows"
  tab-bar: "41px"
  sidebar-width: "264px"
components:
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-contrast}"
    rounded: "{rounded.lg}"
    padding: "6px 12px"
  button-primary-hover:
    backgroundColor: "{colors.accent-hover}"
  button-primary-disabled:
    backgroundColor: "{colors.background-3}"
    textColor: "{colors.foreground-passive}"
  button-danger:
    backgroundColor: "{colors.destructive-background}"
    textColor: "{colors.destructive}"
    rounded: "{rounded.lg}"
  button-ghost:
    backgroundColor: "{colors.surface-elevated}"
    textColor: "{colors.foreground-muted}"
    rounded: "{rounded.md}"
  sidebar-row:
    backgroundColor: "transparent"
    hover: "{colors.background-tertiary-1}"
    selected: "{colors.background-tertiary-2}"
    rounded: "{rounded.lg}"
    height: "32px"
  tab-active:
    backgroundColor: "{colors.background-secondary-1}"
    textColor: "{colors.foreground}"
  modal-plate:
    backgroundColor: "{colors.background-2}"
    rounded: "{rounded.xl}"
    border: "1px {colors.border}"
  input-field:
    backgroundColor: "{colors.background-1}"
    textColor: "{colors.foreground}"
    rounded: "{rounded.md}"
    border: "1px {colors.border}"
  status-dot-working:
    backgroundColor: "{colors.status-in-progress}"
    size: "8px"
  status-dot-review:
    backgroundColor: "{colors.status-in-review}"
    size: "8px"
  status-dot-neutral:
    backgroundColor: "{colors.status-neutral}"
    size: "8px"
---

# Design System: ade

## Overview

**Creative North Star: "emdash's working surface"**

ade wears the visual world of its reference implementation, emdash: a neutral
charcoal chassis where surfaces step through background (main), secondary (tab
bars, wells), and tertiary (the sidebar column); hairline borders do the
separating; and color is strictly functional. Emerald is the one action color —
the primary key that adds, saves, and commits. Blue is the interaction color —
text selection, the keyboard-focus lamp on the active pane's tab, customized
marks, and informational notices. Amber and green are agent-state data on the
sidebar's status dots; red is destructive only. Everything else is a neutral
ramp from #111111 to #e9e8e8.

The system is dense and single-operator: 14px Inter body on a 20px line, 32px
sidebar rows, a 41px tab bar, 264px sidebar. Two type voices: Inter Variable
for every UI word, JetBrains Mono Variable for everything the machine produced
— task names, paths, chords, tab titles, terminal text. Nerd Font stacks
precede JetBrains Mono inside terminals so Powerline glyphs render.

**Key Characteristics:**
- Neutral charcoal chassis: background #111111, sidebar tertiary #201f20, tab bar secondary #181818
- Hairline 1px borders (#383738) separate regions; no glow, no gradient, no backdrop blur
- One emerald action color (#00a67b) on primary keys; emerald brand dot in the wordmark
- Blue (#82baff) for selection, focus lamp, customized marks, and info notices
- Status dots are data: amber = in progress, green = review, neutral gray = todo/done/cancelled
- Two voices: Inter Variable (UI) and JetBrains Mono Variable (machine)
- Radius scale 6/8/10/14; rows rounded-lg (10px), plates rounded-xl (14px)
- Flat surfaces; only dialog plates and the command palette cast drop shadows

## Colors

### Primary
- **Emerald** (#00a67b): the primary button face (Add project, Save, Get started), hover #00b589, border #006c50, white text. Also the brand dot set into the "ade" wordmark. Never a background tint, never decoration.
- **Info Blue** (#82baff): selection foreground, the focus lamp on the keyboard-active pane's tab, customized-shortcut marks and chords, info notice text. Selection wash is #173865 (and rgba(57,142,255,0.475) inside terminals).
- **Destructive Red** (#f27470): delete hovers, danger buttons on a #2a1b1a face with #964441 border, error strips.

### Status (data only)
- **Amber** (#dbae50): a task is in progress.
- **Green** (#59b358): a task awaits review.
- **Neutral gray** (#929192): todo, done, cancelled (dimmed), and every passive dot.

### Neutral
- **Background** (#111111): the main work area and the active tab / terminal well (via secondary-1).
- **Secondary** (#181818): the tab bar strip; terminals sit on it (--xterm-bg).
- **Tertiary** (#201f20): the sidebar column and modal plates' ground (background-2).
- **Row fills**: hover #282728, selected #302f30 (tertiary-1/2).
- **Elevated** (#252525): ghost buttons and modal action keys; hover #2f2f2f.
- **Borders**: #383738 (hairlines), #626162 / #787778 (stronger), #929192 (focus outline).
- **Foreground**: #e9e8e8 (primary), #d5d4d5 (body), #b8b7b8 (muted), #929192 (passive).

### Named Rules
**The One Emerald Rule.** Emerald appears on exactly one class of thing: the
key that creates, saves, or commits — plus the brand dot. If a screen shows
emerald on anything that isn't an action, it's wrong.

**The Dots Are Data Rule.** Amber, green, and red render only as status dots
and destructive affordances. The mapping is fixed: in_progress = amber,
review = green, todo/done/cancelled = neutral gray. Never use status
hues for accents, links, or badges.

## Typography

**UI Font:** Inter Variable — every UI word, from the wordmark to button text.
**Machine Font:** JetBrains Mono Variable — task names, paths, chords, tab
titles, palette hints, terminal text. Nerd Font stacks first inside terminals.

### Hierarchy
- **H1** (600, 20px/28px, -0.01em): the empty-state heading and modal titles use 16px/24px 600.
- **Body** (400, 14px/20px): default UI text; modal copy steps to 13px.
- **Semibold** (600, 13px): section headers inside settings, resource monitor header.
- **Micro label** (500, 12px, passive): sidebar section labels ("Pinned", "Projects").
- **Machine** (12px): task names, tab titles, inputs; 11px for chords and the ⌘K chip; 10px for palette hints and meter labels.

### Named Rules
**The Two Voices Rule.** Inter speaks for the operator; JetBrains Mono speaks
for the machine. Never set a path or chord in Inter, never set body copy in
mono.

## Layout

The shell is a two-column grid: a fixed 264px tertiary sidebar and a fluid
main area on the background color.

- **Sidebar:** 48px header with the wordmark (emerald brand dot) and 26px ghost keys plus the mono ⌘K chip; micro-label sections over 32px project rows and 30px task rows, all rounded-lg with 8px side padding; hover-only affordances (add-task, delete) fade in.
- **Main:** a 41px tab bar on secondary with a hairline below; panes split side by side with a 1px border between; terminal wells on secondary-1.
- **Fixed furniture:** the resource monitor is an elevated plate seated at the bottom-right (12px inset, rounded-lg, drop shadow). Dialog plates center over a 55% black backdrop.

## Elevation & Depth

Surfaces are flat tints separated by hairlines. Depth exists in exactly two
places: dialog plates and the command palette cast a soft drop shadow
(0 12px 40px rgba(0,0,0,0.55)) because they leave the chassis; the resource
monitor casts a smaller one (0 8px 24px). Nothing else floats — no glow, no
gradient, no backdrop blur.

## Shapes

Four radii cover every surface: 6px (small keys, chips), 8px (inputs, ghost
buttons, tab close), 10px (sidebar rows, primary/danger keys, resource
monitor), 14px (dialog plates, palette). Circles belong to dots only: 8px
status dots, 6px brand dot and focus lamp. All borders are 1px hairlines.

## Components

### Buttons
- **Primary:** emerald face, emerald-border, white text, rounded-lg, 6px 12px. Disabled becomes background-3 with passive text.
- **Ghost:** elevated face with a hairline border, muted text; hover brightens the face.
- **Danger:** destructive-background face, destructive text and border; hover deepens the face.

### Sidebar tree
- **Project rows:** 32px, rounded-lg, chevron + 13px 500 name + hover-only add-task key.
- **Task rows:** 30px, indented 30px, status dot first, mono 12px name, hover-only delete.
- **Selection:** tertiary-2 fill; hover tertiary-1.

### Tabs & panes
- **Tab bar:** 41px on secondary, hairline below; tabs are flat cells with a hairline right border.
- **Active tab:** secondary-1 fill with foreground text; the kind glyph (9px mono) brightens.
- **Focus lamp:** the keyboard-active pane marks its active tab with a 6px info-blue dot.
- **Titles:** 12px mono, capped at 180px; close is a 16px × that turns destructive on hover.

### Command palette
- **Plate:** 560px rounded-xl elevated plate, 15vh from the top; mono 13px input over a hairline; results as rounded-md rows; selected row fills tertiary-2; empty query shows the key legend in 10px mono under a hairline.

### Modals
- **Plates:** background-2, hairline border, rounded-xl, 16px 20px padding; 16px/24px 600 titles; recessed mono inputs; action row right-aligned with ghost/primary/danger keys.

### Terminals
- **Surface:** secondary-1 (#181818) with #e9e8e8 text, #e9e8e8 cursor, and the blue selection wash; 8px padding; 12px JetBrains Mono with Nerd Font stacks first.

### Resource monitor
- **Plate:** elevated, rounded-lg, hairline border, seated 12px from the bottom-right; 12px 600 header over a hairline; mono meters with 4px neutral tracks and passive fills.

## Do's and Don'ts

### Do:
- **Do** take every color from the token set; the neutral ramp covers every surface.
- **Do** spend emerald on one action class per view; blue on selection/focus/info; status hues on dots only.
- **Do** voice text by speaker: Inter for the operator, JetBrains Mono for the machine.
- **Do** keep surfaces flat and separated by hairlines; only plates and the palette lift.
- **Do** keep the four radii (6/8/10/14) and 32px/30px row rhythm.
- **Do** honor `prefers-reduced-motion` — the only transitions are 120ms color fades.

### Don't:
- **Don't** add glow, gradients, or backdrop blur.
- **Don't** introduce new accent hues; emerald, blue, amber, green, and red have fixed jobs.
- **Don't** set machine strings in Inter or body copy in mono.
- **Don't** cast drop shadows under work surfaces — only plates lift.
- **Don't** round rows below 10px or plates below 14px; the scale is the system.
- **Don't** decorate: no badges, no tinted washes, no icon tiles. Dots and hairlines carry all the meaning.
