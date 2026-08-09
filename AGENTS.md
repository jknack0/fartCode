# AGENTS.md — fartCode

Rust + Tauri 2 implementation of fartCode, an Agentic Development Environment (ADE).

## Before you start

Read these in order:

1. **`ARCHITECTURE.md`** — the authoritative reference. Traits, error type, async
   boundaries, event bus, DB schema, code patterns. If a ticket contradicts this file,
   this file wins (update the ticket).
2. **`PRD.md`** — product spec and epic inventory.
3. **GitHub issues** — the single source of truth for work
   (`gh issue list -R jknack0/fartCode`; Phase 0 tickets use `phase:0`/`phase:2` +
   `size:*` labels, milestone "Phase 0"). New work gets a new issue — no ticket
   docs. Cross-cutting Phase 0 checklists live in `phase0-checklists.md`.
4. **`MEMORY.md`** — project-level working memory: current milestone state,
   key decisions, and conventions that bite. Check it before starting work; update
   it when you land something durable (newest entries first).

## Workspace layout

```
Cargo workspace (12 crates):
  fartcode-core        domain modules (db, settings, projects, tasks, ...)
  fartcode-git         git2 worktrees, status, diff, commit, push
  fartcode-providers   provider registry (35 agents) + capability descriptors
  fartcode-acp         ACP client (Phase 2)
  fartcode-terminal    portable-pty, tmux
  fartcode-ssh         russh (Phase 3)
  fartcode-scheduler   cron (Phase 2)
  fartcode-integrations issue trackers (Phase 2)
  fartcode-telemetry   allowlisted events (Phase 2)
  fartcode-server      remote workspace daemon (Phase 3)
  fartcode-runtime     out-of-process workers (Phase 2)
  fartcode-app         Tauri 2 shell (main window, command modules, events)
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

- `Result<T, fartcode_core::Error>` everywhere. No panics across crate boundaries.
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

## Tauri commands and the main thread

A non-`async` `#[tauri::command]` compiles to `ExecutionContext::Blocking`: the
body is inlined into the invoke handler and resolved synchronously. The invoke
handler runs on the IPC thread, and on macOS that thread is the **main thread** —
blocking there stalls the NSRunLoop and the window stops repainting. A beachball,
not a spinner (#80).

- **Subprocess, network, sleep, process spawn, or unbounded filesystem work →
  `async` + `tauri::async_runtime::spawn_blocking`.** All of `fartcode-git`
  shells out to the `git` CLI, so every git-backed command qualifies. `async`
  alone is not the fix — it only relocates the stall onto an async-runtime
  worker; the blocking body has to leave the thread.
- **Only cheap, bounded work stays synchronous**: a pure function, an in-memory
  lookup, a short SQLite statement, one small file read/write.
- **Never hold the DB connection guard across an `.await`.** `db.conn().lock()`
  is a non-reentrant mutex — a re-entrant lock deadlocks the app. Take the guard
  inside the `spawn_blocking` closure and drop it there.
- Git network ops need a timeout: `Command::output()` has none, so an
  unreachable remote hangs the app indefinitely.

`fartcode-app/tests/no_blocking_tauri_commands.rs` enforces this. It parses the
`generate_handler!` list and the command bodies as text — no GUI, no app
instance — and runs under `cargo test --workspace`, so it gates CI and
`make check`. A new non-`async` command fails the build until you either make it
`async` + `spawn_blocking` or add it to the test's `SYNC_OK` list with a
one-line justification. Adding a `SYNC_OK` entry is a claim that you walked the
whole call path, not just the command body.

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
