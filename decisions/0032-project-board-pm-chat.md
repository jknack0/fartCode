# ADR-0032: Project board & PM chat — local-first issues, drag-dispatch, proposal-block writes

**Status:** accepted (supersedes the §13 "project-level chat" design; epic E17)

## Context

ARCHITECTURE.md §13's original project-chat design was written at bootstrap
(2026-08-03), before two landslides: the terminal-only task-view pivot (#39,
which deleted the Phase-0 conversation surfaces §13 assumed the project view
would mirror) and the ACP conversation path (E2-11, which made agents external
CLI processes — so §13's "register 14 tools for the project agent" had no
mechanism short of MCP).

A re-grill on 2026-08-06 re-scoped the feature: the project view is not a chat
with a terminal, it is a **planning surface** — a Jira-style board of issues
plus a PM-style chat used to grill requirements, author PRDs, and break them
into issues. Dragging a card into In Progress spawns an agent in a worktree
that starts implementing; blocked-by edges between issues are first-class.

The PRD's E7 model assumed issues live in external trackers (Linear/Jira/
GitHub) with `tasks.linked_issue` pointing outward. fartCode now becomes the
tracker: issues are local-first rows, and tracker connections (when E7/E8
land) become sync/export adapters rather than the store.

## Decision

1. **Local-first issue store.** New `issues` + `issue_dependencies` tables in
   fartcode-core (append-only migration). Blocked status is **derived at read
   time** (any blocker not in Done ⇒ blocked), never stored — unblocking is
   automatic when the blocker lands in Done. Cycles are rejected at edge
   creation (DFS over the adjacency map). Board drag is a user action, so
   dispatched tasks keep `created_by = 'user'`; `tasks.linked_issue` gains a
   local-issue variant pointing at the issue id.
2. **Five lanes, drag-to-dispatch.** Backlog / Ready / In Progress / In
   Review / Done. Dragging a card into In Progress composes the existing
   machinery: `create_task` (issue-derived name, structured prompt packet) +
   agent launch. Dragging a **blocked** card in prompts a confirm dialog, not
   a hard stop.
3. **The board never kills.** Card moves change issue status only; a running
   agent is never torn down by a drag (teardown stays on the explicit
   ⌘Backspace task flow). Re-dragging a card back into In Progress
   **reattaches** to its existing linked task — never spawns a second
   worktree. In Progress → Done on a running card warns but is allowed.
4. **Auto-flip to In Review.** A card moves In Progress → In Review when the
   linked task's agent completes: ACP conversation turn-complete (E2-11-4
   event) or terminal-agent PTY process exit. No idle-timeout heuristics;
   manual drag always works.
5. **Chat writes via proposal blocks, not tools.** The PM agent's system
   prompt defines a fenced ` ```fartCode-proposal ` JSON block (PRD summary +
   issue list with titles/bodies/edges + optional per-issue provider/model).
   The transcript detects it and renders an interactive approval card
   (edit titles, drop issues); Approve writes through normal fartcode-core
   commands. Provider-agnostic (any adapter that prints text works), no new
   processes, hard human gate. An MCP tool server is the E10-era upgrade.
6. **PRD = markdown in the repo.** The PM agent (running in the project
   root, per §13's surviving "where it runs" answer) writes
   `docs/prds/<slug>.md` with its own file tools; approved issues reference
   the path + section. PRDs diff in git and sub-task agents read them with
   normal file access. No DB blob, no parent-issue cramming.
7. **Dispatch packet by reference.** The spawned agent's initial prompt is
   built by fartcode-core: issue title + body + acceptance criteria + PRD path +
   one-line summaries of Done blockers + branch/worktree conventions footer.
   No inlined PRD copy to go stale. Provider defaults to the project's
   `defaultAgent`, with per-issue override carried on the issue from the
   approved proposal.
8. **Layout.** Board is the primary project view; PM chat is a collapsible
   right panel following the ChangesSidebar pattern. Card detail swaps into
   the right panel. Project-root terminals (§13's old "Terminal: Yes"
   answer) are deferred — agent terminals live in their task views.

## Consequences

- §13 is rewritten: the 14-tool registered surface, "same layout as tasks",
  and the sub-task auto-post-to-chat hook are dead; board auto-flip covers
  completion visibility.
- E7/E8 (external trackers, GitHub accounts) change shape when they arrive:
  from "source of issues" to "sync/export adapters" over the local store.
- The proposal-block contract is prompt-level, so it needs a golden-file
  parse test (malformed blocks must surface as plain transcript text, never
  throw) and a PM system prompt that is versioned with the parser.
- fartCode dogfoods on itself: this repo's own GitHub issues remain its dev
  tracker; the board's first real tenant is fartCode managing *other* projects.
  GitHub sync of local issues is explicitly out of scope for E17.
