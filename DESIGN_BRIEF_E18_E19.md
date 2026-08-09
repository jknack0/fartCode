# Design review brief — E18 configurable pipeline columns · E19 feature dossiers

For: design review of `decisions/0037-configurable-pipeline-columns.md` and
`decisions/0038-feature-dossiers.md` (both **proposed**; product decisions
settled, frames needed). System: The Quiet Terminal (DESIGN.md binding —
tokens in `:root`, motion is `fc-pulse` + `fc-caret` only). Tickets: epic
[#60](https://github.com/jknack0/fartCode/issues/60) / epic
[#69](https://github.com/jknack0/fartCode/issues/69); the design-gated ones
are labeled `design-gate`.

What changed conceptually: the board's five lanes become per-project data.
A column is a **shelf**, an **agent step** (own system prompt / provider /
model / effort; fires on drop or after a confirm; holds or auto-advances on
settle), or a **human gate**. Seeded default: Backlog · Ready · Quick ·
In Progress · In Review · Done — behavior-identical to today until edited.
Feature dossiers give every card a repo-markdown memory file agents write
into; the app renders and indexes it.

## Frames needed

### 1. Board at N columns (#66)
- Any column count (4–9 realistic). Wide: grid generalizes from `repeat(5)`.
- **Narrow (settled):** strip + single-column view scroll horizontally,
  h/l walks every column, strip follows focus. Never caps, never truncates.
- Done's special-cased color (left_nav README :65) needs a generalization:
  what does any `counts_as_done` column look like?
- New card state: **step-done** (agent settled, column holds — awaiting a
  human drag). Distinct from needs-you and from review.

### 2. Column headers (#68)
- An agent-step column that fires on drop (no confirm) must show its
  provider/model on the header — confirm-free spend needs visibility.
  Quick is the seeded instance.

### 3. Confirm overlays (#68)
- Queue-confirm (drop on a queue-mode step): names agent, model, branch —
  same shape as today's blocked-dispatch confirm, copy templated from
  column config. Blocked and done-live confirms likewise lose hardcoded
  lane names.

### 4. Columns editor in project settings (#67)
- Add / rename / reorder / delete; kind; counts-as-done; landing; on-enter
  (run/queue); on-settle (hold/advance); step config incl. a system-prompt
  editor and provider/model/effort pickers.
- Delete-with-issues is refused by the backend — needs its guidance moment.
- No board-side editing in v1 (settled).

### 5. First-dispatch consent card (#74)
- Copy direction (settled): "this feature will keep a dossier — write the
  convention files to your repo?" Declining runs the step without memory;
  project settings carries the same switch both ways.

### 6. Card detail dossier section + ⌘K feature hits (#75)
- Card detail gains a dossier section (timeline + step decisions).
- ⌘K feature hit row (new item type); **Enter opens the card detail**
  (settled — one destination for live and landed features).

### 7. Memory value dashboard (#76)
- Four local metrics: memory citations, re-ask rate, context tokens saved,
  time-to-land trend. Headline framing: "your project memory saved N
  re-explanations this month." Time-to-land never shown without its
  attribution caveat. Placement open (settings? project view?).

## Constraints that survive
- The board never kills a running agent; teardown stays ⌘⌫.
- Flyout shows only in-flight work — now defined as "card in an agent-step
  column with a live session, or needs-you."
- The only red action label in the app remains `⌘⌫ delete`.
