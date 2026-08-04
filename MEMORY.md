# MEMORY.md — ade

Project-level working memory. Newest entries first. If a fact here contradicts
AGENTS.md or ARCHITECTURE.md, the docs win — update this file (and the ticket if
one exists).

## Current state (2026-08-04)

- **Work tracking is GitHub issues only** (`jknack0/ade`) — `tickets-phase0.md`
  was retired 2026-08-04; its Appendix is preserved as `phase0-checklists.md`.
  New work = new issue (`phase:0`/`phase:2` + `size:*` labels, milestone "Phase 0").
- **Phase 0 is fully closed** (2026-08-04). Only #21 E2-11 (ACP path,
  phase 2) remains open. E14-01 shipped the keybinding registry; E1-07..
  E1-09 were re-audited against acceptance (E1-09 fix in `7a6c71c`).
- **HEAD (16b8e8f, 2026-08-04):** E14-01 keybinding registry — single
  window keydown listener + command registry (`app-frontend/src/lib/
  registry.ts`, default map in `commands.ts`), scope precedence
  modal > editor > conversation-view > task-view > project-view > global,
  user overrides in `view-state:app:keybindings`. Settings modal (⌘,)
  remaps chords; hints render from the live registry. E2-10's
  `lib/shortcuts.ts` was superseded and deleted.
- **E2-10 (1df8f15):** task-switch nav (⌘⌥↑/↓, visible-tree order) +
  per-task tabs (conversations as tabs, registry in `app-frontend/src/
  lib/tab-registry.tsx`, tab state persisted under
  `view-state:task:<id>:tabs`).
- **E2-08 removed the standalone conversation list** — conversations now live
  under tasks (create-task command + sidebar).
- **E2-07 shipped terminal persistence/resume** — boot rehydration orchestration,
  tmux kill, remote hook, dirty-check on worktree open (ADR-0022 for the
  sync-command decision).

## Key decisions (see decisions/ for full ADRs, 0001–0023)

- **Git strategy:** `git2` v0.21 for worktree lifecycle (add/list/prune); shell
  out to `git` CLI for everything else. `gix` rejected (no worktree ops as of 0.86).
- **git2::Repository is `!Sync`** — all git operations go through the serialized
  GitOps impl (ADR-0003).
- **SQLite migrations are append-only** — never hand-edit an applied migration
  (ADR-0001).
- **Settings** use layered precedence + KV store (ADR-0002).
- **Tauri commands are thin and synchronous** where possible (ADR-0022); domain
  fns return `Result<T, ade_core::Error>`, commands map errors to `String` and
  return DTOs.
- **Terminal persistence** via tmux; resume across restarts (ADR-0021).
- **Task deletion teardown** semantics in ADR-0023.

## Conventions that bite

- `cargo` lives at `~/.cargo/bin` (rustup) — export PATH before cargo commands.
- Frontend UI verification: drive `vite` dev in a headless browser with a
  mocked Tauri backend (`window.__TAURI_INTERNALS__`, seeded via
  `evaluateOnNewDocument`) — the frontend has no test runner; restart
  survival is checked by re-seeding persisted view-state and reloading.
- `make frontend` is **required before `cargo build`** — the app embeds
  `app-frontend/dist`.
- Icons are **placeholder-generated** (amber bar on navy) — fine for dev, must be
  regenerated before first bundling (E16).
- No ad-hoc shell quoting — use the shared `shell_escape` module.
- Worktree paths validated by realpath containment; never delete the project root.
- Versioned JSON (`read_versioned`/`write_versioned`) for all JSON DB columns.
- Tests use `tempfile` / `:memory:` — never touch real app data paths.
- Before touching DB, PTY, SSH, or provider-spawning code, read the matching
  `reference/emdash/agents/risky-areas/*.md` page (reference impl is a clone of
  `generalaction/emdash`, Electron + TS).

## Docs map

- `AGENTS.md` — onboarding + merge gate (fmt, clippy -D warnings, cargo test).
- `ARCHITECTURE.md` — authoritative reference: traits, error type, async
  boundaries, event bus, DB schema. Ticket contradicting it → ticket loses.
- `PRD.md` — product spec + epic inventory.
- GitHub issues — the only work list (`gh issue list -R jknack0/ade`).
- `phase0-checklists.md` — cross-cutting Phase 0 process checklists (ex-Appendix).
- `decisions/` — ADRs 0001–0023; record new ones before merge, not after.
