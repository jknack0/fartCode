# MEMORY.md — ade

Project-level working memory. Newest entries first. If a fact here contradicts
AGENTS.md or ARCHITECTURE.md, the docs win — update this file (and the ticket if
one exists).

## Current state (2026-08-04)

- **Work tracking is GitHub issues only** (`jknack0/ade`) — `tickets-phase0.md`
  was retired 2026-08-04; its Appendix is preserved as `phase0-checklists.md`.
  New work = new issue (`phase:0`/`phase:2` + `size:*` labels, milestone "Phase 0").
- **Phase 0 is fully closed** (2026-08-04). **Phase 2 in progress:** E2-11
  broken into #28–#33; #28 (2827012), #34 (9041aad), #29 (2ca862a) done.
  **#35 E2-12 interactive task terminal done (713dfbd) + terminal-first
  default (5ea481d)** — selecting a task opens a shell in its worktree;
  chat is summoned-only (⌘T/⌘D), never auto-opened; no + button on the
  tab bar. Work-inside-ade path for agents like omp. Next E2-11:
  **#30 E2-11-3** (SessionManager + session-id persistence).
- **HEAD (2827012, 2026-08-04):** E2-11-1 — ade-acp is a real ACP v1
  client: stdio JSON-RPC transport + client lifecycle (initialize/new/load/
  prompt/cancel/set_mode/set_config_option) + scoped fs handlers +
  permission surfacing. Wire types from `agent-client-protocol-schema`
  v1.6 (ADR-0024); test fixture `ade-acp/src/bin/fake_acp_adapter.rs`;
  8 integration tests in `ade-acp/tests/protocol_integration.rs`.
- **E14-01 (16b8e8f):** keybinding registry — scope precedence
  modal > editor > conversation-view > task-view > project-view > global,
  user overrides in `view-state:app:keybindings`. E2-10's
  `lib/shortcuts.ts` was superseded and deleted.
- **E2-08 removed the standalone conversation list** — conversations now live
  under tasks (create-task command + sidebar).
- **E2-07 shipped terminal persistence/resume** — boot rehydration orchestration,
  tmux kill, remote hook, dirty-check on worktree open (ADR-0022 for the
  sync-command decision).

## Key decisions (see decisions/ for full ADRs, 0001–0024)

- **Git strategy:** `git2` v0.21 for worktree lifecycle (add/list/prune); shell
  out to `git` CLI for everything else. `gix` rejected (no worktree ops as of 0.86).
- **git2::Repository is `!Sync`** — all git operations go through the serialized
  GitOps impl (ADR-0003).
- **ACP wire types** come from the official `agent-client-protocol-schema`
  crate; transport/client/SessionManager are ours (ADR-0024, PRD §10.1
  resolved). Workspace `rust-version` = 1.88 because of it.
- **keyring v3 needs a backend feature** (`apple-native` on macOS,
  `sync-secret-service` on Linux) — without one it silently uses a mock
  store and secrets vanish across calls. Secrets never cross a Tauri
  command boundary (maskedSecret DTOs only).
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
