# AGENTS.md — ade

Rust + Tauri 2 implementation of ade, an Agentic Development Environment (ADE).

## Before you start

Read these in order:

1. **`ARCHITECTURE.md`** — the authoritative reference. Traits, error type, async
   boundaries, event bus, DB schema, code patterns. If a ticket contradicts this file,
   this file wins (update the ticket).
2. **`PRD.md`** — product spec and epic inventory.
3. **`tickets-phase0.md`** — spawnable tickets for Phase 0.

## Workspace layout

```
Cargo workspace (12 crates):
  ade-core        domain modules (db, settings, projects, tasks, ...)
  ade-git         git2 worktrees, status, diff, commit, push
  ade-providers   provider registry (35 agents) + capability descriptors
  ade-acp         ACP client (Phase 2)
  ade-terminal    portable-pty, tmux
  ade-ssh         russh (Phase 3)
  ade-scheduler   cron (Phase 2)
  ade-integrations issue trackers (Phase 2)
  ade-telemetry   allowlisted events (Phase 2)
  ade-server      remote workspace daemon (Phase 3)
  ade-runtime     out-of-process workers (Phase 2)
  ade-app         Tauri 2 shell (main window, command modules, events)
app-frontend/        React + Vite webview UI
.github/workflows/   CI (fmt + clippy + test; frontend typecheck)
```

## Build & test

```bash
export PATH="$HOME/.cargo/bin:$PATH"   # cargo lives here (rustup)

make frontend    # npm install + build app-frontend/dist  (required before cargo build)
make dev         # Tauri app with Vite hot reload
make build       # cargo build (needs app-frontend/dist)
make test        # cargo test --workspace
make lint        # cargo clippy --workspace --all-targets -- -D warnings
make fmt-check   # cargo fmt --all --check
make check       # full merge gate: fmt + clippy + test
```

## Merge gate (Definition of Done — every ticket)

- `cargo fmt --check`
- `cargo clippy -D warnings`
- `cargo test` green
- Ticket's acceptance criteria + restart-survival test where noted
- Architectural decisions (esp. deviations from ARCHITECTURE.md or the
  reference) recorded as ADRs in `decisions/` — see `decisions/README.md`

## Conventions (short version — ARCHITECTURE.md is authoritative)

- `Result<T, ade_core::Error>` everywhere. No panics across crate boundaries.
- Versioned JSON for all JSON DB columns (`read_versioned`/`write_versioned`).
- Services are `Arc<dyn Trait>`; wired once in the `App` struct (ARCHITECTURE.md §7).
- Tauri commands are thin: call a domain fn, map error to `String`, return a DTO.
- No ad-hoc shell quoting — use the shared shell_escape module.
- Never delete the project root; worktree paths validated by realpath containment.
- Migrations are append-only; never hand-edit an applied migration.
- `git2::Repository` is `!Sync` — all git ops go through the serialized GitOps impl.
- Tests use `tempfile` / `:memory:`; never touch real app data paths.
- Architectural decisions (esp. deviations from ARCHITECTURE.md / the
  reference) get an ADR in `decisions/` (0001–0004 backfill the first tickets);
  record before merge, not after.

## Git strategy (decided)

`git2` (libgit2 bindings, v0.21) for worktree lifecycle (`worktree()`, `worktrees()`,
`find_worktree()`, `Worktree::prune()`). Shell out to the `git` CLI for ops git2
doesn't cover. gix was evaluated and rejected: as of 0.86 it has no worktree
add/list/prune.

## Frontend decisions (decided)

React + Vite + TypeScript. Zustand for state. CodeMirror 6 for the editor (with
`@codemirror/merge` for diffs). xterm.js for terminals.

> **Icons are placeholder-generated** (simple amber-bar-on-navy). They're fine for
> dev/compile; regenerate real branding before first bundling (E16).

## Reference implementation

`reference/emdash/` is a clone of `generalaction/emdash` (Electron + TS). Read the
matching `agents/risky-areas/*.md` page before touching DB, PTY, SSH, or
provider-spawning code.
