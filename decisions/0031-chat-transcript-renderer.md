# ADR-0031: Chat transcript renderer — two-tier memoized list, composer-docked permissions, docked plan

**Status:** accepted (ticket E2-11-6, #33)

## Context

#33 builds the structured-chat surface over the E2-11-4/5 live models: a
transcript renderer with streaming, tool-call cards, a plan view, permission
prompts, and empty/error states. The reference implementation
(`reference/emdash/packages/chat-ui/`) is a 1.8k-line Solid engine:
measurement-driven virtualization (rAF read/measure/write scheduler,
per-unit height maps, overscan windows), portal slots for a React composer,
and a per-word streaming animation frontier.

## Decision

Port the reference's **concepts**, not its engine:

1. **Surface = a `conversation` tab kind.** The tab id IS the conversation
   id (a DB row), so tabs persist in `view-state:task:<id>:tabs` and survive
   restarts. `ensureTabs` reconcile branches on kind: terminal tabs respawn
   PTYs (ADR-0028), conversation tabs restore as-is and rehydrate the
   transcript from `acp_history` on mount. ⌘⇧A (`open-conversation`,
   task-view scope) opens/focuses the task's ACP conversation, creating one
   with the first ACP-capable provider when none exists (the runtime type
   itself stays a server-side decision, E2-11-5).
2. **Two-tier rendering instead of virtualization.** Committed turns render
   through `SettledTurn` — `React.memo` with a comparator on
   `(turn.id, items.length, outcome.kind)`, sound because the reducer never
   edits a committed turn — while the active turn re-renders per
   `acp:transcript` snapshot. Chunk-by-chunk appends therefore touch only
   the active turn's subtree (verified: settled-turn DOM nodes are identity-
   stable across streaming snapshots). The structure (flat, keyed rows) is
   virtualization-ready if Phase-4 transcripts outgrow it.
3. **Permission prompts dock at the composer** (reference PermissionBand
   placement): the transcript row only shows an awaiting glyph on the gated
   `toolCallId`; the actionable card renders above the input with one
   button per ACP option (`allow*` → primary, `reject*` → danger), wired to
   `acp_resolve_permission` with optimistic removal.
4. **Plan renders as a docked strip above the composer**, not an inline
   transcript row (deviation from the reference). Our reducer keeps the
   plan as a session slice (`LiveModels.plan`, ADR-0029) rather than a
   transcript item, and providers replace entries wholesale — a docked
   strip shows current state without re-anchoring scroll position.
5. **No conversation-view keybinding scope.** The composer is a native
   textarea, so the E14 editor scope already owns it: Enter sends,
   Shift+Enter breaks, and the global ⌘Enter `send-context` command keeps
   working (it routes to the same conversation).
6. **Plain-text rendering.** No markdown pipeline, no per-word streaming
   animation, no syntax/diff highlighting — file changes preview raw
   `newText` capped at 12 lines. The diff renderer is E4-04's job.

## Consequences

- Settled history never re-renders during streaming; the cost per snapshot
  is proportional to the active turn only.
- A future virtualizer can slot in behind `SettledTurn` without changing
  item renderers.
- Transcript scroll offset is not restored across task switches (stick-to-
  bottom re-pins to latest); the reference's height-map restoration was
  deliberately dropped.
- `list_providers` order decides the default conversation provider; a
  provider picker is future UI.
