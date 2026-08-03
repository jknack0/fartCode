# PRD — ade, an Agentic Development Environment (Rust + Tauri)

**Status:** v0.3 (architecture decisions + expanded Phase 0 tickets) · **Last updated:** after adding [`ARCHITECTURE.md`](./ARCHITECTURE.md)
**Spawnable tickets:** Phase 0 (E0 + E1 + E2 + E3 subset + E14-01) expanded in [`tickets-phase0.md`](./tickets-phase0.md)
**Author:** Generated from research of https://ade.ai (landing + docs) + the reference repo source
**Purpose:** Feature baseline + implementation plan so individual tickets can be spawned per epic.
**Target stack:** Rust core + Tauri 2 (web frontend). macOS, Windows, Linux.

---

## 1. Overview

ade is a free, open-source **Agentic Development Environment (ADE)** desktop app. It does **not** run its own model — it orchestrates external coding-agent CLIs (Claude Code, Codex, Cursor, Amp, …) in parallel, each isolated in its own Git worktree, and gives the user one cockpit to create tasks, review diffs, commit/push/PR, monitor CI, schedule recurring runs, manage prompts/skills/MCP servers, and run agents on remote machines over SSH.

The reference implementation (`github.com/generalaction/emdash`, Apache-2.0) is an **Electron + TypeScript + React** app (pnpm/Nx monorepo, SQLite via Drizzle, `node-pty`, `ssh2`, ACP chat). We are building a feature-compatible baseline in **Rust + Tauri 2**. This PRD inventories the full feature surface, then maps it against the reference repo's actual module structure so tickets can be spawned with implementation-aware detail.

### Sources

- Landing + docs: https://ade.ai/ (all `/docs` pages, July 2026 revision)
- Reference repo: `generalaction/emdash` (clone at `reference/emdash/`), incl. `AGENTS.md`, `agents/` architecture docs, `apps/emdash-desktop/` source, `packages/` workspace

### The one-paragraph product

User adds a project (local dir, GitHub clone, or SSH host). Click **Add Task** — ade creates a Git worktree (local default `~/ade/worktrees`, remote `<project>/.ade/worktrees`), spawns the chosen agent in a terminal inside it, and the task becomes a live workspace: terminal(s), conversations, file editor, diff view, in-app browser previews, PR/CI monitoring — isolated per task. Tasks come from branches, issue-tracker tickets, or cron automations. Everything persists across restarts (terminal state, editor buffers, tmux sessions, scheduler).

---

## 2. Goals & Non-Goals

### Goals (v1)
1. Run **any installed coding-agent CLI** in per-task Git worktrees, in parallel, isolated terminal/conversation/review state.
2. Full **review-and-ship loop**: diff, stage, commit, push, open PR, watch CI checks.
3. **Reusable agent resources**: prompts, skills (Agent Skills), MCP configs — synced to agents' native config locations.
4. **Scheduled work**: cron automations with run history + self-healing scheduler.
5. **Remote execution**: SSH connections, all-on-remote projects, per-task provisioned workspaces (BYO infra).
6. **Persistence-first**: terminal state, editor buffers, tmux sessions, scheduler survive restarts.
7. Privacy-respecting, allowlisted **telemetry** with easy opt-out.

### Non-Goals (baseline)
- Running our own coding agent/model — we orchestrate existing CLIs.
- **Cloud / Enterprise** hosted offerings (out of scope; we build the local app).
- Editor **LSP features**, ⌘P quick-open, drag-and-drop files (their roadmap, deferred).
- **Note:** ACP (Agent Client Protocol) is **in scope** — the reference repo already ships ACP-based structured chat (22 of 35 providers ACP-capable). The docs' roadmap page is stale; the repo is the source of truth. MVP may fall back to TUI/PTY chat, but the plan must have an ACP path (see §5.5, E2/E3).

---

## 3. Reference implementation map

How the reference repo is organized, and where each piece lands in our plan. All paths relative to `apps/emdash-desktop/` unless prefixed.

| Their area | What it is | Our epic / crate |
|---|---|---|
| `src/main/` (Electron main) | RPC controllers + domain services + DB + PTY + SSH + updater | `ade-app` Tauri backend |
| `src/preload/` | Tiny typed bridge: `invoke`/`eventSend`/`eventOn` | Tauri `invoke`/`emit` (no bridge needed) |
| `src/renderer/` | React app: `app/`, `features/`, `lib/`, typed RPC client | web frontend (`app-frontend/`) |
| `src/shared/` | Provider registry, IPC primitives, MCP/skills types, events, telemetry | `ade-core` shared types + `src/shared` equivalent |
| `src/main/core/*` (45 domains) | One dir per domain, each with `controller.ts` + services | `ade-core` modules (§5.6) |
| `src/main/db/` | Drizzle schema + migrations (`drizzle/0000..0019`), `emdash4.db` | `ade-core::db` (rusqlite, migration runner) |
| `src/main/core/pty/` | `local-pty` (node-pty) / `ssh2-pty` / tmux / env allowlist | `ade-terminal` (portable-pty) |
| `src/main/core/ssh/` | SSH connection mgmt, config parse, client proxy | `ade-ssh` (russh) |
| `src/main/core/acp/` | Out-of-process ACP worker host (local + SSH transports) | `ade-acp` (new crate) |
| `src/main/core/agent-hooks/` | HTTP hook server, hook config writer, notifications | `ade-core::agent_hooks` |
| `src/main/core/dependencies/` | Agent CLI detection/install/update (host deps) | `ade-core::dependencies` |
| `src/main/core/automations/` | Scheduler, runs, run transitions | `ade-scheduler` |
| `src/main/core/projects/` (+ `worktrees/`, `settings/`) | Project provider pattern, worktree service | `ade-core::projects` |
| `src/main/core/tasks/`, `conversations/`, `terminals/` | Task lifecycle, session supervisor, lifecycle scripts | `ade-core::tasks/conversations/terminals` |
| `src/main/core/git/`, `github/`, `pull-requests/` | Git ops, GitHub API (gh CLI), PR sync engine | `ade-git` + `ade-core::github/pr` |
| `src/main/core/integrations/`, `issues/`, `linear/`, `jira/` | Issue-tracker integrations, issue provider registry | `ade-integrations` |
| `src/main/core/mcp/`, `skills/`, `prompt-library/` | MCP config sync, skills catalog+install, prompt KV | `ade-core::mcp/skills/prompts` |
| `src/main/core/search/`, `editor/`, `view-state/`, `resource-monitor/` | FTS search, editor buffer drafts, view-state KV, resource sampler | `ade-core::search/editor/view_state/resource_monitor` |
| `src/main/core/browser/`, `preview-servers/`, `port-forwards/` | Webview browser, dev-server URL detection, SSH port forwards | `ade-core::browser/preview/port_forwards` |
| `src/main/core/secrets/`, `account/`, `provider-accounts/`, `shared/oauth-flow` | Encrypted secrets, account + provider token registry, OAuth | `ade-core::secrets/account` (keyring) |
| `src/main/core/fs-watch/`, `files/`, `workspaces/` | File watcher worker, file tree, workspace bootstrap | `ade-core::fs/files/workspaces` (notify) |
| `apps/workspace-server/` + `packages/core/src/workspace-server/` | Remote daemon exposing git/files/deps/ACP over `@emdash/wire` (SSH-forwarded socket) | `ade-server` (new bin crate) |
| `packages/core/src/acp/` | Transport-agnostic ACP client, transcript reducer, session machine | `ade-acp` |
| `packages/plugins/src/agents/` | Provider registry: 35 agents, capability descriptors | `ade-providers` (Rust registry) |
| `packages/chat-ui/` (Solid) | Chat transcript renderer | `app-frontend` chat components |
| `packages/ui/`, `packages/shared/`, `packages/wire/`, `packages/runtime/` | UI kit, shared primitives, live-model wire, out-of-process runtimes | `app-frontend` + `ade-runtime` workers |

### Patterns worth porting verbatim (proven in production)

1. **Typed RPC + topic events** — controllers per domain registered in one router; renderer gets a proxy-based typed client. In Tauri: one command module per domain, typed event names with topic suffixing (`eventName.{topic}`).
2. **`Result<T, E>` everywhere** — no thrown exceptions across the IPC boundary; controllers convert `Result` → IPC-compatible responses.
3. **Versioned JSON columns** — every JSON blob in SQLite carries a `version`; readers run a sequential upgrade chain; newer data surfaces as `future-version` for graceful degradation; writers always serialize latest. Port as-is (`serde` + `versioned` wrapper).
4. **Provider pattern for multi-backend domains** — `local-fs.ts`/`ssh-fs.ts`, `local-pty`/`ssh2-pty`, `local-conversation`/`ssh-conversation`: interface + `impl/` dir. Same in Rust (trait + local/ssh impls).
5. **Workspace-server wire protocol** — semver `PROTOCOL_VERSION`, `initialize` handshake on every connect, `min(clientMinor, serverMinor)` feature negotiation, `PROTOCOL_INCOMPATIBLE` with `upgrade-client|upgrade-server` action. Language-agnostic; port as-is (§5.7).
6. **Agent hooks, not output sniffing** — agent status comes from explicit hooks/plugins, never inferred from terminal text.
7. **PTY env allowlist + shell-escaping helpers** — `pty-env.ts` allowlist; never ad-hoc quoting; path containment via `realpath-containment.ts`.
8. **Updater invariants** — two feeds, channel-manifest naming, no manual channel override.
9. **Drizzle-style migrations** — numbered SQL migrations, sha256-hashed journal in DB, FTS tables version-gated via `kv` keys.

---

## 4. Architecture (Rust + Tauri 2)

### 4.1 Process model

```
┌────────────────────────────────────────────────────────────┐
│ Tauri 2 app                                                 │
│                                                             │
│  Frontend (webview: React or Svelte + TS)                   │
│   • Shell: sidebar, task tabs, panes, modals, command pal.  │
│   • Terminal (xterm.js) · Editor (CodeMirror 6 / Monaco)    │
│   • Diff renderer · file tree · chat UI (ACP transcripts)   │
│   • In-app browser tab (Tauri webview)                      │
│        ▲ typed commands + event streams                     │
│  Rust core (single Tauri backend process)                   │
│   • Domain modules (§5.6), SQLite, keychain                 │
│   • PTY manager (portable-pty), tmux, fs watcher (notify)   │
│   • Agent process hosts: TUI (PTY) and ACP (stdio worker)   │
│   • SSH/SFTP client (russh), port forwards, wire client     │
│   • Scheduler, telemetry, resource monitor                  │
└────────────────────────────────────────────────────────────┘
  │ ACP stdio worker (ade-acp-runtime, child process)
  │ SSH-forwarded Unix socket → ade-server daemon (remote)
```

All product logic lives in Rust; the webview renders. Long-lived streams (PTY output, ACP updates, git events, hook events) flow over Tauri events. This mirrors their Electron architecture: **Tauri commands = their `invoke` RPC**, **Tauri events = their `eventOn`**, and the thin preload bridge disappears (Tauri's IPC is already typed).

### 4.2 Crate layout (workspace)

| Crate | Responsibility | Reference counterpart |
|---|---|---|
| `ade-core` | Domain modules: projects, tasks, conversations, workspaces, settings, library, automations, search, editor buffers, view state, resource monitor, secrets | `src/main/core/*` |
| `ade-git` | Git (git2): worktrees, status, staging, diff, commit, push; GitHub/GitLab API clients; PR sync | `src/main/core/git` |
| `ade-providers` | Provider registry (35 agents) + capability descriptors + detection/install descriptors | `packages/plugins/src/agents` |
| `ade-acp` | ACP client: protocol, session manager/cell, transcript reducer, per-provider adapters | `packages/core/src/acp` + `src/main/core/acp` |
| `ade-terminal` | PTY (portable-pty), terminal state persistence, tmux manager | `src/main/core/pty` |
| `ade-ssh` | SSH/SFTP (russh), connection profiles, config parse (`ssh -G`), proxy, port forwards | `src/main/core/ssh` + `port-forwards` |
| `ade-scheduler` | Cron scheduler, automation state machine, restart recovery | `src/main/core/automations` |
| `ade-integrations` | Issue trackers (12), GitHub accounts, CI checks | `src/main/core/integrations|issues|github` |
| `ade-telemetry` | Allowlisted event pipeline, feature flags | `src/main/core/telemetry` |
| `ade-server` | Remote daemon + wire protocol (JSON-RPC over SSH-forwarded socket) | `apps/workspace-server` + `packages/core/src/workspace-server` |
| `ade-runtime` | Out-of-process workers (ACP agent runtime, agent-config resolver, fs-watch) | `packages/runtime` |
| `ade-app` | Tauri shell: command modules, events, window, menu, updater | `src/main/index.ts|rpc.ts` + `updates` |
| `app-frontend/` | Webview UI (React or Svelte) | `src/renderer` + `packages/ui` + `chat-ui` |

### 4.3 Key crates (shortlist)

Backend: `tokio` · `git2` · `portable-pty` · `russh` + `russh-sftp` · `notify` · `rusqlite` · `keyring` · `croner` · `serde`/`serde_json` · `tracing` · `reqwest` · `tauri` 2 + plugins (`shell`, `process`, `updater`, `os`) · `sysinfo` (resource monitor) · `jsonrpc`/`jsonrpsee` or hand-rolled for wire protocol · `ignore` (gitignore filtering) · `glob` · `base64`/`sha2` (migration journal hashing).
Frontend: `xterm.js` · **CodeMirror 6** (chosen over Monaco — lighter, no native deps, has merge addon for diff editing, reference recommends it) · a diff component (CodeMirror merge) · React · Tailwind · Zustand (lighter than MobX, React-native).

### 4.4 Data & config layering

1. **SQLite** (`ade.db`, mirror their `emdash4.db`): full schema in §6. Drizzle-style numbered migrations + sha256 journal.
2. **`.ade.json`** (shareable): only `preservePatterns`, `shellSetup`, `scripts.{setup,run,teardown}`. Local settings override; "Share with team" moves values out of local config.
3. **OS keychain**: SSH passwords/passphrases, API tokens, GitHub tokens (their `app_secrets` table is plaintext-with-wrapper; we use keyring instead — safer and matches docs).
4. **Env-var contract** (injected into task terminals, agent sessions, shell setup, lifecycle scripts):

```
ADE_TASK_ID · ADE_TASK_NAME (slug) · ADE_TASK_PATH · ADE_ROOT_PATH
ADE_DEFAULT_BRANCH · ADE_PORT (base of 10-port range)
```

Additional env knobs from reference: `ADE_DB_FILE`, `ADE_DISABLE_NATIVE_DB`, `ADE_DISABLE_CLONE_CACHE`, `ADE_DISABLE_PTY`, `TELEMETRY_ENABLED`.

### 4.5 ACP vs TUI runtime paths (the big design decision)

The reference runs conversations through **two runtimes**, chosen per provider by its `acp` capability flag:

- **TUI/PTY path** (in main process): `local-pty`/`ssh2-pty` spawn the agent CLI in a PTY; initial prompt delivered via argv flag, stdin, or **keystroke injection** (agents with no prompt flag); session resume via deterministic `--session-id` flags (Claude) or resume flags.
- **ACP path** (out-of-process worker): an ACP client speaks the Agent Client Protocol (JSON-RPC over stdio) to provider adapter binaries (e.g. `@agentclientprotocol/claude-agent-acp`). `SessionManager` owns cross-session lifecycle; `SessionCell` owns one conversation (state machine, transcript reducer, permission broker, prompt queue, turn quiescence). Provider `sessionId`s are persisted by the host (returned from `startSession`/`resumeSession`, stored in `conversations.session_id`).

**Rust plan:** implement `ade-acp` as our own ACP client (the protocol is a public spec) OR use an existing Rust ACP crate (open question §10.1). The runtime worker runs as a child process (mirroring their out-of-process design; keeps protocol state out of the main process). The `SessionManager/SessionCell` split, reducer + live models, and per-provider enrich hooks are the architecture to port. Conversation → terminal attachment (agent-managed terminals via ACP) is a Phase-2 feature.

### 4.6 Domain module layout (mirrors their 45 dirs)

Rust modules under `ade-core` (+ `ade-git`, `ade-ssh`, etc.), one per domain, each exposing a **command module** (Tauri) + services:

`account, acp, agent_config, agent_hooks, agents, app, automations, browser, conversations, dependencies, editor, execution_context (local|ssh), files, fs_watch, git(repo|worktree), github, integrations, issues, mcp, port_forwards, preview_servers, project_setup, projects(worktrees|settings), prompt_library, provider_accounts, pty, pull_requests, repository, resource_monitor, runtime, search, secrets, settings, shared(oauth), skills, ssh(config|connect|credentials|lifecycle|transport), storage, tasks, telemetry, terminal_shell, terminals, updates, utils, view_state, workspaces`

Conventions (from their docs, ported): controllers are thin wrappers over service functions; stateful concerns use singletons with `initialize()` called at boot; multi-backend domains use trait + `impl/`; `Result<T,E>` everywhere; events named `eventName` or `eventName.{topic}`.

### 4.7 Workspace-server wire protocol (port as-is)

Remote projects/tasks talk to a daemon on the remote host over an **SSH-forwarded Unix socket**. Contract domains: `health, initialize, git, files, deps, tuiAgents, acp`.

- `PROTOCOL_VERSION = '1.0.0'` semver; major = breaking, minor = additive, patch = no wire impact.
- Every connect starts with `initialize` (re-called on reconnect). Response agrees `min(clientMinor, serverMinor)`.
- Major mismatch → `PROTOCOL_INCOMPATIBLE { action: 'upgrade-client'|'upgrade-server', ... }`.
- Same-major = compatible. Never repurpose fields; add-new-and-deprecate-old; unknown JSON keys are ignored (tolerant reader).

Our `ade-server` bin crate implements this contract in Rust; negotiation rules ported unchanged.

### 4.8 Security-sensitive areas (from their `risky-areas/`)

Treat as high-risk, with dedicated review: **PTY env passthrough** (allowlist in `pty-env.ts` equivalent), **shell quoting/escaping** (single shared helper, never ad-hoc), **ACP process spawning**, **SSH command construction + host-key handling**, **worktree path validation** (`realpath` containment — ops constrained to workspace root), **secrets** (keyring only, redaction in logs), **updater** (signed artifacts, channel invariants), **migrations** (never hand-edit; forward-only upgrades).

---

## 5. Data model (SQLite — port of their schema)

DB file `ade.db`; numbered migrations `0000..0019`-style; FTS tables version-gated via `kv` keys (`fts_version`, `file_index_version`).

| Table | Purpose | Key columns |
|---|---|---|
| `projects` | Project roots | path, workspace_provider local\|ssh, base_ref, ssh_connection_id |
| `project_remotes` | Git remote URL per project | (project_id, remote_name) PK |
| `project_settings` | Per-project settings | base/shareable JSON blobs |
| `app_settings` | KV app preferences | — |
| `tasks` | Task rows | project_id, name, status, linked_issue JSON, archived_at, type task\|automation-run, automation_run_id |
| `workspaces` | Worktree/BYOI workspaces | key, type, kind worktree\|project-root\|byoi, location local\|remote, config versioned JSON, branch_name |
| `conversations` | Chat sessions | project/task FKs, title, provider, config JSON, session_id, agent_status |
| `terminals` | Saved terminal tabs | project/task FKs, ssh flag, shell_id |
| `messages` | Legacy chat rows | conversation FK, sender, content, metadata |
| `editor_buffers` | Unsaved editor drafts | id `project:workspace:path`, content |
| `automations` | Automation defs | trigger/conversation/task config JSON, enabled, soft-deleted_at |
| `automation_runs` | Run instances | times, status, error, config snapshots, generated_task_name |
| `pull_requests` | PR sync data | URL PK, provider, base/head refs+oids, title/desc, status, review stats |
| `pull_request_users/labels/assignees/checks` | PR sub-resources | — |
| `ssh_connections` | Saved SSH hosts | host, port, username, auth_type, private_key_path, metadata |
| `provider_accounts` | OAuth accounts | credential_ref, is_default, meta JSON |
| `app_secrets` | Encrypted secrets | (we use keyring instead; table optional) |
| `kv` | Generic KV | account sessions, host-dep state, GitHub accounts/tokens, integration credentials, prompt library, PR sync cursor, view-state, telemetry, legacy port |
| FTS: `search_index`, `workspace_file_index`(+meta) | Command palette + workspace search | FTS5 |

**Not in SQLite:** MCP servers (per-agent config JSON files), skills (bundled catalog + disk installs), prompts (KV under `prompt-library`).

---

## 6. Feature inventory & epics

Legend: **S** <1d · **M** 2–5d · **L** 1–2w · **XL** 2w+. "Ref:" = reference repo location to study when implementing.

### E1 — App shell, projects & settings

**Features:** Add project (local / clone GitHub / remote SSH) · left sidebar tree with pinned tasks · Project Settings (GitHub account, worktree directory, default branch, base remote, push remote, tmux, workspace provider, preserve patterns, shell setup, lifecycle scripts setup/run/teardown) · `.ade.json` sharing + precedence + migration · env contract · script logs in terminal drawer (⌘J) · open project in editor (⌘O) · onboarding flow (sign-in/import steps) · command palette over projects/tasks/conversations + resource monitor view.

**Tickets:**

| ID | Ticket | Size | Acceptance (key points) |
|---|---|---|---|
| E1-01 | SQLite init + migration runner (numbered SQL + sha256 journal, FTS setup) | M | Fresh install + upgrade paths; `db:reset`-style helper |
| E1-02 | Settings store with layered precedence (local > `.ade.json`) + `kv` store | M | Precedence tests; Share-with-team moves values |
| E1-03 | Project model: add local / clone GitHub / connect remote | L | Ref: `core/projects`, `core/project-setup` (Pick/Clone/New) |
| E1-04 | Project tree sidebar + pinned tasks + create/delete (⌘⇧N) | M | Tree order drives task-switch nav |
| E1-05 | Project Settings UI + persistence (all fields) | L | GitHub account picker, tmux, workspace provider |
| E1-06 | Lifecycle script runner (setup/run/teardown) + env contract + drawer logs | L | `ADE_*` vars; 10-port range; logs in drawer |
| E1-07 | Preserve-pattern copying into new tasks | S | Never copies tracked files or `.ade.json` |
| E1-08 | Onboarding flow + view-state persistence (KV) | M | First-run; window layout restore |
| E1-09 | Command palette (⌘K) over projects/tasks/conversations, FTS-backed | M | Ref: `core/search` + `features/command-palette` |

### E2 — Task engine (core)

**Features:** Add Task (⌘N) from branch/issue/PR · auto/manual name (human-id style names; branch = prefix `ade` + random suffix, prefix configurable) · provider + model selector ("Default model") · worktree per task (defaults + override + disable-with-warning) · agent spawn (TUI via PTY or ACP) · terminal state autosave/resume · conversations (⌘T/⌘D/⌘⇧A/⌘Enter) · multiple terminals (⌘⇧T) · delete tasks (⌘Backspace, teardown agents/tmux/worktrees) · task switching (⌘⌥↑/↓) · task status lifecycle + telemetry.

**Tickets:**

| ID | Ticket | Size | Acceptance (key points) |
|---|---|---|---|
| E2-01 | Task model + lifecycle state machine (create→provision→running→done/archived/deleted) | M | Ref: `core/tasks` (task-service, task-builder, operations/) |
| E2-02 | Worktree manager (create/remove/reuse; local + overrides; preserve patterns) | L | Ref: `core/projects/worktrees/worktree-service.ts`; never hardcode paths |
| E2-03 | Task name + branch generation (prefix + random suffix, issue-derived) | S | Ref: `tasks/name-generation/`, `resolveTaskBranchName.ts` |
| E2-04 | Add Task flow: branch source, provider/model pickers | M | Model selector incl. "Default model" |
| E2-05 | Conversation session supervisor (local vs SSH impls) | L | Ref: `core/conversations` (`impl/local-conversation.ts`, `ssh-conversation.ts`); session resume via session-id/resume flags |
| E2-06 | Terminal spawn + agent launch (TUI path first) | L | PTY attach; prompt passing per provider (argv/stdin/keystroke injection) |
| E2-07 | Terminal state persistence + resume across restarts (tmux hook-point) | L | Rehydrate PTYs/processes |
| E2-08 | Conversations: create, splits, context menu, prompt/issue insertion | M | ⌘T/⌘D/⌘⇧A/⌘Enter |
| E2-09 | Task deletion/teardown (agents, tmux, worktrees) | M | Idempotent; safe with running agents |
| E2-10 | Task-switch navigation + project-tree integration | S | Skips collapsed/hidden tasks |
| E2-11 | **ACP conversation path** (worker, SessionManager/SessionCell, transcript reducer, live models) | XL | Ref: `packages/core/src/acp` + `core/acp`; see §4.5; Phase 2 |

### E3 — Agent providers

**Features:** 35 providers registered (codex, claude, grok, devin, qwen, qoder, droid, antigravity, cursor, copilot, amp, commandcode, opencode, hermes, charm, auggie, goose, kimi, kilocode, kiro, rovo, cline, codebuddy, continue, codebuff, freebuff, mistral, jules, junie, oh-my-pi, pi, letta, autohand, mimocode, zero) · **22 ACP-capable** · detection via PATH + install descriptors per OS · per-provider capability descriptors (metadata, capabilities, behavior, validate) · model enumeration · provider accounts (OAuth/login methods, API keys) · agent hooks (HTTP hook server, hook config writer, notifications) · PTY env allowlist · keystroke-injection prompt delivery for no-flag agents.

Capability flags (port): `acp, auth, autoApprove, effort, hooks, hostDependency, mcp, models, plugins, prompt, sessions, trust`.

**Tickets:**

| ID | Ticket | Size | Acceptance (key points) |
|---|---|---|---|
| E3-01 | Provider registry + capability descriptor types (Rust) | M | Ref: `packages/plugins/src/agents/registry.ts` + `impl/*`; all 35 entries |
| E3-02 | Host dependency detection/install/update/uninstall descriptors | L | Ref: `core/dependencies` (install-runner, dependency-managers) |
| E3-03 | Prompt-passing strategies (argv / stdin / keystroke injection) | L | Covers Amp (stdin) and no-flag agents |
| E3-04 | Auto-approve flag plumbing per provider | S | Only for capable providers |
| E3-05 | Agent hooks: hook server, config writer (`.claude/settings.local.json`, etc.), OS notifications | L | Ref: `core/agent-hooks`; never infer status from output |
| E3-06 | Model enumeration + selector incl. "Default model" | M | Task creation + new conversation |
| E3-07 | Provider accounts (login methods, API key registry) + OAuth flow helper | M | Ref: `core/provider-accounts`, `core/shared/oauth-flow.ts` |
| E3-08 | PTY env allowlist + spawn platform layer | M | Ref: `core/pty/pty-env.ts`; security-reviewed |

### E4 — Git & diff view (incl. line comments)

**Features:** Changes icon → right sidebar (Changed/Staged/Pull Requests) · auto-refresh on file+git events · file rows with status icons (added/modified/deleted/renamed/conflicted) · stage/unstage/discard (confirm on discard) · unified/split diff · inline-edit unstaged diffs (⌘S) · **Line comments: select code in diff → popover → Add Note or Create Task** (§14 of ARCHITECTURE.md) · bidirectional comment-task linking · inline task status badges on comments · agent-created comments via tool · commit card (Commit / Commit & Push / Commit & Create PR; Push & Create PR when needed) · PR section (files/commits/checks/comments/merge state) · footer git actions (fetch/pull/push/publish/add-remote) · PR sync engine + scheduler (ref: `core/pull-requests/pr-sync-engine.ts`).

**Tickets:**

| ID | Ticket | Size | Acceptance (key points) |
|---|---|---|---|
| E4-01 | File+git event watcher → live refresh pipeline | L | notify + git events; editor/agent/git all trigger |
| E4-02 | Git status/diff engine (worktree-scoped) | L | Status icons incl. conflict detection |
| E4-03 | Changes sidebar (Changed/Staged) + stage/unstage/discard | L | Confirm on discard; stage-all |
| E4-04 | Diff renderer: unified/split, preview vs persistent tab | L | Large diffs readable |
| E4-05 | Inline editing of unstaged diffs (⌘S) | M | Writes to disk; diff refreshes |
| E4-06 | Commit card: Commit / Commit & Push / Commit & Create PR | M | PR-open guard; push-before-PR |
| E4-07 | PR section: files/commits/checks/comments/merge state | L | GitHub API; empty states |
| E4-08 | Footer git actions (fetch/pull/push/publish/add-remote) | M | Correct disabled states |
| E4-09 | PR sync engine + scheduler + storage (pull_requests + sub-tables) | L | Ref: `core/pull-requests`; cursor in `kv` |
| E4-10 | Line comment system: selection popover, Add Note / Create Task, persistence | L | Ref: ARCHITECTURE.md §14; comment-task bidirectional linking; inline status badges |
| E4-11 | Agent comment tool + resolution flow | M | Agents call `add_line_comment`; manual resolve by user; badge updates on task completion |

### E5 — File editor

**Features:** folder icon → editor left / file tree right · tabs (preview vs persistent) · hidden dirs (node_modules/.git/build output) · git-changed highlight · delete file/folder (recursive + confirm; auto-close tabs) · CSV preview · syntax highlighting + find/replace · autosave/recovery (2 s debounce → `editor_buffers` table) · ⌘S / ⌘⇧S · unsaved dot · diff sync on save. Deferred: LSP, ⌘P, drag-drop.

**Tickets:**

| ID | Ticket | Size | Acceptance (key points) |
|---|---|---|---|
| E5-01 | File tree panel (hidden-dir filter, git-changed highlight) | M | Ref: `core/files` |
| E5-02 | Editor tabs + open/save/close semantics | L | ⌘S, ⌘⇧S, unsaved dot |
| E5-03 | Buffer recovery autosave (2 s debounce → `editor_buffers`) | M | Recovers after restart |
| E5-04 | Delete file/folder + tab auto-close (workspace events) | M | Recursive; confirmation |
| E5-05 | CSV preview + Preview/Source toggle | S | |
| E5-06 | Syntax highlighting + find/replace | M | |
| E5-07 | Editor ↔ diff view sync after save | S | |

### E6 — In-app browser & previews

**Features:** per-task browser tab (⌘⇧B) · **dev-server URL detection from terminal output** (ref: `core/preview-servers/terminal-url-detector.ts`) + preview server lifecycle · browser profiles/partitions · **SSH port forwards** for remote previews (ref: `core/port-forwards`) · webview security (CORS relaxation, user-agent).

**Tickets:**

| ID | Ticket | Size | Acceptance (key points) |
|---|---|---|---|
| E6-01 | Embedded webview browser tab + navigation chrome | L | Tauri webview; per-task instance |
| E6-02 | Dev-server URL detection from terminal output + preview lifecycle | M | Ref: `terminal-url-detector.ts`; ports from `ADE_PORT` |
| E6-03 | Browser profiles/partitions + webview security | M | Profile isolation |
| E6-04 | SSH port-forward tunnels for remote previews | L | Ref: `core/port-forwards` |

### E7 — Issue integrations

**Features:** Add Task → From Issue · 12 trackers (Linear, Asana, Trello, Monday.com, Jira, GitHub Issues, GitLab, Plane, Forgejo, Featurebase, Plain, Notion) with documented auth flows · issue context injection + editable pill (provider, identifier, title, URL, description, status, assignees, project) · Linear branch-name special case · issue provider registry + plugin pattern (ref: `core/issues`, `core/integrations`, `packages/plugins` issue plugins).

**Tickets:**

| ID | Ticket | Size | Acceptance (key points) |
|---|---|---|---|
| E7-01 | Tracker auth manager (tokens → keychain) + Integrations settings UI | L | All 12 connection flows |
| E7-02 | Issue provider registry + provider interface | M | Ref: `core/issues/registry.ts` |
| E7-03 | From-Issue task creation flow | M | Branch/name/provider; Linear special-case |
| E7-04 | Issue context injection + editable pill | M | Context shape per docs; append semantics |
| E7-05 | Trackers, grouped (Linear+Jira+GitHub first; then Asana/Trello/Monday/GitLab/Plane/Forgejo/Featurebase/Plain/Notion) | XL | One ticket per tracker or per group |

### E8 — GitHub accounts

**Features:** capabilities (issues/PRs/checks/comments/repo-create) · connection methods: ade-account OAuth, **gh CLI import**, device flow · account manager (default account, removal cascade) · per-project selection · GitHub API via own client (ref: their `github/` uses gh CLI + Octokit; we use `reqwest` + GraphQL/REST).

**Tickets:**

| ID | Ticket | Size | Acceptance (key points) |
|---|---|---|---|
| E8-01 | GitHub auth: device flow + gh CLI import + token storage | L | Keychain storage |
| E8-02 | Account manager UI + default-account logic | M | Removal cascades per docs |
| E8-03 | GitHub API client (issues/PRs/checks/comments/repo-create) | L | Shared by E4/E7/E9 |
| E8-04 | Per-project account selection + gating | S | "No GitHub account" disables features |

### E9 — CI/CD checks

**Features:** Files/Commits/Checks tabs · per-check status icon, name+source, duration, external link · PR comments below runs · refresh on tab open + head-commit change · failed sorts to top.

**Tickets:**

| ID | Ticket | Size | Acceptance (key points) |
|---|---|---|---|
| E9-01 | Checks data layer (list runs per head commit, refresh rules) | M | Refresh on open + new head |
| E9-02 | Checks UI (status icons, duration, link, comments) | M | Failed sorts to top |

### E10 — Library (prompts, skills, MCP)

**Features:** Prompts CRUD + search (KV-backed `prompt-library`) + insert into conversations · Skills: OpenAI + Anthropic catalogs, install → `~/.agentskills/` + symlinks to agent dirs, custom skills (SKILL.md frontmatter), uninstall/Open · MCP: Added/Recommended (54-server catalog), custom stdio/http, per-agent config sync with adapters (ref: `core/mcp/utils/` adapters), transport capability filtering, edit/remove.

**Tickets:**

| ID | Ticket | Size | Acceptance (key points) |
|---|---|---|---|
| E10-01 | Prompts CRUD + search + insertion (KV-backed) | M | Built-in Review prompt; append semantics |
| E10-02 | Skill manager: catalogs, install/uninstall, custom creation | L | Ref: `core/skills` (bundled-catalog.json + disk) |
| E10-03 | Skill symlink sync to agent dirs | M | Symlink lifecycle; works standalone |
| E10-04 | MCP config model + per-agent sync engine with adapters | L | Ref: `core/mcp` (services/McpService, utils/adapters) |
| E10-05 | MCP UI: Added/Recommended, catalog add, custom add, edit/remove | L | Transport capability filtering |
| E10-06 | Library shell (⌘L, section routing) | S | |

### E11 — Automations

**Features:** per-project cron automation (schedule, prompt+provider, workspace setting, enable/pause, Run now) · runs create normal tasks (convert-to-task) · run history (status/timing/trigger/errors) · lifecycle states (`scheduled→queued→creating_task→launching_task→creating_conversation→done|failed|skipped`) · self-healing scheduler (one upcoming run per automation; restart recovery) · automation templates.

**Tickets:**

| ID | Ticket | Size | Acceptance (key points) |
|---|---|---|---|
| E11-01 | Cron scheduler core (croner) + single-upcoming-run invariant | M | Restart recovery test |
| E11-02 | Automation CRUD + enable/pause + Run now + templates | M | Workspace-setting options |
| E11-03 | Run execution state machine + task/conversation creation | L | Ref: `core/automations` (run-transitions, runtime) |
| E11-04 | Run history persistence + UI | M | Survives restart |

### E12 — Remote development (SSH, remote projects, remote tasks, workspace server)

**Features:** SSH connection profiles (manual + `~/.ssh/config` alias via `ssh -G`, auth: password/key/agent, ProxyJump/ProxyCommand/ForwardAgent, supported-directive list, ambiguous-agent-socket guard) · Remote Projects (Pick/Clone/New, SFTP browse, remote worktrees `<project>/.ade/worktrees`, remote PTYs via SSH, remote agent detection/install, connection states + backoff 1/2/5/10/20 s, MaxSessions panel, rehydrate on reconnect) · Remote Tasks (provision/terminate scripts, JSON contract, 10-min timeouts, `REMOTE_WORKSPACE_ID`, forwardAgent) · **workspace-server daemon + wire protocol** (§4.7) · tmux durability · SSH port forwards.

**Tickets:**

| ID | Ticket | Size | Acceptance (key points) |
|---|---|---|---|
| E12-01 | SSH client layer (russh): connect, auth matrix, PTY channels | L | Password/key/agent auth |
| E12-02 | SFTP layer: browse, read/write, search, realpath | M | Path-constrained to workspace |
| E12-03 | Connection profile CRUD + `ssh -G` resolution + ProxyJump/ProxyCommand/ForwardAgent | L | Locked fields; token handling; ambiguous-socket guard |
| E12-04 | Remote project: Pick/Clone/New + worktree-on-remote | L | Remote defaults + overrides |
| E12-05 | Remote terminals + agent launch over SSH PTY + remote detection | L | Env vars; install-agent path |
| E12-06 | Connection lifecycle: states, backoff, rehydrate, MaxSessions panel | M | Backoff 1/2/5/10/20; no auto-reconnect on manual disconnect |
| E12-07 | Remote tasks: provision/terminate runner + JSON contract + timeouts | L | 10-min timeout; idempotent terminate |
| E12-08 | **Workspace-server daemon + wire protocol (Rust)** | XL | Ref: `apps/workspace-server` + `packages/core/src/workspace-server`; handshake + negotiation ported verbatim; SSH-forwarded Unix socket |
| E12-09 | SSH port-forward tunnels (shared with E6-04) | L | |
| E12-10 | Remote Tasks feature gate + task-side wiring | M | Build flag; workspace provider settings |

### E13 — Tmux sessions

**Features:** app-wide default + per-project toggle · deterministic `ade-<encoded PTY ID>` sessions · create-if-missing, mouse + large history · reattach on restart/reconnect · scrollback in tmux, output streamed · cleanup on delete · Windows local never tmux.

**Tickets:**

| ID | Ticket | Size | Acceptance (key points) |
|---|---|---|---|
| E13-01 | Tmux manager: session id derivation, create/attach/kill | M | Deterministic naming; mouse/history opts |
| E13-02 | Persistence + reattach across restart / SSH reconnect | L | Toggle-resolution rules per docs |
| E13-03 | Settings wiring + cleanup on delete | S | Windows local disabled |

### E14 — Keyboard shortcuts & UI shell

**Features:** full default map (⌘K, ⌘,, ⌘L, ⌘N, ⌘Backspace, ⌘⇧N, ⌘O, ⌘Enter, ⌘[/], ⌘B/⌘., ⌘⇧1/2/3, ⌘T/⌘D, ⌘⇧T, ⌘⇧B, ⌘J, ⌘⌥↑/↓, tab nav ⌘1–9/⌘W/⌘\, Ctrl+Tab wrap, ⌘F, ⌘⇧A, ⌘Enter) · customizable via Settings · scoping rules · editor-focus exemption · pane/tab layout system (ref: `features/tabs` — pane-store, tab-bar, tab-view-factory) · command registry (ref: `renderer/lib/commands/registry.ts`).

**Tickets:**

| ID | Ticket | Size | Acceptance (key points) |
|---|---|---|---|
| E14-01 | Keybinding registry + default map + scoping engine | L | Scope precedence; editor exemption |
| E14-02 | Settings UI for shortcuts + hint rendering from live bindings | M | Clear-binds-remove-hint |
| E14-03 | Command palette (⌘K) — merged with E1-09 | M | |
| E14-04 | Pane/tab layout system (splits, tab bar, tab registry) | L | Ref: `features/tabs`; tab scoping per task |

### E15 — Telemetry & feature flags

**Features:** opt-out (Settings toggle + `TELEMETRY_ENABLED=false|0|no`) · dev builds silent · `instanceId` in `kv` · identity only when signed in · never collect file contents/prompts/env/text/repo names/IP/recordings · allowlist sanitization (trim, length limits, bounded numbers) · event taxonomy (lifecycle, focus/views, projects, tasks, conversations/agents w/ provider+exit_code, terminals, PRs, VCS, account, integrations/issues, external/SSH, MCP, skills, settings/UI, errors) · PostHog-style ingestion + `decide` feature flags.

**Tickets:**

| ID | Ticket | Size | Acceptance (key points) |
|---|---|---|---|
| E15-01 | Telemetry pipeline: allowlist, sanitization, batching, opt-out | M | Env var + toggle; dev builds silent |
| E15-02 | Core event coverage (lifecycle, task, agent, VCS) | M | Parity with taxonomy |
| E15-03 | Feature-flag client + `kv`-backed settings | S | Flag-driven UI toggles |

### E16 — Distribution & packaging

**Features:** macOS arm64/x64 dmg + Homebrew cask · Windows x64 exe + msi · Linux x86_64 AppImage + amd64 deb · release channel (R2/GitHub Releases; their release flows: `release-prod.yml` / `release-canary.yml`) · updater (two feeds, channel-manifest naming, signed) · launch-time agent auto-detection · works fully offline without sign-in.

**Tickets:**

| ID | Ticket | Size | Acceptance (key points) |
|---|---|---|---|
| E16-01 | Tauri bundling matrix (dmg/msi/exe/AppImage/deb) + CI | L | All 3 platforms |
| E16-02 | Signed updater (two feeds, channel manifest) | M | Ref: `core/updates` + `risky-areas/updater.md` |
| E16-03 | Homebrew cask + release notes flow | S | Formula updates |

---

## 7. Testing strategy (mirroring theirs)

Their merge gate: `format → lint → typecheck → test`; Vitest projects: `node`, `main-db` (real SQLite integration), `fixtures`, `migrations`, `browser` (Playwright renderer), `scripts`. Integration tests create **temporary repos + worktrees in `os.tmpdir()`**. CI runs `nx affected` on touched projects.

Rust equivalent:
- `cargo test` per crate; `ade-core::db` integration tests against real SQLite temp files (their `main-db`).
- Migration tests: apply `0000..N`, assert schema; versioned-JSON upgrade-chain tests incl. `future-version` degradation.
- Fixture generator for dev DBs.
- Git integration tests: build temp repos with `git2`, create worktrees, exercise stage/commit/push against a local bare remote.
- ACP runtime tests: protocol state machine, transcript reducer, per-provider enrich hooks (unit); process-host integration against a fake ACP adapter binary.
- SSH tests: their `run:docker-ssh` uses Docker; we can use a local `sshd` fixture or docker when available; unit-test `ssh -G` parsing with fixture configs.
- UI: Tauri WebDriver (`tauri-driver`) for renderer smoke tests; Playwright for the web frontend.
- CI: GitHub Actions mirroring `code-consistency-check.yml` (format/lint/clippy + `cargo test` on affected crates).

---

## 8. Phased delivery plan

**Phase 0 — Foundations (single local project, TUI path)**
E1 (projects, settings, DB), E2-01..04, E3-01..04/08, E14-01, terminal+worktree MVP.
→ *Milestone: create a task in a local repo, agent runs in a PTY, state survives restart.*

**Phase 1 — Local review & ship**
E2-05..10, E3-05..07, E4, E5, **Project Chat (see ARCHITECTURE.md §13 — project-level agent w/ delegation)**, E14-04, E16-01 (dogfood builds), testing harness.
→ *Milestone: full local loop — task → agent → diff → commit → push → PR + CI checks; project-level agent delegates to sub-tasks.*

**Phase 2 — Scale & integrations**
E6, E7, E8, E9, E10, E11, E15, **E2-11 (ACP chat path)**.
→ *Milestone: parallel agents at scale, automations, library, issue-driven tasks, structured ACP chat.*

**Phase 3 — Remote**
E12 (incl. E12-08 workspace-server), E13.
→ *Milestone: SSH projects, provisioned remote tasks, durable tmux sessions.*

**Phase 4 — Backlog (explicitly deferred)**
Editor LSP features, ⌘P quick-open, drag-and-drop, Cloud/Enterprise offerings, remaining tracker breadth, ACP agent-managed terminals.

---

## 9. Cross-cutting concerns

1. **Persistence contract**: terminal state, editor buffers (2 s debounce), tmux sessions, scheduler, run history — all restart-safe; kill-restart tests required.
2. **Secrets hygiene**: OS keychain only; never SQLite/`.ade.json`/logs; redaction in file logging.
3. **Isolation**: worktree per task; 10-port env range; path-scoped remote ops; no cross-task mutation.
4. **Event pipeline**: single internal event bus (git, fs, agent hooks, ACP updates, ssh) → typed Tauri events; used by diff auto-refresh, editor tab auto-close, PR/CI refresh.
5. **Error surfacing**: `Result<T,E>` everywhere; SSH degraded-health panel, provision/terminate failure logs, agent exit codes visible in UI + telemetry.
6. **Windows parity caveats**: no local tmux; x64-only installers; path handling; RunAsNode-style fuse concerns don't apply (Tauri), but updater signing does.
7. **Performance**: file watcher in a worker (ref: `fs-watch` runtime process); search FTS; virtualized lists (sidebar, transcripts).

---

## 10. Open questions (decide before/while building)

1. **ACP in Rust** — implement the ACP client ourselves (public spec, JSON-RPC over stdio) vs a community crate? Their stack: `@agentclientprotocol/sdk`. Recommendation: own minimal client; TUI path ships first (Phase 0), ACP in Phase 2 (E2-11).
2. **Git library** — `git2` (libgit2 bindings, v0.21). **Confirmed**: git2 has `worktree()`, `worktrees()`, `find_worktree()`, `Worktree::prune()` — full worktree lifecycle. gix 0.86 only handles worktree state (index, attributes), not add/list/prune. Use git2 for worktree operations; shell out to `git` CLI for operations git2 doesn't cover. Note: git2 is `!Sync` — all git ops must be serialized.
3. **Frontend framework** — React (their choice; larger ecosystem, Monaco) vs Svelte (lighter). Their chat-ui is Solid-based; we won't reuse it.
4. **Editor component** — CodeMirror 6 (recommended, lighter) vs Monaco (what they use; heavier but closest parity for inline diff editing).
5. **Workspace-server scope in v1** — the wire protocol daemon (E12-08) is the heaviest single ticket. Option: Phase 3 uses direct SSH commands first (like their legacy path), workspace-server daemon follows. Confirm.
6. **Issue-tracker priority** for E7-05 — Linear/Jira/GitHub first, then the rest.
7. **Sign-in/account** — reference has optional ade account + `provider_accounts`. Recommend skipping our own account system; device flow + gh import cover GitHub.
8. **Telemetry provider** — PostHog parity vs self-hosted vs disabled-by-default (privacy allowlist required either way).
9. **Domain parity** — 45 main-process domains is a big surface. Confirm which to cut in MVP (proposal: cut `repository` (open-in-provider), `account`, `storage` operations, `runtime` legacy manager; keep the rest as stubs).
10. **Name/branding** — resolved: `ade` (crate prefix `ade-*`).

---

## Appendix A — Reference module map (their 45 domains → our plan)

`src/main/core/`: account(E15/E8) · acp(E2-11) · agent-config(E3) · agent-hooks(E3-05) · agents(E3) · app(E1) · automations(E11) · browser(E6) · conversations(E2) · dependencies(E3-02) · editor(E5) · execution-context(E2/E12) · files(E5) · fs-watch(E4-01) · git(E4) · github(E8) · integrations(E7) · issues(E7) · mcp(E10) · port-forwards(E6/E12) · preview-servers(E6-02) · project-setup(E1-03) · projects(E1/E2) · prompt-library(E10) · provider-accounts(E3-07) · pty(E2/E3-08) · pull-requests(E4-09) · repository(deferred) · resource-monitor(E1-09) · runtime(deferred) · search(E1-09) · secrets(E15/E8) · settings(E1) · shared/oauth(E8) · skills(E10) · ssh(E12) · storage(deferred) · tasks(E2) · telemetry(E15) · terminal-shell(E2) · terminals(E2) · updates(E16) · view-state(E1-08) · workspaces(E1/E12)

## Appendix B — Reference (upstream Emdash) facts for product decisions

- Open source (Apache-2.0), YC W26, 5.3k stars, 1M+ downloads, app v1.1.40.
- Works fully locally without sign-in; GitHub/issue-tracker connections optional.
- Free desktop app; Cloud/Enterprise are separate hosted offerings (out of scope).
- 35 providers registered in repo (22 ACP-capable); docs claim 34 — repo wins.
- ACP chat is shipped (docs roadmap page is stale).
- Roadmap/deferred: LSP editor features, ⌘P, drag-drop, terminal-backend choice, Windows installer fix.

## Appendix C — Docs page → epic map

| Docs page | Epic(s) |
|---|---|
| tasks, keyboard-shortcuts, project-config | E1, E2, E14 |
| diff-view, ci-checks | E4, E9 |
| file-editor | E5 |
| in-app-browser (landing) | E6 |
| issues, github-accounts | E7, E8 |
| library + prompts/skills/mcp | E10 |
| automations | E11 |
| remote-development + remote-projects/remote-tasks/ssh-connections, tmux-sessions | E12, E13 |
| providers | E3 |
| telemetry, installation | E15, E16 |
| roadmap, landing | E6, E10, E12, backlog |
