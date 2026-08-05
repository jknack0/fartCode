# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

> Tauri 2 desktop app (macOS, Windows, Linux) whose UI is a webview-rendered
> React app. The design language is web, not OS-native; no per-OS chrome or
> platform widgets are planned.

## Users

Solo developer on their own machine. One person running several coding-agent
CLIs in parallel across their own projects, using ade as the daily cockpit for
creating, supervising, reviewing, and shipping agent work.

## Product Purpose

ade is a free, open-source Agentic Development Environment. It does not run its
own model — it orchestrates external coding-agent CLIs (Claude Code, Codex, Amp,
omp, …) in parallel, each isolated in its own Git worktree, and gives the user
one cockpit to create tasks, review diffs, commit/push/PR, monitor CI, schedule
recurring runs, manage prompts/skills/MCP servers, and run agents on remote
machines over SSH. Success means the user can run many agents at once without
them colliding and without losing state — everything persists across restarts
(terminals, editor buffers, tmux sessions, scheduler).

## Positioning

Any agent, one cockpit. ade orchestrates whatever coding-agent CLIs the user
already has installed (35 providers in the registry, 22 ACP-capable) and runs
no model of its own. A neighboring product that ships or fronts a single model
could not truthfully claim this neutrality.

## Operating Context

- Workflow: add project (local dir, GitHub clone, or SSH host) → Add Task →
  ade creates a Git worktree and spawns the chosen agent in a terminal inside
  it → the task becomes a live workspace (terminals, editor, diff view) →
  review, stage, commit, push, open PR, watch CI checks without leaving the app.
- Tasks come from branches, issue-tracker tickets, or cron automations.
- Keyboard-driven desktop use: command palette, task switching, pane splits,
  chords for terminals/splits/agents. Scope precedence for shortcuts:
  modal > editor > task-view > project-view > app-view > global.
- Local-first: SQLite app DB, OS keychain for secrets, offline-capable (no CDN
  dependencies in the shipped frontend).

## Capabilities and Constraints

- Capabilities: per-task Git worktree isolation; terminal-first task workspaces
  (current UI); unified/split diff with inline editing; PR + CI monitoring;
  line comments linking review notes to tasks; reusable prompts/skills/MCP
  configs synced to agents' native config locations; cron automations with run
  history; remote execution over SSH; persistence-first restart survival.
- Non-goals (v1): no own coding agent/model, no cloud/enterprise offering, no
  editor LSP features / ⌘P quick-open / drag-and-drop files.
- Technical: Rust core + Tauri 2 shell; React + Vite + TypeScript frontend
  (Zustand, CodeMirror 6, xterm.js); SQLite with append-only numbered
  migrations; `Result<T, Error>` across crate boundaries.
- Terminology: project, task, worktree, workspace, conversation (chat session;
  currently not surfaced in UI), provider (agent CLI), automation, ACP (Agent
  Client Protocol).
- Open decisions (deliberately not confirmed as binding on 2026-08-05):
  - Whether the task workspace stays terminal-first or reintroduces chat
    surfaces (chat UI was removed 2026-08-04; the backend conversation layer
    and ACP path remain).
- Resolved 2026-08-05 (user decision): the visual system is the "emdash
  world" — the reference implementation's working surface adopted at full
  fidelity (neutral charcoal chassis #111111/#201f20/#181818, emerald primary
  action, blue selection/focus, amber/green status dots, Inter + JetBrains
  Mono). It supersedes both the earlier "Signal" system and the short-lived
  "INSTRUMENT" experiment. Recorded in DESIGN.md.

## Brand Commitments

- Name: **ade**.
- Free and open-source; no pricing tiers, no hosted/cloud offering. Local app
  only.
- App icons are placeholder-generated (amber bar on navy) and must be
  regenerated before first bundling (epic E16) — they are not brand assets.
- No other identity constraints confirmed; visual direction is an open
  decision (see above).

## Evidence on Hand

- `PRD.md` — full feature inventory, epics, reference-implementation map.
- `ARCHITECTURE.md` — authoritative technical reference (traits, DB schema,
  event bus, code patterns).
- `MEMORY.md` — current milestone state and conventions.
- `decisions/` — ADRs 0001–0027.
- `reference/emdash/` — clone of generalaction/emdash (Electron reference
  implementation) incl. architecture docs.
- `app-frontend/src/styles.css` — incumbent "Signal" token set (current
  implementation, not a confirmed commitment).
- GitHub issues (`jknack0/ade`) — the single work list.
- Absences future work must not fabricate: no testimonials, customers,
  benchmarks, pricing, licensing terms, marketing imagery, or screenshots of a
  shipped product exist. ade.ai landing/docs were research sources for the PRD,
  not owned assets.

## Product Principles

1. **Orchestrate, don't imitate.** ade's value is the cockpit, not a model.
   Every feature must work through the user's existing agent CLIs.
2. **Parallel isolation is the core mechanic.** Per-task worktrees, terminals,
   and state must never collide; anything that breaks isolation breaks the
   product.
3. **Persistence-first.** Restart survival (terminals, buffers, sessions,
   scheduler) is a feature, not a nicety — state loss reads as a bug.
4. **Local and private.** Secrets in the OS keychain, telemetry allowlisted
   with easy opt-out, no dependency on cloud services for core function.
5. **Built for one pair of hands.** Solo-developer ergonomics: dense,
   keyboard-driven, zero collaboration theater, no setup ceremony.
