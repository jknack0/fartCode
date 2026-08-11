# MEMORY.md — fartCode
## #96 File tree panel LANDED (2026-08-11, c6a89cf)

E5-01 — the editor epic opens (last unbuilt Phase-1 epic). Backend:
`fartcode_core::files::list_dir` beside `write_file`, same two-way
containment (lexical no-abs/no-`..` + canonical starts_with); symlink
entries list as FILES and are never followed (link-cycle + escape guard);
hidden dirs filtered (`.git`, `node_modules`, `dist`, `target`, `build`,
`out`, `.next`, `.venv`, `__pycache__`); dirs-first case-insensitive sort.
Command `list_workspace_dir` is async + `off_main_thread` (#80). Frontend:
new `files` tab kind — id `files:<workspaceId>` (restart-safe, no sidecar),
`FileTreeView` keeps tree state in the component (tabs stay mounted, so it
survives tab flips free), lazy expand, refetches LOADED dirs on
`files:changed`/`git:changed` (never polls), changed tint = snapshot paths
+ ancestor dirs from the E4-03 changes store. `lib/open-file.ts` is the
E5-01→E5-02 seam: tree emits OpenFileIntent, editor tabs will subscribe.
Header `files` button → `openFileTree` (workspaceId resolved like
ChangesSidebar: task.workspaceId ?? project.repositoryWorkspaceId).

Bites:
- Remote (SFTP) workspaces: `workspace_path` errors "no local path" — the
  tree shows it. SFTP listing is a follow-up (E5 × E12-02).
- Styles: theme vars are `--meta`/`--foreground`/`--hover-bg`/`--fc-bad-text`
  (NOT --text-secondary etc.) — check changes.css before guessing.

Next: E5-02 editor tabs (CodeMirror 6 per ARCHITECTURE §Editor,
`editor_buffers` table already exists) — consumes the open-file seam.
## #95 SSH port forwards LANDED (2026-08-11, 8963b08)

E12-09 (shared E6-04). `fartcode-ssh/src/forward.rs`: `open_tunnel` binds a
127.0.0.1 listener (preferred port → ephemeral on AddrInUse), forwards each
accepted socket through a fresh direct-tcpip channel, `copy_bidirectional`.
Remote loopback family fallback 127.0.0.1 → ::1 ONLY on ConnectFailed (Node
≥17 dev servers often bind [::1] only — ref emdash port-forward-tunnel.ts).
`TunnelDialer` trait returns a boxed stream (russh `Channel` is not fakeable
— trait made the tunnel testable with a TCP echo fake). App layer:
`PortForwardService` (id-keyed, idempotent open, race keeps FIRST tunnel),
`RegistryDialer` dials via `client_for` each time (rehydrated session picked
up free) + report_channel_error/ok. Commands port_forward_open/stop/list;
`ssh_disconnect` tears down that connection\u2019s tunnels. Preview UI → E6-04.

Bites:
- **#80 rule is mechanical**: even a mutex-only command must be `async` —
  the no_blocking_tauri_commands test flags registration shape, not cost.
- russh renders channel-open refusal as `Failed to open channel
  (ConnectFailed)` inside our `Error::SshChannel` string — detection is a
  lowercase substring match on "connectfailed".
- Drive-by: byoi.rs terminate let-else → `?` (clippy -D warnings on current
  toolchain failed on main; `?` on the Option keeps identical semantics).

Next per the agreed order: E12 is COMPLETE (01–10). Frontier: E6 previews
(E6-04 consumes this tunnel layer) or E14-02/03/04 UI shell.

## #100 Terminate warnings LANDED (2026-08-11)

ADR-0044's deferred call, called. `byoi::terminate` still never fails, but
now returns `Option<String>` — the warning for the USER when the script
exits nonzero or cannot run. `byoi_tasks::terminate` forwards it (plus its
own "no terminate context" case) as new `InternalEvent::ByoiTerminateWarning`
→ bridged as `task:terminate_warning` → `<Warnings/>`, a fixed bottom-right
dismissible strip mounted in App root. Dismiss is the only action: the task
is already deleted, the machine is the user's infrastructure to check.

Bites:
- **"Never fails" and "reports failure" are compatible** — the signature
  moved from `()` to `Option<String>` without violating the ADR: teardown
  still continues unconditionally, the Option is advice, not control flow.
- The warning must outlive its task: by teardown time the task row is going,
  so nothing task-scoped can host the message — hence an app-level strip,
  not a task-view banner.
- `EventBus` is a trait — `use fartcode_core::events::EventBus` is required
  at the send site even though the field type is `Arc<BroadcastEventBus>`.
- Component subscribes itself (SshConnections pattern) — no store slice; a
  dismissed warning is GONE, which is correct for advice with no action.

Next per the agreed order: E12-09 SSH port-forward tunnels (shared E6-04).

## #99 Host key verification LANDED (2026-08-11)

`fartcode-ssh/src/known_hosts.rs` closes the E12-01 ponytail
(`check_server_key` accepted every key — MITM-able since the SSH era began).
Policy is OpenSSH `accept-new`: known key connects; unknown host is recorded
in `~/.ssh/known_hosts` and connects (TOFU); a CHANGED or `@revoked` key
REFUSES. Matching covers the file's whole vocabulary: comma-separated globs
(`*`/`?`), `!` negations, `[host]:port` brackets, `|1|` HMAC-SHA1 hashed
hostnames. `SshHandler` is no longer a unit struct — it carries the dialed
`host`/`port`, including through `connect_over` (jump hops verify the
TARGET's key end-to-end, which is the point of ProxyJump).

Bites:
- **`russh::Error` is a closed enum** — refusal is `Ok(false)` from
  `check_server_key`, so the user-visible error is russh's generic one and
  the actionable detail (file, fingerprint, "remove the stale entry") lives
  on the `tracing::error!`. Surfacing that in the UI is future work.
- ssh-key here is the RUSSH FORK (`internal-russh-forked-ssh-key`); it does
  NOT re-export `base64ct` — dev-dep it directly for the hashed-entry test.
- `@cert-authority` entries are SKIPPED, failing toward "unknown" (record)
  rather than trusting an unverified CA. `@revoked` for a different key is
  neither a match nor a mismatch.
- Port 22 must match both `host` and `[host]:22` spellings; other ports only
  the bracket form.
- Tests generate ed25519 keys with a hand-rolled LCG `CryptoRngCore` — no
  rand dev-dep, deterministic keys per seed.
- No `$HOME` → verify is impossible; warn loudly and proceed (the old
  behavior, said out loud) rather than bricking containerized runs.

## #98 New-repo flow LANDED (2026-08-11)

E12-04's third leg. Core: `RemoteProjectStore::create_remote_new` —
slugify via `safe_path_segment(name, "repo")`, refuse an occupied target,
`git init --initial-branch main`, then delegate to `create_remote` (whose
`rev-parse --show-toplevel` check passes on an empty repo, and whose base-ref
resolution already falls back to the bare branch when there is no remote).
Command: `new_remote_project(connection_id, name)` — destination is
`remote_projects_dir` per profile, like clone. UI: the dialog's `clone`
boolean became `kind: pick | clone | new`; `new repo` pill is REMOTE-ONLY
and switching to the local tab mid-new falls back to pick (no dead submit).

Bites:
- **Anchor-append editing bit me twice today**: inserting a new test "before"
  an anchor by `old → old + new` where `new` re-quotes the anchor duplicated
  the fn header → unclosed-delimiter at a line far from the mistake. Insert
  AFTER a block, or replace the full block.
- FakeHost needed a `git init --initial-branch` match arm — its `_ => ok`
  default makes an unmodeled git command an invisible no-op, so the follow-up
  `create_remote` fails "not a git repository", which is at least loud.
- `safe_path_segment` keeps spaces ("my app" stays "my app") — it strips
  path-hostile chars, it does not kebab-case.
- Local "New" is deliberately absent: PRD E1-03 is add/clone/connect; a
  local init flow is a different ticket if it is ever wanted.
- Something in the shell pipeline summarizes cargo output ("PASS: 0 passed")
  — write to a file and `pi.read` the log for the real result.

## #97 Clone flows UI LANDED (2026-08-11)

E12-04 shipped Pick/Clone/New commands; #95 wired Pick. Now Clone:
`CreateProjectDialog` grew a `clone url` pill (right end of the source
toggle, orthogonal to the local/remote tabs). Clone on:
- local tab → `clone_project` (FLOWS.md F2 — e2e FIRST-16's
  "unreachable-entirely" finally closed),
- remote tab → `clone_remote_project(connectionId, url)` — host select stays,
  directory browser hides (the backend picks the projects dir per profile).
New bindings `cloneProject`/`cloneRemoteProject` + matching sidebar store
actions (same append+select bookkeeping).

Bites:
- **Host select had to be HOISTED out of the remote-tab fragment** —
  clone+remote needs the connection but not the browser, so the JSX is now
  `{remote && select} {clone ? url : local ? path : browser}` rather than a
  two-arm ternary owning everything.
- Clone destination is the BACKEND's choice on both paths (local projects
  dir setting / `remote_projects_dir` per profile) — the dialog deliberately
  offers no destination field.
- `cloneProject` collides with the dialog-local naming — store selectors are
  bound as `cloneProjectAction`/`cloneRemoteProjectAction` in Modals.tsx.
- Footer verb tracks the pair: add project / add remote project / clone
  project / clone on host — the label is the submit contract.

## #96 BYOI settings gate LANDED (2026-08-11)

The workspace-provider settings turned out to already EXIST — the
"Provision · terminate commands" row in `ProjectSettings.tsx` (pair editor,
saves on blur, `type: "script"` enforced server-side). What was missing was
the E12-10 gate: `remote_tasks_enabled` had no caller, so a build compiled
without `remote-tasks` still offered a form for scripts it can never run.
The row now renders only when `remoteTasksEnabled()` resolves true; the
probe fails closed (catch → hidden).

Bites:
- **Read the pane before writing the ticket** — #94's "missing UI" note was
  half stale; the form shipped with the pane, only the gate was absent.
- Gate is per-BUILD, not per-project: one `useEffect` probe on mount, no
  event to subscribe to (a cargo feature cannot change at runtime).
- Teardown stays ungated on the backend (ADR-0045): hiding the FORM is safe
  because terminate ignores the gate — a disabled build still cleans up.
- Test factory mocks default via `vi.fn(() => Promise.resolve(false))`;
  `clearAllMocks` keeps factory impls, so the hidden case needs no beforeEach
  line and the shown case overrides per-test.

## #95 Remote project picker LANDED (2026-08-11)

`remote_browse` finally has a caller: the Add-project dialog
(`Modals.tsx::CreateProjectDialog`) grew a local/remote source toggle. Remote
tab = connection select (profiles from `ssh_connection_list`) + a directory
walker over the POOLED session; "add remote project" hands the CURRENT
directory to `create_remote_project` via a new `useSidebar.createRemoteProject`
action (same append+select bookkeeping as local). New tauri.ts bindings:
`RemoteEntryDto`, `remoteBrowse`, `createRemoteProject`.

Bites:
- **`remote_browse` never echoes the cwd back** — no-path means the host's
  login dir, but the response is entries only. The dialog recovers cwd from an
  entry's parent path; an EMPTY login dir leaves cwd unknown (and add
  disabled) until the user types a path.
- Every click is a fresh listing, not client-side tree state — one round trip
  per step, nothing to invalidate on reconnect.
- Files are filtered out client-side; the picker deals only in directories.
- Repo validation stays server-side in `create_remote_project` — the picker
  does not probe for `.git`, the error row reports the backend's verdict.
- `react-hooks/exhaustive-deps` is NOT installed in this eslint config — a
  disable comment for it is itself a lint error.
- Styling: `fc-src-toggle` + `fc-remote-list` in modals.css; everything else
  reuses the composer grammar (fc-input-row / fc-opt-row).

Still missing UI: workspace-provider (BYOI) settings form.

## #94 SSH connections UI + command layer LANDED (2026-08-11)

E12-03's store finally has a door: `fartcode-app/src/commands/ssh_connections.rs`
(`ssh_connection_list` / `_save` / `_delete`, no secrets in the DTO) and
`app-frontend/src/components/SshConnections.tsx`, mounted as a **Connections**
section in `SettingsModal`. Live state from `ssh:state_changed`
(`reconnecting · 5s (3/5)` — the backend's ladder numbers, not a UI timer),
MaxSessions advice from `ssh:health_changed`.

Bites:
- **Saving an edited profile calls `remote_pty.forget(id)`** — the pooled
  session describes the OLD host/auth. `forget`, not `disconnect`: no
  manual-disconnect intent, so the next open dials with the new fields.
- Delete refuses while `reference_count > 0`; a dangling `ssh_connection_id`
  is a project that can never open.
- `has_secret` in `ssh_connections::secrets` is a boolean probe — the form
  shows “stored — blank keeps it” instead of an empty box that looks unsaved.
- Panel styling reuses the provider-account vocabulary (`fc-acct-*`); only the
  state pill and the MaxSessions note are new CSS. Colour is rationed per
  DESIGN.md: green = connected, `--fc-bad-text` = unreachable, everything
  transient stays `--meta`.
- MaxSessions is deliberately NOT an error line: the session is alive, so
  “reconnect” would be wrong advice — the note tells the user to raise
  `MaxSessions` on the host.

Still missing UI: workspace-provider (BYOI) settings form, remote project
picker (`remote_browse` has no caller).

## #93 E13 closeout — tmux slot release + Windows guard LANDED (2026-08-11)

Two E13 acceptance items that survived into the SSH era. `release_slot` in
`fartcode-app/src/terminals.rs` is now the single place a tmux slot is freed
(failed spawn / close / **unexpected exit**), and the output pump calls it —
so an SSH drop no longer strands a live remote session while the next open
mints `:1` beside it. `local_tmux_supported()` in `fartcode-core/src/pty/tmux.rs`
short-circuits binary resolution on Windows.

Bites:
- **A dead PTY is not a dead tmux session.** The client dies with the SSH
  connection; the session keeps running on the host. Ownership tracking has to
  follow the CLIENT, or reattach after an E12-06 reconnect silently duplicates
  sessions.
- `task_slots` is now `Arc<Mutex<..>>` — the pump thread needs it.
- Windows: tmux on PATH there is msys/WSL, a different filesystem namespace
  than the worktree. Refuse locally; remote hosts probe their own tmux.
- Toggle resolution is reference parity and now documented at the read site:
  app-wide `tmuxByDefault` is stamped into the project row at creation;
  reads are `tmux ?? false`. Changing the app default does not retro-apply.

## #92 E12-10 BYOI task wiring + feature gate LANDED (2026-08-11) — ADR-0045

`fartcode-app/src/byoi_tasks.rs` connects E12-07's contract to real tasks:
`provision` runs before core's provision (which is a no-op for byoi rows),
`terminate` runs first in `delete_task_blocking`. `RemotePtyRegistry` gained
transient connections (`register_transient`/`forget_transient`) so a
provisioned machine gets the whole E12-06 lifecycle without a saved profile.
Core gained `byoi_workspace_for_task` + `record_provisioned_machine`.
Command: `remote_tasks_enabled`.

Bites:
- **Gate is `remote-tasks`, a cargo feature, default OFF**, enforced at
  PROVISION only. Teardown deliberately ignores it — a disabled build must
  still clean up what an enabled one created.
- **`ssh_connection_id` (not `remoteWorkspaceId`) means "provisioned"**: a
  script may return a blank id, and re-provision would leak the first VM.
- Connection id is `task:<task id>` — namespaced so it cannot collide with a
  saved profile UUID; the pool checks transients BEFORE the profile store.
- Scripts run where the PROJECT lives, never on the provisioned machine.
- A new non-async tauri command needs a `SYNC_OK` entry in
  `tests/no_blocking_tauri_commands.rs` or the guard fails (bit me on
  `remote_tasks_enabled`).
- `clippy::assertions_on_constants` rejects `assert!(!ENABLED)` — bind to a
  `let` first if you want a real test of a `cfg!` const.
- Deleting a task waits on terminate (10-min budget) by design.

## #91 E12-07 BYOI provision/terminate LANDED (2026-08-11) — ADR-0044

`fartcode-core/src/tasks/byoi.rs`: the BYOI contract — `ProvisionOutput`
(JSON descriptor: `id`, `host` required; `port`/`username`/`worktreePath`/
`password`/`forwardAgent` optional), `parse_provision_output`, `provision`,
`terminate`, `script_command_line`, `LocalScriptRunner`. `ScriptRunner` is the
one-method trait; `fartcode-ssh/src/byoi.rs` adds `SshScriptRunner` +
`connect_params` (user@host split, port 22, username fallback chain,
forward-agent guard). 23 tests, no live SSH.

Bites:
- **Terminate returns `()` on purpose.** It runs inside teardown; an
  already-gone machine, a nonzero exit, and an unreachable host all warn and
  continue. Do not "fix" it into a `Result` — a failing teardown strands the
  task half-deleted.
- **`ProvisionOutput` hand-writes `Debug`** to redact `password` (same rule as
  `AuthMethod`). Adding a field means updating that impl.
- Descriptor parsing is lenient about unknown fields, strict about `host`;
  error text quotes the script's own output capped at 200 chars.
- `SshScriptRunner` builds `KEY='v' cmd` through `script_command_line` (values
  quoted, keys validated) — a remote shell has no `Command::envs`.
- `connect_params` REFUSES `forwardAgent` without `SSH_AUTH_SOCK`, and refuses
  a descriptor with neither password nor agent, instead of dialing and failing
  on auth.
- 10-minute budget for both scripts; local child dies via `kill_on_drop` when
  the timeout future is dropped.
- **Nothing calls any of it yet** — provision-flow wiring, the feature gate
  and pool registration are E12-10.

## #90 E12-06 connection lifecycle LANDED (2026-08-11) — ADR-0043

`RemotePtyRegistry` (fartcode-app/src/remote_pty.rs) is now the only owner of
remote sessions: one pooled `SshClient` per connection, shared by the PTY
manager, `RemoteTmux`, and the remote-project commands. States
(`connecting|connected|reconnecting|disconnected|error`), a 1/2/5/10/20 s
reconnect ladder, manual-disconnect intent, and a MaxSessions health signal.
Events: `ssh:state_changed` (carries `attempt`/`delayMs` while reconnecting)
and `ssh:health_changed`. Commands: `ssh_connection_state` (now a status
object, was a bool) + new `ssh_connection_states`.

Bites:
- **russh `Handle` has `is_closed()` but no close future** — liveness is a 2 s
  poll per session, not a subscription. It is NOT a keepalive.
- **A cached entry can be a corpse.** Every read goes through `cached()`,
  which drops a closed client rather than handing it out; before this, a
  terminal open against a dead host failed at channel time.
- **Watchdogs need a `Weak<Self>`** — the registry is built with
  `Arc::new_cyclic` so a spawned watchdog cannot keep sessions alive past app
  teardown, and generation stamps make a stale watchdog exit instead of racing
  the dial that replaced it.
- **Reconnect ≠ terminal reattach.** The pool rebinds; the terminal's process
  and scrollback come back from the HOST's tmux on reopen (ADR-0025).
- **MaxSessions is health, not state**: reconnecting cannot fix a refused
  channel. Classified from error text (`is_channel_open_failure` in
  fartcode-ssh) because russh flattens the channel-open reason code.
- `dial()` deliberately does not publish `error` — a mid-ladder failure is not
  a failed connection; the caller (or the exhausted ladder) decides.

## #88 E12-04 remote projects LANDED (2026-08-11) — ADR-0042

Projects can live on an SSH host. `fartcode-core/src/projects/remote.rs` —
`RemoteHost` trait (realpath/list_dir/stat/mkdir_all/remove_dir_all/run) +
`RemoteProjectStore` (create_remote, create_remote_clone, ensure/remove remote
worktree). `fartcode-ssh/src/host.rs` implements it over `SshClient` +
`RemoteSftp`. Commands: `remote_browse`, `create_remote_project`,
`clone_remote_project`, and `clone_project` — the local clone flow that had a
store method since E1-03 and no command (e2e FIRST-16 "unreachable-entirely").

Bites:
- **`projects.path` was globally UNIQUE** — the same path on two hosts could
  not both be projects. Migration 0012 keys on
  `(path, COALESCE(ssh_connection_id, ''))`; the COALESCE is load-bearing,
  since NULLs are distinct in a SQLite unique index and a bare two-column
  index would silently allow duplicate LOCAL projects.
- **Remote projects must not run the local open flow.** `.git/info/exclude` +
  worktree re-detection touch this machine's filesystem. Remote create reuses
  only `insert_project_row` + `ensure_repository_workspace` + `project:added`.
- `SshClient::run_command` returns **stdout only** — a failing `git rev-parse`
  reads as an empty success. `host.rs::exec_collect` walks `channel.wait()`
  for `ExitStatus`/`ExitSignal` and treats "closed with no status" as failure.
- Remote worktrees live INSIDE the project (`<p>/.fartCode/worktrees/<seg>`),
  segment hashed from the workspace key (`ssh:<conn>:<path>`), not the name.
- `RemoteSftp::remove` errors on a missing path; the trait contract is
  idempotent, so `remove_dir_all` stats first.
- SFTP session is bound to `/`; containment is enforced per op against the
  project root (`ensure_contained`), still lexical (E12-02 caveat stands).
- Tests use an in-memory `FakeHost` — no live SSH in the suite.

## #88 E12-04 remote project CREATED (2026-08-11)

Next ticket: remote projects — Pick/Clone/New over SSH + worktrees on the
remote at `<project>/.fartCode/worktrees`. DB shape already exists
(`projects.ssh_connection_id`, `repository_workspace_key` → `ssh:<conn>:<path>`);
nothing writes it — `finish_create` is always `None`. Also registers the missing
local `create_clone` tauri command (backend exists, unreachable from the UI).
18 acceptance criteria. size:L, phase:3.

## #87 E12-03 connection profiles + ssh -G LANDED (2026-08-11)

`fartcode-ssh/src/config.rs` — `ssh -G <alias>` is the source of truth; we parse
its output instead of reimplementing `~/.ssh/config`. `fartcode-core/src/ssh_connections/`
— profile CRUD, secrets in keyring (`ssh-connection:<id>`), delete blocked while
`projects`/`workspaces` still point at the row (no FK on those columns).
`SshClient::connect_alias()` ties it together; ProxyJump beats ProxyCommand.

Bites:
- **Alias goes into argv, so it must be validated first** — an alias of
  `-oProxyCommand=...` would otherwise become an ssh flag. `validate_alias`
  rejects leading `-` and anything outside OpenSSH host syntax.
- **Never guess the agent socket.** `IdentityAgent` disagreeing with
  `SSH_AUTH_SOCK` returns `AgentSocket::Ambiguous`, not a pick — forwarding the
  wrong agent hands a remote host the wrong keys.
- ProxyJump is a **direct-tcpip channel**, not a shell hop: the handshake and
  auth are end-to-end, so the bastion never sees target credentials. Every hop
  client must stay alive (`SshClient::via`) or the tunnel collapses.
- ProxyCommand runs under `/bin/sh -c` **verbatim** — `ssh -G` already expanded
  `%h`/`%p`, and re-quoting a user's shell command line would break it. The
  child is `kill_on_drop`, held in `SshClient::proxy`.
- `AuthMethod`/`SshClient` have **hand-written `Debug`** — the derive printed
  passwords and passphrases into any `{:?}` of `ConnectionParams`.
- `ssh_connections` already exists in `0000_initial.sql`; no migration needed.

## #86 E12-02 SFTP layer LANDED (2026-08-10)

`fartcode-ssh/src/sftp.rs` — `RemoteSftp` over russh-sftp 2.4 (workspace dep bumped
from the stale 0.50 line; 2.4 is the current release of the same crate).
Ops: list (dirs-first sort, hidden opt-in), read (200KB default cap, 100MB hard
ceiling, truncated flag), write (auto-mkdir parents), stat (None on ENOENT),
realpath, exists, mkdir, remove (iterative stack, no shell `rm -rf`).

Bites:
- SFTP errors are **typed** — match `client::error::Error::Status(s).status_code`
  against `StatusCode::{NoSuchFile,Failure}`. Never string-match error Display;
  an `IO("no such file")` would false-positive as ENOENT.
- `mkdir` on an existing dir returns generic `Failure` (4), not AlreadyExists —
  confirm with a stat before treating it as success, else real errors vanish.
- Path containment is **lexical** (normalize + `starts_with(root)`); a remote
  symlink pointing outside root is not caught. Harden in E12-03 if it matters.

## #86 E12-02 SFTP layer CREATED (2026-08-10)

Next ticket: SFTP filesystem layer in fartcode-ssh using russh-sftp 0.50.
Browse, read, write, stat, realpath, mkdir, remove — path-constrained to
workspace root. 14 acceptance criteria. size:M, phase:3.

## #85 SSH client layer LANDED + CLOSED (2026-08-09) — ADR-0041

fartcode-ssh crate with russh 0.50. Auth: password/key file/agent. PTY channels
(open, resize, shell). Exec (non-interactive + collect). Direct TCP/IP forward.
Handler accepts all server keys (ponytail: known_hosts in E12-03). 4 tests,
fmt + clippy clean. Foundation for E12-02..12-10.



Project-level working memory. Newest entries first. If a fact here contradicts
AGENTS.md or ARCHITECTURE.md, the docs win — update this file (and the ticket if
one exists).

## #82 chain guard + step spend ledger LANDED (2026-08-09) — ADR-0040

Run-mode column chains are now bounded and recorded. `chain_guard` in
`step_engine::settle_issues_observed` (the ONE chaining site) refuses the
next automatic launch on: cycle (visited set incl. the settling column),
depth (default cap 3 consecutive auto launches, `step_chain_depth_cap`
base setting), budget (`step_budget_tokens` vs summed reported tokens).
Hold = card stays put + `step_ledger` hold row + StepSettled +
StepChainHeld ("held · reason" card meta line; Spend section in card
detail via `step_ledger_list`). Invariants to remember:
- Chain state (`ChainState`) is MEMORY-ONLY, reset by every human gesture
  (entry epoch, confirm); the ledger (migration 0011, table 12) is the
  durable record. Restart forgets chain position by design.
- Queue targets are never guarded — the confirm gate IS the human check,
  and confirming resets depth.
- Self-`advance_to` is already blocked by column validation; the guard's
  `target == settled column` check is defense only (untestable via API).
- Ledger ordering is `ORDER BY rowid` — `created_at` is second-granular
  and uuid ids don't sort. Token backfill targets newest tokenless
  launch row per (issue, column).
- Budget fails CLOSED on ledger read error, but unreadable settings fall
  back to default-cap/no-budget (never invent a budget).
- Migration COUNT assertions now at 12 (tests/migrations.rs +
  db_integration.rs); the 0009-upgrade test drops step_ledger too.
- Nothing kills a running agent — the guard only refuses the next launch.
DESIGN DEVIATIONS (frames pending, per #82's design gate): "held ·
auto-run limit/loop detected/budget spent" copy on the card meta line;
Spend section reuses card-detail-timeline styling; ProjectSettings rows
"Step chain depth cap"/"Step token budget" as InlineInputs.
Suites: 888 workspace tests + 243 frontend; fmt + clippy clean.

## #81 worktree pool segments LANDED (2026-08-09, fef5f49 + 8ef46f2) — ADR-0039

Pool dirs are now unique per project: `projects.worktree_pool_segment`
(migration 0010) stores `<safePathSegment>-<hash8(stored path)>`; the
one-shot kv-gated adoption pass (`worktree_pool_adoption_v1`, runs in
`DbProjectStore::new`) stamps legacy segments in place when unique and
moves+repairs (`git worktree repair`, new `GitOps::worktree_repair`)
colliding projects' worktrees out of shared pools. Deleting a project can
no longer destroy a same-basename sibling's worktrees (FIRST-58). The
per-project `worktree_directory` setting is finally consumed by
`worktree_pool_path` (invalid override falls back to app default).
Review round: 9 findings, 8 CONFIRMED (1 high), 1 refuted — all fixed in
8ef46f2. The hardened invariants to remember:
- Gate is set ONLY on a fully successful pass; partial passes retry next
  startup (stamping is `WHERE segment IS NULL`, moved rows repoint via the
  `!old_path.exists()` branch — re-runs are churn-free).
- Adoption is override-aware: legacy pools under the app default are
  relocated to the project's override root (pre-#81 the override was dead,
  so that's where every legacy pool lives).
- Repairs are a post-move SWEEP over every row now under the new pool;
  any repair failure blocks the stamp → retried, never half-linked.
- Sole-member groups check the segment isn't already held before adopting
  it in place (crash-between-stamps can't rebirth the shared-pool hazard).
- Delete teardown canonicalizes both paths (symlink guard) and SKIPS
  `remove_dir_all` when another project's workspace rows still live in
  the pool dir (half-adopted leftovers survive the keeper's delete).
- `remove_stale_path` refuses unless cleanliness is PROVEN, and
  `CliGit::is_worktree_clean` errors on non-zero exit (broken linkage is
  UNKNOWN, not clean).
LESSON: the round-1 HIGH was masked by a flat test fixture — production
worktrees nest under the branch prefix (`pool/fartCode/<branch>`), the
fixtures used `b1`. Fixtures must mirror production path layout.
Suites: 881 passed / 0 failed / 1 ignored (env-gated probe); migration
COUNT assertions at 11; fmt + clippy clean.

## #76 memory value dashboard LANDED (2026-08-09) — E19 CLOSED (#69)

732c391 + 73de08e. Memory pane at settings → project → Memory renders
#73's four signals AS STATES: Unknown/Insufficient/NoData/SinglePoint are
honest blanks (headline "no memory signal yet", never "0 re-explanations
avoided"); the time-to-land caveat renders verbatim from the payload's
`caveat` field, unconditionally; no sparkline/arrow for SinglePoint.
- Backend extension: `TimeToLandKind::Trend` gained `landed_hours`
  (landing-ordered cycle hours) so the §8g sparkline is real data, not
  glyphs fabricated from two medians. Series lives INSIDE the
  caveat-welded private type; Copy dropped from TimeToLand(Kind), read()
  clones; compile_fail doctest intact.
- Wire format: the tagged telemetry enums (TimeToLandKind, ReAskRate,
  TokensSaved) now serialize variant FIELDS camelCase
  (`rename_all_fields`) — verified nothing persists these shapes; the
  dashboard is the only consumer.
- SettingsModal section parsing generalized to `project:<id>[:<child>]`;
  Memory joins Columns as a nav child.
- Review round: 7 findings, 6 confirmed, 1 refuted. Fixed: citations row
  dropped `unknownWithHit` — the excluded-but-cited count that makes the
  rate a floor travels with the exclusion now.
- DESIGN DEVIATIONS listed on #76 for design review (do not auto-fix):
  "last 90 days" vs frame's "this month" (window is 90d — "this month"
  would be false, and the test certifies current copy); prose sentences in
  mono value slots for empty states; pre-existing TREND_CAVEAT copy vs
  §8g wording (ADR-0038 vs frame conflict); dangling "Memory · " /
  "Columns · " title when a project is deleted mid-view.
Suites: core 306, app 212 across suites, telemetry 65 (+1 doctest),
frontend 242. fmt + clippy clean. One fartcode-app lib test flaked once
(103/1) and passed clean on re-run — unidentified, watch for recurrence.

## #70 feature dossiers: file lifecycle LANDED (2026-08-09) — E19-01

ADR-0038 items 1–2, backend only. Migration 0009 adds
`issues.dossier_path` (nullable, NULL everywhere on upgrade). The dossier
is born in `dispatch::provision_issue_task` — the ONE helper board
dispatch and the step engine's first `agent_step` entry share — AFTER the
worktree exists and written INSIDE it, so it rides the feature branch.
Being there is what makes "first step entry" the single birth moment: a
card with a live linked task never reaches the helper, so a second step
column cannot mint a second file.
- `fartcode-core/src/dossiers.rs` owns content + file ops (slug, header,
  append); `fartcode-app/src/dossiers.rs` owns lifecycle (consent,
  worktree resolution, `dossier_path`, the bus subscriber).
- SLUG reuses `generate_task_name(Some(title), None, false)` — the exact
  call `create_task_params` makes for the branch name. No second
  slugifier. Unsluggable title → the issue id.
- CONSENT: `BaseProjectSettings.feature_dossiers: Option<bool>` (base,
  NOT shareable — consent to write into a checkout is local).
  **FAIL-CLOSED: `Some(false)` AND `None` (never asked) both refuse**, as
  does an unreadable settings row. Unset is not an edge case — nothing
  can set the field until #74, so it is every project. The first cut had
  `unwrap_or(true)` and the review found it 8× independently: the
  dispatch prompt tells the agent to commit as it goes, so an
  unrequested dossier rides the branch into the user's PR. Feature is
  inert until #74 writes `Some(_)` on BOTH answers. **Consent is
  re-checked on every append**, not just at creation — an existing
  dossier is not standing permission.
- PROVENANCE is derived, not stored: `external_ref` → import, else
  `prd_path` → proposal, else manual. Known imprecision: a PRD-less
  proposal reads as manual. A real column is the fix if anything but the
  header ever wants it.
- ADOPTION IS NARROW. `docs/features/` is a common hand-written
  convention, so "the file exists" is never permission. `inspect()`
  classifies Free / OurDossier / OtherDossier / Foreign; only a file
  carrying `DOSSIER_MARKER` (or a Timeline section) AND this card's
  `- card: \`<id>\`` line is adopted. Anything else → step aside onto
  `<slug>-<short id>.md`, then random suffixes. Two same-titled cards get
  two files instead of interleaving.
- CARD TEXT IS DATA, NEVER STRUCTURE. Body is heading-demoted, one-line
  fields are `inline()`d. The Timeline anchor is a machine sentinel
  (`<!-- fartcode:timeline -->`) resolved before the visible heading, and
  the heading fallback uses `rposition`. Without this a card body
  containing `## Timeline` captured every breadcrumb.
- WRITES ARE ATOMIC: temp-file (uuid-suffixed, same dir) + rename, so a
  crash can't leave a truncated corpse that adoption then inherits. The
  read-modify-write window is narrowed by len+mtime re-stat before the
  rename; after 4 lost races it errors rather than clobbering. Not a
  lock — documented as such.
- APPENDER (`TimelineAppender`) is STATELESS: StepLaunch (skips
  reattach), StepSettled, **`IssueColumnChanged`** (new event, emitted by
  `enter_column`, carries from+to because only the emitter knows them),
  PrUpdated. The old in-memory last-column map is gone — it read the
  column at handler time, so rapid moves recorded the wrong "from". PR
  once-key dedupe is LINE-ANCHORED (`ends_with`), not `contains`: PR #1
  was being masked by an existing #12 line.
- Creation failure NEVER fails dispatch: `create_for_task` returns the
  UPDATED issue, so the caller has nothing fallible left to do (it used
  to re-read and propagate the read's error).
Suites: core lib 240, app lib 91 + 17 integration, frontend 180.
Fixed in passing: `tests/migrations.rs` and `tests/db_integration.rs`
migration COUNTS had been stale since #66 added 0008 (both binaries red
on clean main); now 10.

## E19-04/05/06 LANDED (2026-08-09) — #73, #74, #75 closed; only #76 left in E19

#74 consent card (99878b5 + 89881cf): THE switch that turns dossiers on —
everything before it was inert (fail-closed None). Gate now lives in
store/dossierConsent.ts with the card rendered app-level, awaited by ALL
THREE entries (board enterGated, taskPipeline.enterColumn, CardDetail
dispatch). Frontend 213.

#75 card detail + ⌘K (a6a075e + e88fe26): §8f Dossier group, §8h feature
rows, PALETTE_HIDDEN_TYPES removed. Core 302, frontend 229.

#73 telemetry (0e5c1c6 + b1cd345): four local signals; new Db::kv_update
for atomic RMW. Telemetry 64, core 306, app 212.

THREE RULES THIS ROUND PAID FOR — all found by review, all would have
shipped:
1. A CONSENT DECISION MUST BE BOUND TO THE QUESTION ASKED, never to
   ambient state. #74 wrote the answer against the CURRENTLY selected
   project (BoardView is not remounted on project switch), so answering in
   A could grant consent for B — or flip a B that had DECLINED — then
   dispatch A's card into an unviewed project. And the backdrop click,
   which means "nothing happened" on every other overlay in this app,
   declined permanently AND dispatched. Now: ask carries its projectId,
   withdrawn on project change; backdrop inert (Onboarding precedent).
2. A METRIC THAT CAN ONLY FLATTER ITSELF IS WORSE THAN NONE. #73's
   injected-prompt exclusion was right for ACP and the PTY path (the
   MAINSTREAM one) flattened scrollback into one unprovenanced segment —
   the echoed seeded prompt contains the dossier path AND both tag
   literals, so citations read 100% by construction and re-ask a
   fabricated 50%. Fixed by making unstructured scrollback UNSCANNABLE
   rather than masking a reconstructed span (a TUI reflows/redraws/
   truncates its echo, so a partial match silently restores the bug).
   Corollary now enforced: the TEACHING text must not parse as the thing
   it teaches — skills.rs asserts its own prompt tallies to zero.
3. A FIXTURE THAT HAND-BUILDS THE ARTIFACT CERTIFIES A FICTION. #75's
   tests injected a `· settled` line and claimed the appender wrote it —
   but on the SEEDED board no step ever writes one (both agent-step
   columns are on_settle: Advance; StepSettled fires only on Hold arms),
   so every step rendered as permanently `running · 3w` with a live
   ticker. Fixtures now drive settle_issues_for_task for real.

Also: three readers of the dossier file now share ONE anchor
(dossiers::timeline_block) — #75's viewer had resolved the Timeline block
by heading text while the appender uses the sentinel, so an agent writing
a bare `## Timeline` in prose stole the block.

DEFERRED WITH REASONING, not forgotten: #83 (`· landed` needs base-ref
ancestry — working-tree presence is not ancestry, and answering it right
means base-ref resolution + git cat-file per keystroke). §8f artifact-diff
rows still blocked on a board_columns artifact field + migration — TWO
tickets have now bumped into it, worth its own.

NEXT: #76 memory value dashboard (§8g) is the last E19 ticket, then close
epic #69. It renders #73's four signals — the labels now carry Unknown/
Insufficient/Estimated states and the time-to-land caveat is structurally
inseparable from the value, so the dashboard MUST render those states
rather than coercing them to numbers.

## E19-02 + E19-03 LANDED (2026-08-09) — #71, #72 closed

#71 (a5f24c6 + 2ae341e): `.claude/skills/feature-log/` + one AGENTS.md
pointer line + the step-prompt append instruction, all versioned together
by `FEATURE_LOG_VERSION`. Scaffold is written into the WORKTREE (rides the
feature branch, lands in the PR where the user can drop it). The append
instruction is injected at PROMPT-ASSEMBLY time — never stored in
SEED_COLUMNS, whose step_prompt is NULL by design; storing it would write
a dossier instruction into the DB before consent is asked and survive
revocation. `feature_log_seeded_version` (base, non-shareable) makes a
user's DELETION STICK while a version bump still self-heals — the first
build printed "delete these files to remove the convention" while
re-seeding on every provision.

#72 (b5e108c + 74d6503): dossier sections indexed as `feature` rows;
reindex on settle (inside settle_issues_for_task — the ADVANCE branch
emits NO StepSettled, so subscribing to the event would miss it), on
project pull, and a boot sweep (search::backfill wipes the WHOLE table at
launch). item_id = `<issue id>#<heading>`, ordinal deduped on the EMITTED
id. Feature rows are held out of ⌘K by PALETTE_HIDDEN_TYPES until #75
lands their row style — a unit test over the constant makes #75's removal
a visible deletion.

THE LESSON OF THIS PAIR (write it on the wall): #72's parser factoring
made fences structural and REGRESSED #70's hardening — untrusted card
text could again change how the file parses below it. An unclosed fence
in a card body (a pasted stack trace) swallowed every heading below it:
breadcrumbs landed at EOF, and on the index side later sections went
invisible AND their still-present rows were pruned. Fixed at both ends
(anchor-relative scan with fresh fence state; demote_headings CLOSES an
unbalanced fence rather than mangling markers — a balanced block is
legitimate content). Separately, #72's landed-copy fallback indexed ANY
file at the path — the exact adopt-any-file bug #70's review fixed ONE
TICKET EARLIER. RULE: when you make a new thing structural, re-audit
every trust boundary that existed to contain untrusted text; and never
resolve a file by path alone — `inspect()` it.

Suites after both: core 283 lib, app 174 across suites, frontend 180.
Chipped: fartcode-terminal env_policy_controls_inheritance is flaky under
workspace parallelism (mutates process env). Open pre-existing: settings
update_project_settings is FULL-REPLACE, so a stale open settings pane
can clear feature_log_seeded_version / re-grant consent — wants a
patch-shaped update command.

## E19-01 DOSSIERS LANDED (2026-08-09, 6217dd9 + bbaca0c) — #70 closed

Migration 0009 adds issues.dossier_path. Dossier is born INSIDE the
worktree at first agent_step entry (in provision_issue_task, the shared
helper), with a backfilled header (title/body/acceptance, PRD link,
derived provenance, blockers) and a `## Timeline` section the app owns.
TimelineAppender subscribes to the bus (StepLaunch non-reattach,
StepSettled, IssueColumnChanged, PrUpdated) and appends breadcrumbs only
while a worktree exists. Dossier work NEVER fails a dispatch.

REVIEW ROUND WAS THE BIG ONE (40 agents, 34 verdicts, 24 stood -> 8
distinct defects, 10 refuted). Rules that came out of it, all now
enforced by tests:
- CONSENT FAILS CLOSED. `feature_dossiers: Option<bool>` is base (NOT
  shareable — consent to write into a checkout must never ride a
  teammate's .fartCode.json). None = never asked = DO NOT WRITE. The
  first build had unwrap_or(true), which would have committed unrequested
  files into users' PRs (the dispatch prompt tells agents to commit as
  they go). #74 must persist Some(_) on BOTH answers.
- Consent is checked on the APPEND path too (TimelineAppender::target),
  not just at birth — one choke point for all four event arms.
- NEVER adopt a file just because the path exists. `inspect() -> Occupant
  {Free, OurDossier, OtherDossier, Foreign}`: adoption needs the
  DOSSIER_MARKER *and* this card's `- card:` line; anything else gets a
  disambiguated path. docs/features/ is a common hand-written convention —
  we were one line away from appending machine breadcrumbs into people's
  own specs.
- User text can FORGE our anchors: card bodies are heading-demoted, and
  the Timeline anchor resolves by `<!-- fartcode:timeline -->` sentinel
  (rposition heading fallback).
- Repo writes are atomic (uuid temp + rename, mirroring
  settings/service.rs) with a re-stat/recompute window. A plain fs::write
  is O_TRUNC: a crash left an empty dossier that adopt-never-clobber
  would then happily adopt.
- Column moves emit InternalEvent::IssueColumnChanged {from,to} from
  enter_column (which holds both endpoints) — never re-read state at
  handler time. The appender is stateless now.
Suites: core 240, app lib 91 + dossiers integration 17, frontend 180.

OPERATIONAL: a review VERIFIER agent edited source in the build worktree
(neutered on_pr_updated to test a finding) and restored its backup to the
wrong path. Harness flagged it; both trees verified clean before landing.
Review agents must read, not mutate — check `git status` in the worktree
after any review round before cherry-picking.

## #68 templated confirms + park rehydration LANDED (2026-08-09, 63090ba + 508d00b) — #68 closed

Most of §8c had landed with the render round; this closed the three real
gaps. Blocked confirm names each blocker's column via blockerColumnName
(multi-column: "#b (Quick), #c (In Review)" parentheticals — designer-
bound deviation). ProposalCard derives "approve N → <landing>" from the
columns store (generic copy while unloaded, never a wrong name).
New `step_parked_list` read-only command (async + spawn_blocking) +
store/steps.ts `hydrateParkedSteps`: parks survive webview reload;
events win over hydration via PER-HYDRATION TOMBSTONE COLLECTORS —
clearIssue/clearPark record into every in-flight hydration's set, so a
park cleared between the IPC resolve and the seed can't resurrect as a
ghost confirm (the review's one high). Suites: app lib 91, frontend 180.
Review round: 6 deduped, 2 confirmed (the ghost-park race + unguarded
BoardView mount wiring), 4 refuted. Pre-existing acp_e2e_integration
failures are environment-dependent (fail identically on clean main).

## #66 AUTHORITY FLIP LANDED (2026-08-09) — column_id owns placement

E18-07: `issues.column_id` is authoritative end-to-end; `lane` is a
derived display mirror (synced from seed_lane on seeded-column entry,
frozen otherwise). Migration 0008 (append-only) backfills every
mirrorless row (seeded-lane match, else the landing column); every write
path now enforces a non-NULL column. Consequences that bite:
- `move_to` is a wire-compat adapter over `enter_column`; a lane whose
  seeded column was deleted is a TYPED error (the `column_id = NULL`
  fallback write is dead). Same for create's lane override / a project
  with no landing column.
- BLOCKED_SQL is the single join `c.id = b.column_id`; renumbering and
  board order come from column position (the lane-rank CASE died).
- Delete guard: occupancy strictly by column_id; the temporary
  seeded-agent-step lock is LIFTED; deleting an `advance_to` target is
  REFUSED with `Error::BoardColumnIsAdvanceTarget` naming the referrer
  (decided: refuse, never FK-degrade — next-by-position rerouting is the
  ADR-0037 item 4 unconfirmed-dispatch spend hazard). Frontend delete
  reasons: occupied → landing → advance-target, one at a time.
- `issue_dispatch` resolves the seeded In Progress column from config up
  front (typed error if deleted, before provisioning a worktree).
- Wire shapes unchanged: IssueDto.columnId stays `string | null`;
  columnIdForIssue stays as defensive display resolution.
Suites: core 215, app 89, frontend 168 (all green, workspace gate too).

Review round (16 agents): 11 deduped findings, 7 stood, 4 refuted — all
7 traced to ONE root cause the flip created: paths that became fallible
(typed deleted-seeded-column errors) whose callers still ran destructive
step-engine side effects BEFORE the fallible call. Fixed in 144cbb0:
`issue_move` now moves first and applies epoch/park side effects only on
success (`on_lane_move_committed`, keyed on the captured PRE-move lane);
`issue_dispatch_blocking` prechecks issue + In Progress resolution before
dropping the park. Regression tests prove a refused move/dispatch leaves
the park, epoch, and launch registry untouched. RULE going forward: when
an infallible path becomes fallible, audit its callers for
side-effects-before-call ordering — this class produced 7 of 7 standing
findings.

## #67 Columns editor LANDED (2026-08-09, 64821b5 + abbca79) — #67 closed

The column CRUD commands have their first UI callers; the configurable
pipeline is no longer console-only (GAP-10/BOARD-46 closed). New
ColumnsEditor.tsx (ColumnsPane) + nested `Columns` nav child in
SettingsModal (renders only under the ACTIVE project row; prefix-safe id
match). Summaries via columnConfigSummary + columnSublineTone — board and
editor stay byte-identical, one formatter. Frontend suite: 165 tests.

Review round: 19 raw -> 15 confirmed, 3 refuted, all fixed in abbca79.
The one to remember: stepTools [] (fail-closed allowlist) and null
(unrestricted) rendered identically in the tools editor and blur-save
patched unconditionally — open-then-click-away silently granted
unrestricted tools. Editor now guards every blur-save on actual change,
renders [] as "none", splits tools on NEWLINES only (`write plan.md` is
one tool — the frame's own example), keeps one `busy` flag across all
mutations, and derives occupancy live (event-subscribed, refetched in
mutate, re-checked at delete click so the backend refusal stays
unreachable per §8d).

10 design deviations listed on #67 for the designer (notably: no Memory
child until #76, `--focus-bg` for the frame's nonexistent `--focus-row`,
no "v<N> of the seeded template" until E19 versions templates). Known
pre-existing, chipped: eslint flat config never registers react-hooks, so
5 exhaustive-deps suppressions reference an unregistered rule and
`npm run lint` fails on main. NEXT: #66 authority flip (checklist on the
ticket), then #68 templated confirms.

## UI-thread audit LANDED — #80 closed (2026-08-09)

41 of 101 tauri commands moved off the macOS main thread (async +
spawn_blocking; 49 async command fns now, was 8). All 16 git commands
shell out (App::init wires CliGit, so the whole GitOps trait is
std::process::Command) — they route through `off_main_thread` in
commands/git.rs, a tested spawn_blocking wrapper. fartcode-git subprocesses
gained a bounded wait; they had NO timeout, so an unreachable remote hung
the app forever. Wire contracts unchanged — zero frontend edits.

GUARD: `fartcode-app/tests/no_blocking_tauri_commands.rs` enforces two
rules (async-or-SYNC_OK, AND known-blocking must actually offload — async
alone just moves the stall to a tokio worker). Rides cargo test into CI +
make check. Rule written into AGENTS.md §"Tauri commands and the main
thread". To exempt a command, add it to SYNC_OK with a justification; to
add a new offload helper, teach the guard its name (it knows spawn_blocking
and off_main_thread).

`make check` now clears fmt-check + lint; the ONLY remaining failure is the
3 pre-existing tmux durability tests (chipped, fartcode-terminal untouched
since 34e26ff). Known follow-up: pr_section_get / pr_section_sync are async
but do keyring + git subprocesses inline before their first await — they
stall a tokio worker, not the UI.

## Task view carries pipeline context (2026-08-09, 273cb68, #79 closed)

E18-10: the task view had ZERO board awareness — a step settling in a hold
column rendered only on the board while the user was in the terminal. Now
the 46px header crumb reads `project / <column> / <ref>` (resolved via the
board's own columnIdForIssue) and the actions row carries four key-labelled
actions: advance ⌘⇧→ (advanceTo ?? next, matching settle_issues_for_task),
confirm parked step ⌘⇧D (names provider·model·effort, never spends on the
press), move-to-column ⌘⇧M (key-first picker through issueEnterColumn), open
card detail ⌘⇧I. Agent dot re-derived from the live agent terminal, NOT
task.status. `⌘N new task` added to the header because the flyout's button
vanishes when collapsed (⌘N was always global — the gap was mouse-only).
Cardless ad-hoc tasks render no pipeline chrome. Frontend suite: 146 tests.
11 design judgments listed on #79 for the designer — this extends handoff v2
§5a with no frame.

## OPERATIONAL: agent worktrees start STALE (2026-08-09)

Three agents this session branched from an old commit (34e26ff) rather than
current main and had to be corrected mid-flight; one rsynced the shared
checkout into its worktree to compensate. ALWAYS
`git -C <worktree> reset --hard main` before dispatching work to a worktree
agent, and tell the agent its base explicitly. Cherry-pick only the agent's
own work commit — verify with `git show <sha>^:<file> | shasum` against
main's copy before picking.

## UI-thread blocking found; DB re-entrancy RULED OUT (2026-08-09)

Chasing three deadlocked agent `cargo test` runs (0% CPU, 45-75 min — all
in agent worktrees, main is clean) turned up something bigger, filed as
issue #80: **93 of 101 tauri commands are non-async, so they run inline on
the macOS main thread and freeze the window** (non-async #[tauri::command]
→ ExecutionContext::Blocking → inlined into the invoke handler → wry's
main-queue callback). Verified to the tauri-macros source + 2 probe tests.
Worst: create_task (git fetch, unbounded), delete_task (5s spin-sleep PER
LEAF), git_push/pull/fetch/create_pr (Command::output with NO timeout),
issue_enter_column/step_confirm (our new dispatch path). Only 8 commands
are async. Fix is async + spawn_blocking (async alone just moves the block
to a runtime worker).

RULED OUT, do not re-investigate: DB `Mutex<Connection>` re-entrancy. 10
candidate hazards raised, ALL 10 refuted — no reachable production path
holds the guard across a re-acquiring call; the code scopes guards
deliberately (see commands/git.rs:258). The mutex is a contention
amplifier behind main-thread holders, not a deadlock source. The agent
test deadlocks were test-authored (guard held across store.get()).

## E2E scenario catalogue + board fix round (2026-08-09)

`docs/e2e-scenarios.md` (e535a1a): 449 scenarios over 8 journeys, 153
deduped gaps (44 high), authored by reading the implementation not the
specs. Status vocabulary marks unreachable/not-built honestly. USE IT as
the gap backlog and the E2E test spec. Highest-severity findings not yet
ticketed: worktree pool keyed on project NAME (two same-named projects
share a pool; deleting one destroys the other's worktrees), `curl|bash`
agent install with no confirm, delete_project does no process teardown,
task.status never changes so needs-you can never render, unbounded chained
spend (no depth cap/budget on run-mode column chains). No E2E driver
exists for the Tauri app; the doc separates backend-command-drivable
scenarios from ones needing tauri-driver.

E18-07 fix round landed (69262eb) closing all 16 review findings. Notable:
step events now live in an app-lifetime store subscription (store/steps.ts)
because BoardView unmounting on dispatch was eating settle-chained
launches; re-entry PROBES FOR A LIVE AGENT before writing — the fix agent
correctly argued down my backend-guard lean, since `reattached` answers
"did the card re-enter its own column", not "is an agent running", and
TerminalManager is unreachable from &App. Frontend suite: 108 tests.

## E18-06 + E18-07 landed; board renders from config (2026-08-09)

Commits: 5628ab0 (E18-06 entry paths → is_landing + PM prompt from column
config), c340fbd (E18-07 board renders N columns — columnConfigSummary in
lib/columnConfig.ts is THE shared formatter, #67 must reuse it; new
store/columns.ts; consumes step:launch/queued/queue_cleared/settled),
e2d1de1 (PM prompt regression fix), a789600 (E18-06 review fix round),
f4116f1 (ADR amendment). #65 closed.

REVIEW FINDINGS THAT CHANGED THE DESIGN: (1) ADR-0037 item 7 now says a
landing column is NEVER an agent_step — entry paths write rows directly and
never fire on_enter, so a run-mode landing column deposits inert cards, and
routing creation through the engine would make a 50-issue import launch 50
agents. Work dispatches by MOVING onto a step, never by arrival. (2) Delete
guard ownership: the mirror owns a card whenever set; lane mapping covers
only mirrorless pre-E18 rows (was double-counting). (3) PM prompt
ticket-edit example was the exact shape parseTicketEdit rejects;
PM_PROMPT_VERSION now 3.

Still open on the board: E18-07's authority-flip half (column_id
authoritative, BLOCKED_SQL join, delete-guard switch, lift the
seeded-agent-step delete guard, In Review pin degradation) — deliberately
split out of the render round; checklist is on #66.

## v2 WIP committed at last (2026-08-09, 3adb7a1)

The design_handoff_v2 implementation had been sitting UNCOMMITTED (114
files, +16k/−4.5k) while three stash dances rode over it. Now committed as
one commit together with today's ADR-0037/0038 + design brief. `.claude/`
(5.9 GB of agent worktrees, previously untracked-but-not-ignored) added to
.gitignore — never commit it. `fartCode.zip` left untracked deliberately.
Consequence for agents: the UI wave now branches from a base that CONTAINS
the v2 board/task-view/PM-chat work — never rewrite those files from
scratch, always read first.

## E18-04/05 STEP ENGINE LANDED (2026-08-09, 5e8c017) — E18 backend COMPLETE

Squash of three worktree commits (build aa30918 + fix bf9a4a1 + final
8757ab4) cherry-picked onto main; 7-file stash dance, one conflict (app.rs:
engine's steps/Step events + WIP's host_dependencies/SettingChanged — both
kept). Combined tree: core 201 lib + suites green, app lib 42 green, tsc
clean. Migration 0007 pins In Progress advance_to → In Review (0006
untouched — LANDED MIGRATIONS ARE HASH-FROZEN, never edit). Restart
contract: parks/registry in-memory; settle re-parks queue columns after
restart (never advances through an unconfirmed gate). Ticket bookkeeping:
filing error had duplicated E18-03 (#63 dupe of #77) and never created
E18-04 — refiled as #78 (closed); corrected map on epic #60. Closed: #61
#62 #77 #78 #64. Next: #65 (E18-06 landing), then UI wave #66-#68, then E19.

## E18-01/02 LANDED on main (2026-08-09, b1ddde2)

Spike cherry-picked onto main (linear history; worktree branch commit
be04415). Landing dance: main had 102 dirty WIP files, 4 overlapped — stash
push on those 4 → cherry-pick → stash pop; one conflict (app.rs: spike's
`columns` store vs WIP's `host_dependencies` store, both kept), stash
dropped after resolve. Combined tree verified: cargo check fartcode-app +
tsc clean. E18-03 LANDED too (ade8d63, clean pop, #77 closed): BLOCKED_SQL +
dispatch blocker filter key on counts_as_done via seed_lane resolution;
BlockerRef.countsAsDone exposed to the frontend DTO. E18-04/05 step engine
building now in the same worktree — architecture: issue_enter_column
primitive (column_id always, lane synced via reverse seed_lane, unchanged
for non-seeded), on_enter queue = park + step_confirm command, settle reads
current column config (advance→enter(advance_to ?? next), hold→step-settled
event, step-done is DERIVED), reattach-never-respawn preserved, two golden
parity tests (In Progress drag + auto-flip). Adversarial review gate before
landing. REVIEW RESULT (20 agents): 14 CONFIRMED defects in aa30918, 2
refuted (acyclicity concern refuted — do not add validation). Root cause of
~half: settle is task-scoped with NO session identity — stale sessions
bypass the confirm gate (verifier repro: two settles from one session
marched a parked card into Done), walk advance chains, double-launch. Also:
seeded In Progress advance_to must be PINNED to In Review (NULL next-column
reroutes to Done if In Review deleted/reordered); parks leak on issue
delete; confirm_step check-then-act race; reattach discriminator ignores
the seed_lane fallback. Fix round dispatched: in-memory launch registry
(session-scoped settle + tombstones + restart fallback), pinned gate +
temp seeded-agent-step delete guard, park lifecycle, discriminator
alignment. Fix round bf9a4a1 mapped all 14 → tests; 3-agent
verify then found: park atomicity SOUND; two NEW blockers — (1) bf9a4a1
edited landed migration 0006 in place (sha256 startup failure on applied
DBs; pin must ship as 0007) — NEVER edit a landed migration, they are
hash-frozen; (2) restart-state confirm-gate bypass via the no-entry
heuristic on queue+advance columns (fix: heuristic refuses/re-parks on
on_enter=Queue); plus consumed-set lifetime regression breaking the E17
rework loop (fix: clear consumed per column entry). Final fix round
dispatched. Landing BLOCKED until green + my 0006-diff-empty check.

## Handoff v3 accepted — ADR-0037/0038 now BINDING (2026-08-09)

`~/Downloads/design_handoff_v3/` (README + FLOWS §5 + turn-8 frames 8a–8h)
accepted BOTH ADRs at design review; statuses flipped to accepted. Design
gate lifted from #66/#67/#68/#74/#75/#76 (label removed). DESIGN.md gained a
"Pipeline board (handoff v3)" section (step-done dot, header kind sublines,
run-mode sublines at --text-muted #9a9aa1 — existing token, sidecar
unchanged, landing tag never green, counts_as_done drives dimming,
delete-with-issues = disabled label not dialog). ERRATUM resolved by user:
v3's seed line says In Progress on_enter=queue — seed stays RUN
(behavior-identical migration wins); queue is a settings flip. Adopted from
v3: Quick seeds claude·haiku (spike updated). Dashboard placement: settings
→ project → Memory. Frames 2d/2e/4d remain archived non-spec (FLOWS §3.5).

## E18/E19 filed + design brief + schema spike (2026-08-09)

ADR-0037 → epic #60 (E18 configurable pipeline columns): #61 schema/seed,
#62 CRUD, #77 counts_as_done, #63 step engine, #64 settle, #65 landing/PM
prompt; design-gated #66/#67/#68. ADR-0038 → epic #69 (E19 feature
dossiers): #70 dossier birth, #71 skill seed, #72 FTS, #73 telemetry;
design-gated #74/#75/#76. `design-gate` label = held for frames.
`DESIGN_BRIEF_E18_E19.md` (repo root) is the designer punch list. User
explicitly overrode the design gate to start an E18-01/02 schema spike
(worktree `.claude/worktrees/agent-aae7632299c6f64d3`, UNCOMMITTED) — lane
stays authoritative, column_id mirrors. Spike passed an adversarial review
round: 6 confirmed defects fixed in place (0006 edited pre-commit, not
0007). Model change that fell out: `advance_to` target column on on_settle
(ADR-0037 items 1/4 amended — without it Quick advanced into In Progress
and double-dispatched) + `seed_lane` mapping so the delete guard derives
occupancy from the authoritative lane. Tri-state null-clear contract on
column_update (omit=keep, null=clear); step_tools fails CLOSED (corrupt →
empty allowlist, Some([]) ≠ None=unrestricted). Latent twin of the
null-clear bug exists in issues.rs UpdateIssueRequest (chip filed). All
suites green: fartcode-core 192, fartcode-app lib 20, tsc clean.

## ADR-0038 drafted: feature dossiers (2026-08-09)

`decisions/0038-feature-dossiers.md` (status: proposed, companion to 0037) —
per-feature `docs/features/<slug>.md` born with the worktree at first step
entry; app appends event-driven Timeline breadcrumbs, step prompts instruct
agents to append decision sections; convention seeded into managed repos as
`.claude/skills/feature-log/` + AGENTS.md pointer (OPT-IN, provenance-tagged
— never silently write a user's repo); sections indexed into the existing
FTS5 `search_index` as item_type "feature" for ⌘K. Moat decision settled:
repo owns the memory, app owns the intelligence (index/links/dashboard) —
app-owned storage REJECTED as it blinds outside-app agents; value telemetry
(citations, re-ask rate, tokens saved, time-to-land) computed locally in
fartcode-telemetry. Leftover questions settled 2026-08-09: consent asked at
FIRST DISPATCH (reversible via settings switch); dossier born with header
backfilled from issue/PRD/proposal; ⌘K feature hits open the CARD DETAIL
(gains a dossier section); transcript indexing deferred until citation
metrics justify it. In 0037: seeded order Backlog·Ready·Quick·In Progress·
In Review·Done; narrow mode SCROLLS, never caps. Held for DESIGN REVIEW
with 0037.

## ADR-0037 drafted: configurable pipeline columns (2026-08-09)

`decisions/0037-configurable-pipeline-columns.md` (status: proposed) —
columns become per-project data (`kind` shelf/agent_step/human_gate, per-step
prompt/model/tools, `on_enter` run/queue, `on_settle` hold/advance,
`counts_as_done`, `is_landing`); one task+worktree per card with steps as
successive sessions; classic five + a gateless "Quick" express column seeded
(express is a place, not a per-card flag; ⌘N ad-hoc stays the board-free
path; drag-skip stays legal).
Held for DESIGN REVIEW — do not start building against it until the user or
a handoff accepts it. Supersedes ADR-0032 items 2/4 if accepted.

## Rail tile click reopens flyout (2026-08-09)

User-settled interaction addition (not in the left-nav handoff, which only
specifies ⌘\\ to toggle): clicking a project rail tile now also
`setSidebarVisible(true)`, so a collapsed flyout has a mouse path back.
Auditors: not a deviation — do not revert to spec.

## v2 audit + fix round (2026-08-09, same day)

A 9-auditor fidelity audit + build gate ran after the implementation; 76
findings, all closed except the held-open design-review list below. Notable
behavioural fixes: agent-launch now waits for a green auto-run setup
(create_task defers via TerminalManager::wait_for_exit; ⌘T refuses during
setup and opens the drawer after a failed one); lifecycle scripts echo
`$`-prefixed dim command lines and append a red/dim `<type> exited <code> ·
<elapsed>` tail line; the task-header dot reads the LIVE agent terminal
(task.status never changes today — do not derive agent state from it);
`set_default_agent` command + `setting:changed` event landed (settings
Default-agent row is a real picker); the diff view dropped
@codemirror/theme-one-dark (removed from package.json) for a token theme;
board Enter routes to the task on failed cards; lane labels are sentence
case. Legacy CSS is fully dead-checked (scripted top-level-block checker vs
className usage — 0 dead blocks).

## design_handoff_v2 implemented — all 12 surfaces (2026-08-09)

`~/Downloads/design_handoff_v2/` (README + FLOWS + frames) is implemented on
top of the v1 nav. DESIGN.md is REWRITTEN to this system ("The Quiet
Terminal") and formally supersedes the 2026-08-05 emdash-world decision;
`.impeccable/design.json` regenerated to match. What landed:

- **Backend commands added** (thin over existing core): `task_archive`/
  `task_restore` (+ `task:archived`/`task:restored` events), terminal DTO
  `running`/`exitCode`, `project_settings_share`/`project_settings_provenance`
  (keys: preservePatterns|shellSetup|scripts), `host_dependency_list/install/
  update/registry_summary` (HostDependencyStore now in App state). TS
  wrappers in `lib/tauri.ts`.
- **Task view** (5a/5b/7b): `TaskHeader` (46px breadcrumb + script
  launchers + changes toggle), `tv-empty` stopped state, `Drawer.tsx` ⌘J
  bottom sheet hosting lifecycle-script terminals via `store/scripts.ts` —
  the `lifecycle-script` tab kind is GONE from the tab registry.
- **Keymap (FLOWS §3.5 settled)**: ⌘T resume-agent · ⌘⇧T new terminal ·
  ⌘J toggle-drawer · ⌘. stop-agent (SIGINT to the live agent PTY) ·
  toggle-right-panel moved to ⌘⇧. · git fetch/pull/push/publish are
  palette-only commands · archived tasks restore via ⌘K search. No ⌘1–5
  project switching. `chordFromEvent` normalizes shifted punctuation.
- **Surfaces restyled** per frames: PM chat (bubbles/proposal card, panel
  400px), Changes+commit card (single-key s/u/d/a, inline discard confirm —
  ui.discardTarget deleted), PR/checks (failed-first, accent tab underline),
  line comments (lc-* classes; .diff-sel-* kept for CardDetail), board
  (blocked-by meta, dispatch/done confirm overlays, 4a/4b card states,
  j/k/h/l + ⇧ moves), composer ⌥ options unfold, delete confirm itemizes +
  `a` archives, settings 170px nav + provenance `shared` tags + ⇧⌘S share,
  AgentsList (7d) in App settings + onboarding step two.
- **Logo**: fC mark inline SVG in the rail; full Tauri icon set generated
  from `assets/logo/fartcode-icon.svg` via `scripts/gen-icons.sh` (headless
  Chrome + embedded JetBrains Mono; rerun after mark changes).
- **CSS**: per-surface files under `src/styles/` (`taskview/changes/pr/
  comments/modals/settings` + board/project-chat), all `@import`ed from
  styles.css; ~700 lines of dead emdash-era rules deleted; xterm theme now
  reads `--xterm-*` tokens (bg #101012, emerald selection wash).
- **Known gaps (data, not design)**: no numeric task ids (frames' `#392` →
  name/uuid8), no install progress events (installing rows show no %),
  `HostDependencyDto.latest` always null (update ⌄ hidden), no branch-prefix
  command (composer shows `auto · fartCode…`), create_task takes no
  issue-link/provider params (composer issue row omitted, agent row static),
  tmux session name not itemized in the delete confirm, no merge-conflict /
  queue-ordinal / stop-attribution state on tasks (frame 4a's "conflict with
  main", "queued · 2nd of 3", "stopped by you" degrade to what the model
  holds), no would-be branch preview on a first dispatch (confirm footer
  omits the branch until the task exists).
- **Deviations held OPEN for design review (2026-08-09 — do not "fix"
  silently)**: the flyout's Recent group (v1 spec deletes it; kept so ad-hoc
  tasks stay reachable outside ⌘K — user is taking it to design); rail `+`
  = Add project not New task (same review); rail/flyout top padding 28/32px
  clears the macOS traffic lights (platform, not spec); settings renders as
  a floating card over a scrim, not the frame's full-window rail takeover;
  PM file mentions are styled spans, not links (no file surface until E5).

## Styling rules (left-nav redesign, binding for new UI work, 2026-08-08)

Superseded reference: `DESIGN.md` now carries the binding system (v2). The
v1 rules below still hold where they don't conflict.

The app follows `design_handoff_left_nav/` (README.md is the spec; frames
in `fartCode App.dc.html`). When adding/restyling UI:

- Tokens live ONLY as CSS vars in `styles.css` `:root` (`--rail-bg`,
  `--flyout-bg`, `--overlay`, `--hairline`, `--hover-bg`, `--focus-bg`,
  `--text-card`, `--meta`, `--fc-bad`, …). Never hardcode hex in components;
  never introduce a second styling system (no inline-style objects, no CSS-in-JS,
  no utility framework).
- Meaningful colour, and only these: `oklch(.78 .15 155)` = selection/additions
  (the accent); `oklch(.8 .13 80)` = an agent is working (filled) or needs you
  (hollow 1.5px ring); `--fc-bad #c96b6b` = a run ended badly; `--info #7c8fd0`
  = a link out and NOTHING else. `--meta #5f5f66` is the legibility floor —
  nothing informative goes dimmer.
- Cards/rows have no box at rest: hover paints `--hover-bg`, selection/focus is
  `--focus-bg` + a 2px accent left rail. No borders/backgrounds on idle rows.
- System sans for human text, `var(--font-mono)` for machine text (paths,
  chords, IDs, elapsed, counts). Uppercase group labels carry `letter-spacing: .14em`.
- Icons are typographic glyphs (`+`, `⌘`, `‹`, `>_`, `›`) — do NOT add an icon set.
- Motion: only the running-dot pulse (`fc-pulse` 1.8s) and the transcript caret.
  No entrance animations, no transitions on cards/columns.
- The flyout shows IN-FLIGHT work only; the board owns the rest. Do not re-add
  task trees/recents/archive lists to the nav — ⌘K is the jump surface.
- Every action needs a key first, and its button labelled with the key.

## Left-nav redesign: rail + flyout (design_handoff_left_nav, 2026-08-08)

- `components/Sidebar.tsx` is gone; `components/Nav.tsx` renders a 56px
  `LeftRail` (project letter tiles, worst-of agent dot, + new task, ⌘
  settings) plus a 244px `ProjectFlyout` fed IN-FLIGHT tasks only
  (in_progress = Running, review = Needs you). Every other task is
  reachable via ⌘K FTS — pinned/recent/archive tree sections were deleted
  per the design; pin data still drives `visibleTaskOrder` (E2-10).
- `ui.sidebarVisible` now means "flyout open"; ⌘B and ⌘\\ both toggle it
  (command `toggle-sidebar`, relabeled "Toggle project flyout").
- Design tokens live as CSS vars in styles.css `:root` (`--rail-bg`,
  `--flyout-bg`, `--hairline`, `--meta`, `--fc-bad`, …); accent is now
  oklch(.78 .15 155) with DARK `--accent-contrast`, links are `--info`
  #7c8fd0, agent-working amber oklch(.8 .13 80). Board cards are boxless
  (hover bg, selected = accent left-rail); chip row renders as mono meta.
- Skipped from the handoff (no backend surface yet): sessions view/history,
  composer overlay with `>` session switch, ⌘1–5 project switching, 1s
  elapsed tick (flyout uses a 30s tick — display is minute-coarse).

## Create-task dialog: workspace + branch pickers (#59, 2026-08-08)
- Sidebar "+" and ⌘N now open `CreateTaskDialog` (Modals.tsx, driven by
  `ui.createTaskTarget`) instead of instant-creating: workspace select
  (`new-worktree` default / `project-root`) + existing-branch picker fed by
  the new `list_project_branches` command (`BranchRef` now derives Serialize).
- `create_task` gained optional `workspace`/`branch` params; the mapping
  lives in `create_task_params` (now `pub` for tests — same pattern as
  `create_task_from_comment_core`). `project-root` ⇒ `GitSetup::None` +
  `WorkspaceTarget::ProjectRoot` (never touches the live checkout — the
  dogfood mode: agent edits hit `make dev` hot reload immediately).
  Existing branch ⇒ `GitSetup::UseBranch` in a new worktree (fetch + track).
  Comment/dispatch callers pass `None`/`None` — behavior unchanged.
- Core provision paths for both were already tested
  (tasks_operations_integration.rs); the new mapping is covered by
  fartcode-app/tests/create_task_params.rs.

## E4 PR section, PR sync, agent comment tool (#47/#49/#51, 2026-08-07)

- **GitHub client** lives in `fartcode-core/src/github` (token.rs keyring +
  `gh auth token` import; client.rs reqwest REST; models.rs DTOs). Secrets only
  in the OS keyring — never SQLite/logs. Parsers are unit-tested against
  recorded fixtures (`client::fixtures`). Rate-limit aware: 401→GithubAuth,
  403/429+remaining:0→GithubRateLimited(reset_at).
- **PR sync cache** (`pull_requests`, migration 0005): one row per PR URL,
  scalar query columns + full `PrDto` in a versioned-JSON `data` column
  (ADR-0036 — JSON sub-collections, not four normalized sub-tables). Idempotent
  upsert = deserialize-and-compare → skip write+event when byte-identical.
- **Scheduler** in `fartcode-git/src/pr_sync.rs`: periodic `run_scheduler`
  (base 60s, exp backoff on failures capped 1h, jitter), rebuilds targets from
  DB each cycle (restart-safe), `IN_FLIGHT` set dedupes concurrent syncs.
  Cursors in `kv` (`pr_sync:last:*` / `pr_sync:failures:*`). Rate-limit ends the
  cycle early (account-global). The PR tab reads the cache (instant/offline) and
  kicks a background sync; scheduler keeps it warm.
- **Commit-card PR-open guard** is now `CachedPrLookup` (reads the sync cache —
  local, offline-safe) instead of `StubPrLookup`. `PrLookup::pr_url` gained a
  `remote` param.
- **Agent comment tool** (#51): `LineCommentStore::add_agent_comment` validates
  against the task's materialized worktree (path containment, file exists,
  in-range anchor) with typed errors, attributes `created_by = agent:<provider>`.
  Exposed as `agent_add_line_comment`. **Autonomous agent invocation (MCP tool
  registration) is deferred** — no MCP custom-tool infra exists yet; see
  ADR-0035.
- Gotchas: migration count tests assert 6 now (0000–0005). `DOMAIN_TABLES`
  includes `pull_requests` + `issues`. Frontend PR tab is `store/pr.ts` +
  `PullRequestPanel.tsx`; agent comments show a `⚡ <provider>` chip via
  `commentAuthor()`.

## Wrong tab on new task — three root causes fixed (2026-08-07)

The "TTY/Setup script tab on every new task" bug was three stacked defects,
found by reading the real app DB (`~/Library/Application Support/fartCode`):
1. **Auto-run flag ignored:** `run_auto_lifecycle_scripts` never consulted
   `auto_run_enabled` — a configured `scripts.setup` (ade project: `omp`)
   spawned on EVERY task creation with the flag defaulting off. Now gated;
   regression test in tests/task_creation_agent_launch.rs.
2. **Silent failures:** fartcode-app had NO tracing subscriber — every
   best-effort launch error was dropped. `run()` now installs an
   EnvFilter (default `info`, override RUST_LOG) and agent-launch failure
   logs at `warn`.
3. **PATH fragility:** GUI/`make dev` launches can inherit a PATH without
   `~/.local/bin` (where claude lives). `find_on_path` now falls back to
   common user bin dirs (`.local/bin`, `.bun/bin`, `.cargo/bin`,
   homebrew) AFTER the real PATH — mirrors the reference
   remote-shell-profile PATH inclusion.


## Unified top chrome + one agent terminal per task (2026-08-07, ADR-0033)

- The header grid area now ALWAYS renders: `ProjectHeader` (project scope)
  or the new `TaskHeader` (task scope — project/task breadcrumb + script
  launchers + Changes toggle). TaskView's tab bars are pure tab switching;
  `.changes-toggle`/`.tab-bar-trailing`/`.tab-bar-actions` CSS deleted.
- **One agent terminal per task:** `terminal_open_agent` reattaches a live
  agent entry (`TerminalManager::find_running_agent`, lifecycle-dedupe
  pattern) before provider resolution. Frontend: tabs-store `addTab`
  focuses an existing same-id tab; `ensureTabs` surfaces uncovered live
  agent terminals as tabs (dispatch spawns before navigation, so the task
  view must show the hand-off). Switching agents = close the agent tab.
- **No tab bar unless there's something to switch:** `TaskView` renders the
  left pane's `TabBar` only when the task has 2+ tabs or a split. One agent
  terminal (the normal case) now sits directly under the header — the lone
  "TTY claude" chip was the "why is there a tab for the task?" report.
  Verified live on the running app: 1 tab → no bar, ⌘T → bar with both
  chips, close → bar gone.
- Integration test: fartcode-app/tests/agent_terminals_integration.rs.
- **Add Task (left nav) launches the default agent:** `create_task` calls
  `launch_default_agent` (best-effort, same provider resolution as
  dispatch: DEFAULT_AGENT setting → registry binary on PATH). With the
  agent installed, a fresh task opens straight on the agent tab. The
  frontend NEVER auto-spawns a plain shell on task open anymore — the old
  `ensureTabs` terminal fallback is deleted; an empty pane shows ⌘T/⌘D
  summon hints. Test: tests/task_creation_agent_launch.rs.
- **Gotcha (bit in practice):** a "still see the TTY tab" report after
  these changes = the running app is a STALE process. The Rust
  `create_task` launch needs a rebuild+restart (`make dev` / relaunch),
  and store-level frontend changes need a webview reload, not just HMR.
  Check the running pid's start time vs the binary mtime before
  re-diagnosing.


## Issue board design pass (2026-08-07)

- BoardView + CardDetail + board.css rebuilt as ONE ruled surface: a
  hairline-framed plate (`--background-1`) with five lanes divided by 1px
  `--border` rules, shared 32px head row + mono counts; cards are rows
  (title + canonical `.status-dot` + mono chips). Replaces the old
  five-floating-boxes layout. Narrow windows scroll the frame at a 750px
  floor (heads and lanes share `min-width` so they stay registered).
- Cards: linked-task dot uses the CANONICAL `.status-dot` mapping (the
  old board.css had wrong hues: done=green, review=blue — both violate
  the Dots-Are-Data rule). Provider chip is mono passive; gh provenance
  chip opens externalRef via `plugin:shell|open`; blocked chip keeps
  amber + hover popover; acceptance tally "Nac" on the title row.
- CardDetail is now an inspector: lane header with status dot (task
  status wins over lane), agent row with the ONE emerald key —
  Dispatch (backend resolves provider fallback) or Open task when
  linked — meta grid (Source/PRD/Task/Created), empty-state rows for
  acceptance/blockers, hover-only destructive remove keys, sticky
  footer delete confirm. Sheet widens to 420px via
  `.changes-panel.detail-open`.
- Toolbar gained "Add issue" (creates in Backlog, opens its detail) —
  new frontend call to `issue_create`; board empty state teaches the
  GitHub-sync key. All verified in the mocked-backend browser smoke
  (drag/move, blocked-dispatch confirm modal, dispatch → agent write →
  task navigation, gh chip URL open, dirty-save, contrast ≥5.2:1).

## Repo renamed ade → fartCode (2026-08-07)

- User rename, everywhere: 12 crates `ade-*` → `fartcode-*` (dirs + Cargo
  names + `fartcode_*` identifiers), lib crate `fartcode_app_lib`, runtime
  bin `fartcode_acp_runtime`, event channel `fartcode:event` (JS types
  `FartcodeEvent`/`onFartcodeEvent`), env contract `FARTCODE_*`
  (incl. `FARTCODE_PORT`, `fartcode_port`), config file `.fartCode.json`,
  Tauri productName `fartCode` + identifier `dev.fartcode.app`, branch
  prefix setting, all docs/decisions. Product branding is **fartCode**;
  crate/identifier spelling is lowercase `fartcode`.
- GitHub repo `jknack0/ade` → `jknack0/fartCode` (renamed; old URL
  redirects). Full gate green post-rename (fmt/clippy/test + tsc/eslint).

## E17-03 dispatch engine landed (2026-08-06, 5ecacf7) — E17 epic COMPLETE

- **Sheet layout (886bb86, user pick):** at project scope the right surface
  is ONE sheet — Changes on top, PM chat docked at the bottom (flex 42%);
  card click swaps the whole sheet to CardDetail. ⌘⇧2 shows chat AND opens
  the sheet (setChangesOpen(true) in the command); the GitHub icon toggles
  the sheet. Chat/detail mount inside ChangesSidebar, not ProjectView.

- `issue_dispatch` (fartcode-app/src/dispatch.rs): reattach if linked task lives;
  else provider = issue.provider ?? defaultAgent setting, prompt packet
  (`build_dispatch_prompt` in issues module), create_with_provision with
  `linked_issue {provider:"local", identifier:issue_id}` (NO struct change —
  the external-tracker shape absorbs the local variant), link + move.
- **Auto-flip hooks:** terminals.rs pump (agent PTY exit) and
  acp_events.rs transcript_changed (turn settles Done, once-per-turn edge
  detection via flipped_turns map). Both reach App state via
  `app.try_state::<Arc<App>>()` (needs `use tauri::Manager`). Flip = only
  in_progress → in_review.
- Frontend: in_progress drop → dispatch (unlinked) or move+focus (linked);
  agent terminal gets the packet bracket-pasted (Modals.tsx flow).
- AgentStart event is still DEAD (no consumer) — dispatch skips it; the
  frontend launches the terminal explicitly.

## E17-02 + E17-04 landed (2026-08-06)

- **Dogfood fixes (6532b9b):** AcpRuntime::resolve_cwd hard-errored on
  project-scoped conversations ("no workspace yet") — now resolves to the
  project root (regression tests in acp_runtime.rs). Project view header:
  project name + GitHub icon (`project_github_url` command — base remote
  normalized scp/ssh/https → https; non-GitHub hides the icon) + chat
  toggle; PM panel has a minimize button (⌘⇧2 toggles back).

- **#56 board UI** (f47b3e6): 5-lane board with native HTML5 DnD →
  `issue_move` (midpoint drop index, within-lane reorder correction),
  blocked→In-Progress confirm modal, provider/linked-task badges, blocked
  hover popover, CardDetail in the project view's right region (edits via
  `issue_update`, edge add/remove, two-click delete). Card detail takes the
  right region over the PM chat via `ui.boardDetailIssueId`.
- **#58 PM chat** (dad40b5): `fartcode_core::issue_proposal` (parse — never
  panics; apply — all-or-nothing with compensating rollback) +
  `issue_parse_proposal`/`issue_apply_proposal` commands; frontend
  `ProposalCard` in the transcript (rename rewrites blockedBy refs; parse
  failure renders raw text); `PM_PROMPT` as hiddenContext on PM sends.
- **Seams commit 2e00b8e** (pre-landed): project-scoped conversations
  (store scope lift + `get_or_create_project_conversation`), issue command
  wrappers/events, `ProjectView` shell, owner-key conversation store
  (`project:<id>` keys), `toggle-project-chat` ⌘⇧2.
- **Mock-recipe traps hit:** Tauri listen callbacks receive
  `{event, payload}` (emit `payload` or listeners get undefined); mock
  eventHandlers must be ARRAYS fanned out (last-writer-wins silently
  un-wires earlier subscribers); programmatic `blur()` needs `focus()` first
  or React onBlur never fires.
- Remaining: #57 dispatch engine (needs both, now unblocked).

## E17 project board & PM chat — design locked (2026-08-06)

- Re-grilled the §13 project-chat design; it was **stale** (predated the #39
  terminal-only pivot and the E2-11 ACP landing). Full re-design recorded in
  ARCHITECTURE.md §13 (rewritten) + `decisions/0032-project-board-pm-chat.md`.
- Locked: local-first `issues`/`issue_dependencies` tables (fartCode IS the
  tracker; E7/E8 become sync adapters later); 5 lanes with drag-into-
  In-Progress spawning task+agent; board never kills (re-drag reattaches);
  blocked-by derived at read time + cycle rejection + confirm-on-dispatch;
  auto-flip to In Review on ACP turn-complete / PTY exit; chat writes via
  fenced `fartCode-proposal` block → approval card (no MCP until E10 era); PRDs =
  `docs/prds/*.md` in the repo; dispatch prompt packet by reference.
- Tickets: epic #54; #55 (E17-01 issues module) → #56 board UI / #58 PM chat
  panel → #57 dispatch engine.

## E1-06 lifecycle scripts wired into the app (2026-08-06)

- **The E1-06 runner was unwired**: settings UI + core `LifecycleScriptService`
  existed, but nothing in fartcode-app ever ran a script — "set a script, create a
  task, it just opened the terminal". Now lifecycle scripts are REAL task
  terminals: `terminal_open_lifecycle(task_id, script_type)` spawns
  `sh -c '<script>'` (shellSetup prepended) in the worktree with the FARTCODE_*
  env contract (port seed = worktree path), via TerminalManager so output
  streams to the tab like any shell.
- **Retention:** lifecycle entries are RETAINED after exit (pump sets
  `Entry.exited` and skips the map removal only for lifecycle terminals) —
  the finished run's tab reattaches and replays the tail (64 KiB). Plain
  shells/agents keep drop-on-exit. Dedupe: `find_running_lifecycle(task,
  type)` — a rerun while one is in flight reattaches.
- **Auto-run:** `create_task` + `create_task_from_comment_core` call
  `run_auto_lifecycle_scripts` (best-effort) when
  `autoRunSetupScriptOnTaskCreation`/`autoRunRunScriptOnTaskCreation` +
  a non-blank script are set; the task view surfaces backend lifecycle
  terminals as `lifecycle-script` tabs on open (ensureTabs discovery from
  `terminal_list_for_task` kind/scriptType fields). Dead lifecycle tabs in
  persisted view-state are DROPPED on restore (never respawn as a shell).
- **UI:** TabKind `lifecycle-script` (glyph SCR, TerminalView), titles
  "Setup script"/"Run script"/"Teardown script" (`scriptTabTitle` in
  tab-registry). Per-configured-script `Run <type>` keys live in the
  task-scope header row (`TaskHeader`, ADR-0033 — moved there from the
  old tab-bar trailing slot), fetched via getProjectSettings per project
  open. ⌘-free.
- **Testing:** `TerminalManager` is now `TerminalManager<R: Runtime = Wry>`
  + `tauri = { features = ["test"] }` in fartcode-app — integration tests drive
  the REAL PTY layer via `tauri::test::mock_app()` (retain/dedupe/kind,
  plain-shell drop, tail survival). Pure fns (`lifecycle_script_text`,
  `auto_run_enabled`) unit-tested in commands/lifecycle.rs. Browser smoke
  (mocked backend): button render + click-through, auto-run discovery
  (tab without spawn), dead-tab drop, double-click focus dedupe.

## Current state (2026-08-06, E2-13 task startup command)

- **Per-project `taskStartupCommand` (#52) shipped.** Project settings gain a
  BASE (non-shareable, DB-only) `taskStartupCommand` — `share_with_team`
  never writes it to `.fartCode.json`. `terminal_open` now does ONE effective
  settings read (tmux flag + startup command), and when the command is set
  spawns `sh -c '<cmd>'` INSTEAD of `$SHELL` (replace-the-shell semantics —
  terminal exits when the command exits, like agent terminals). Both paths
  covered: plain PTY (program+args already flowed) and tmux durability
  (new `build_terminal_session_command_args` in `fartcode-core::pty::tmux` —
  args were previously documented as not passed into sessions; the plain
  `build_terminal_session_command` is unchanged). Pure decision fn
  `terminal_program(&ProjectSettings, shell)` in
  `fartcode-app/src/commands/terminals.rs` (trim, blank→shell). UI: "Task startup
  command" input in ProjectSettings.tsx (placeholder `e.g. omp`), DTO field
  `taskStartupCommand`. Tests: terminal_program unit tests, tmux args
  builder round-trip through real sh (hostile quotes + $HOME), settings
  round-trip incl. not-shareable assert, and a real PTY smoke in
  fartcode-terminal (spawn `sh -c` in task cwd — macOS /private realpath trap
  on cwd compare, canonicalize). Browser-smoke verified save→reopen
  persistence. ⌘⇧O `terminal_open_agent('omp')` unchanged — explicit agent
  tab composes with the default.
- Next: **#47** E4-07 PR section (L, GitHub client) — last E4 frontier
  with #49(⇐47), #51(⇐50).

## Project-level pull (2026-08-06, left nav)

- **Sidebar project rows carry a pull action** — `project_git_pull(project_id)`
  command resolves `app.projects.get(id)` → `fartcode_git::remote::pull` (ff-only,
  same contract as the E4-08 footer) at the project ROOT checkout. Motivation:
  after a worktree branch lands on origin's default branch, the project
  checkout (often the branch the app itself runs from) had no in-app way to
  catch up. UI: hover-revealed `IconPull` button on `.project-row` (reuses
  `.add-task-btn` styling; `:disabled` = in-flight pulse), errors inline under
  the row via `.project-pull-error` (no toast system). Verified via mocked
  Tauri browser smoke (success / non-ff error / retry-clears).

## Current state (2026-08-06, E4-10 line comments)

- **E4-10 Line comments (#50) shipped — ARCHITECTURE §14 end-to-end.**
  Migration 0001_line_comments (journal when=1800000000001 + sql_for_tag
  arm; ALTERs: source_side/line_end/linked_task_id/resolved/resolved_at/
  created_by + tasks.source_comment_id). Migration-count tests
  (db_integration + migrations.rs) hardcode the journal length — bump
  them with every new migration. Domain: `fartcode_core::line_comments`
  (LineCommentStore CRUD + link_task both-directions in one tx +
  build_comment_prompt pinned EXACTLY to the §14 template; guard
  failures degrade, never fail state reads). Events CommentCreated/
  CommentResolved → `comment:created`/`comment:resolved` envelopes.
  Commands: add_line_comment (takes ONE `request` struct — clippy
  too_many_arguments forced it; frontend wraps `{request: args}`),
  list/resolve/delete_line_comment, create_task_from_comment (core split
  out as `create_task_from_comment_core(&App, ...)` for tests; fartcode-app
  lib now exposes `pub mod app; pub mod commands;`). create_task's param
  building extracted to `create_task_params` (shared with the comment
  flow, which layers an InitialConversationConfig whose initial_prompt =
  §14 template → conversations.config carries it raw, NOT
  versioned-enveloped). Worktree pool comes from app-level
  `localProject.defaultWorktreeDirectory` — per-project
  worktree_directory is NOT consulted by worktree_pool_path.
  Frontend: store/line-comments.ts (byTask, `__lineCommentsStore` seam,
  wireLineCommentEvents in App.tsx); DiffSelectionPopover FAB renamed
  "+ Comment", actions now Add Note / Create Task ⚡ / Send to agent —
  both comment paths go THROUGH the store (markLinked needs the row in
  byTask); QuickTaskDialog (ui.quickTaskTarget) prefills name/provider,
  calls create_task_from_comment then terminal_open_agent + bracketed-
  paste of the prompt; DiffView comment gutters per side (before→a,
  after→b, unified→after only) in Compartments reconfigured by a
  comments effect — markers survive rebuilds via markerMountsRef;
  CommentThread panel (resolve ✓ manual per §14, linked-task badge reads
  live status from sidebar tasksByProject, click → switchToTask).
  Browser-smoke lessons: CM6 ignores re-selecting the SAME range
  (collapse elsewhere first); syntax highlighting splits text nodes
  (select whole .cm-line via TreeWalker); `.diff-sel-actions
  button:nth-child(3)` counts the destination span — use `$$(...
  button)[i]`.
- Next: **#47** E4-07 PR section (L, GitHub client) — last E4 frontier
  with #49(⇐47), #51(⇐50 done now).

## Current state (2026-08-06, E4-08 footer git actions)

- **E4-08 Footer git actions (#48) shipped:** GitFooter under the commit
  card in the Changes sidebar — branch label + ↑ahead/↓behind badges +
  Fetch / Pull / Push / Publish, and an inline add-remote mini-form when
  `remotes.length === 0`. Backend: new `fartcode_git::remote` module —
  `fetch` (-q), `pull` **--ff-only** (deliberate reference deviation per
  ticket; diverged history surfaces git's stderr, never a hidden merge),
  `publish` (push -u, refuses when upstream already set — that's
  commit.rs::push's path), `add_remote` (name charset + dup + empty-url
  validation). `CommitState` extended with upstream/ahead/behind/remotes
  — ONE DTO now feeds both card and footer (git_commit_state is the
  single repo-state read). Commands: git_fetch/git_pull/git_publish/
  git_add_remote. Frontend: store actions refetch state after every
  success so the footer flips immediately (publish → push/pull
  affordances, acceptance); errors inline (.git-footer-error, role=alert,
  cleared on next success — repo has no toast system). Disabled matrix:
  fetch needs hasRemote; pull needs upstream && behind>0; push needs
  remote+branch+(upstream||published); Publish visible only when
  branch+remote && !upstream. Browser smoke: 4 scenarios by workspace id
  (synced ↑2↓1, no-remote+add-form, unpublished publish-flip, diverged
  pull error). Rust tests: bare-remote clone fixture for real
  ahead/behind + ff-pull + diverged-pull + rebase recovery.
- Next: **#47** E4-07 PR section (needs the #49 sync engine's storage —
  check its body) or **#50** E4-10 line comments; #49(⇐#46 done) and #51.

## Current state (2026-08-05, E4-06 commit card)

- **E4-06 Commit card (#46) shipped:** bottom-of-Changes-sidebar card —
  message input + Commit / Commit & Push / Commit & Create PR.
  Backend: new `fartcode_git::commit` module (free fns like stage.rs — NOT
  GitOps trait methods; commit/push are UI mutations, stage.rs precedent).
  `commit()` = `commit -m` + rev-parse HEAD, empty msg rejected pre-spawn;
  `push()` = upstream-configured → plain `git push`, else `push -u
  <remote> <branch>` (reference publishBranch), returns combined
  stdout+stderr (PR URLs live on stderr); `state()` = CommitState DTO
  (branch/remote/hasRemote/published/prOpen/canCreatePr) with pushRemote
  resolved workspace→task→project settings `effective_push_remote()`;
  `create_pr()` = Phase 0 stub-level integration: guard → push-if-
  unpublished → GitHub compare URL (`/compare/<branch>?expand=1`, ssh +
  https remotes) opened in the browser via @tauri-apps/plugin-shell
  (JS dep added; Rust plugin + `shell:allow-open` capability already
  registered). **PrLookup trait + StubPrLookup** = the PR-open guard seam
  (always None until E4-07/E8); guard failures degrade to "proceed",
  never fail the state read. Commands: git_commit_state/git_commit/
  git_push/git_create_pr.
  Frontend: `store/commit-state.ts` (per-workspace, `__commitStateStore`
  seam; refetch rides the changes.ts 150ms event debounce — ONE
  subscription, timer body refreshes both stores), `CommitCard.tsx` in
  ChangesSidebar (rendered only when workspaceId && snapshot). Disabled
  matrix: empty msg | nothing staged | detached HEAD disable Commit;
  push additionally needs hasRemote; PR-open → Create PR button replaced
  by "PR already open — push instead" note. Errors inline
  (.commit-error), message kept for retry, cleared on success.
  Deliberate reference deviations: no autoStage (card commits exactly
  the staged set; Stage all lives in panel header), no description
  field, explicit buttons instead of split button + remembered action.
  Browser smoke (mocked backend, per-workspace state scenarios by id):
  all 4 matrix rows + commit/push happy path + error surfacing verified.
  13 fartcode-git tests (incl. bare-remote upstream fixture, mocked PrLookup
  guard, offline-safe create_pr).
- Next: **#48** E4-08 footer git actions (fetch/pull/push/publish/
  add-remote — reuses commit.rs `push()` + `state()` patterns) or **#50**
  E4-10 line comments; then #47(⇐#46) → #49 PR chain.

## Current state (2026-08-05, selection → agent)

- **Terminal reattach on frontend reload (fd5956c):** vite HMR/webview
  reloads used to RESPAWN every terminal tab (fresh shell!) while the
  live agent PTY stayed orphaned in the backend — "my sessions don't
  show". ensureTabs reconcile now reattaches persisted tabs whose id is
  live in `terminal_list_for_task` (title + agent preserved), respawning
  only dead ids (app-restart path unchanged). Scrollback: TerminalManager
  keeps a 64KB output tail per entry, replayed via `terminal_tail` into a
  fresh xterm (subscribe-first buffering so the fetch race can't lose
  chunks). tmux shells benefit equally (no fresh attach ⇒ no tmux redraw
  ⇒ tail is the only content source, no duplication). Mock lesson again:
  EVERYTHING in __MOCK re-seeds on reload — flip cross-reload state via
  localStorage overrides.
- **Selection prompt routes to the LIVE AGENT TERMINAL first (68939da,
  user-directed):** opening a parallel ACP chat "while the work happens
  elsewhere" was wrong. TerminalSpec/Entry now carry `agent: Option<provider>`
  (set by terminal_open_agent; shells are None) and
  `terminal_list_for_task(taskId)` exposes it. Popover submit: agent
  terminal → `terminal_write(id, ESC[200~ + prompt + ESC[201~ + \r)`
  (bracketed paste so multi-line lands as one block) + focus that tab;
  ACP conversation is the FALLBACK (no agent terminal). The popover shows
  the destination on open ("→ omp terminal" / "→ Agent chat"). Smoke:
  both routes verified (write to term-omp with paste markers + no ACP
  call; shells-only → acp prompt + Agent tab). Provider AGENCY for the
  ACP path is still "first ACP-capable registry entry" (claude); the
  `defaultAgent` setting exists but is still unread — if provider choice
  becomes a thing, wire that + a popover picker.
- **Selection → agent WORKS end-to-end with the real adapter (verified
  live):** the "I don't see anything" report was a SLOW, SILENT turn, not
  a hang — tools-first edits (Bash/Read of a 560-line file before any
  text) leave the UI showing just the user card with no strong working
  signal for 20-60s. UX gap to close in the conversation view: an
  unmistakable working indicator (elapsed time + latest tool activity)
  during silent stretches. Postmortem artifacts: `fartcode-app/tests/
  acp_real_adapter_probe.rs` (ignored live probe — start → edit prompt →
  turn settle → file edited; run with --ignored) and a standalone stdio
  driver pattern (/tmp/acp-probe.mjs style: initialize/session/new/
  session/prompt over newline-delimited JSON-RPC). The claude adapter
  auto-approves fs edits without session/request_permission when the
  client declares fs read+write caps; zero CPU on the adapter does NOT
  distinguish hung-vs-fast-completed turns (node is sub-second per turn).
- **ACP adapter resolution (7a0b16e):** `default_adapter_resolver`'s
  `<id>-acp` format names binaries that don't exist in the wild —
  claude's real adapter is `claude-agent-acp` from
  `@agentclientprotocol/claude-agent-acp` (installed globally on drfart's
  machine, v0.65.0). Resolver now has a per-provider table with npm
  install hints in the error; the table's long-term home is the provider
  descriptor's adapter metadata (Phase 2 plugin machinery). Claude spawn
  sets CLAUDE_CODE_EXECUTABLE to the host binary (reference behavior,
  avoids the SDK's ~50MB auto-download). Codex's ACP is a SUBCOMMAND
  (`codex acp`, not a binary) — the path resolver can't express
  command+args yet; known limitation when codex ACP gets exercised.
- **Diff selection → agent prompt (de6c9eb, user-directed reshape of #50's
  popover):** select text in ANY diff editor (split a/b, unified, single-
  doc) → floating "Ask agent" button at selection end → popover textarea
  (Enter sends, ⇧Enter breaks, Esc closes) → `<path> lines X–Y[(baseline)]:`
  + fenced code + prompt → task's ACP conversation via shared
  `lib/acp-conversation.ts` (`ensureAcpConversation` find-or-create +
  `focusConversationTab`, extracted from the ⌘⇧A command) → conversation
  tab focused. Selection lives in diffs store (`selectionByTab`, capped
  4K chars). #50 (line comments) now inherits this popover — its
  remaining scope is Add Note / Create Task actions + persistence +
  comment-task linking, not popover mechanics. Mock lesson (recurring):
  ALWAYS close+kill the browser before re-registering an init script —
  duplicate init scripts share one scope and the second dies on
  const-redeclaration; mock is now IIFE-wrapped for idempotent
  registration. Also: no backticks in `git commit -m` double-quoted
  strings (shell ate a code span + 'syntax error at end of input').

## Current state (2026-08-05, E4-05)

- **#45 E4-05 Inline editing of unstaged diffs (c42dd17):** worktree side
  of unstaged diffs is editable (split b-editor, unified view, Added
  single-doc); staged + baselines read-only. ⌘S bound via CM keymap IN
  the editable editor (no global-registry entry — E5 keeps its own path).
  New `fartcode-core::files::write_file` (lexical + canonical containment;
  `Error::PathEscape`) behind `write_workspace_file` (commands/files.rs;
  `workspace_path` in commands/git.rs is now pub(crate)). Refresh rules:
  content-identical payload (save echo) skips rebuild (cursor/scroll/undo
  survive) — BUT the skip requires the view KIND to match the requested
  mode (mode flip bug found in smoke); external change rebuilds with
  scroll+selection preserved; refresh while dirty deferred (edit wins).
  Dirty dot in TabBar + header badge; saveError chip. Live CM handles in
  `lib/diff-views.ts` map (non-serializable, never in zustand);
  `window.__tabsStore` seam added to store/tabs.ts (HMR resets zustand
  stores on module reload — smoke calls into a re-created store silently
  no-op until ensureTabs rehydrates; wait ~2s after HMR). E4 is 5/11;
  next: **#48 E4-08** footer git actions or **#46 E4-06** commit card.

## Current state (2026-08-05, dogfood fix)

- **create_task never provisioned (pre-existing gap, 393abee):** the
  command used bare `DbTaskStore::create` — E2-04's `create_with_provision`
  was dead code with zero callers, so tasks got config-less `worktree` rows
  with `path=NULL` and terminals silently fell back to the project path
  (terminals.rs COALESCE). E4-03's `git_status` exposed it as "workspace
  has no local path". Fix: `create_task` routes through
  `TaskCreationService` (now in App); branch = `fartCode/<slug>-<suffix>` from
  the typed `registry::PROJECT` group (**settings group key is "project"
  SINGULAR** — `get_json("projects")` throws InvalidSettingKey; typed
  `.get()` is a DbSettingsStore inherent method, the trait only has
  get_json). `provision_task` command heals legacy rows; provision's
  config-less worktree fallback mints + persists a default intent
  (regression test `provision_heals_legacy_configless_worktree_row`).
  Changes panel: not-ready state + Provision button (error match needs
  `.includes()` — frontend errors are "Error: <msg>" prefixed). Changes
  toggle moved to TabBar trailing slot (upper right; right pane's bar
  when split). Flaky: `fartcode-runtime worker_integration
  renderer_env_discarded_and_server_env_reaches_adapter` failed once,
  passed on rerun — timing-sensitive, watch it.

## Current state (2026-08-05, latest+++++)

- **#43 E4-03 + #44 E4-04 Changes sidebar + diff renderer (one commit):**
  Right-side Changes panel (`.changes-panel`, `.shell` grid now
  `264px 1fr auto` with explicit `grid-column: 3`) toggled by sidebar-header
  branch icon or ⌘⇧1 (`toggle-changes` command; ui store `changesOpen` —
  NOT persisted, matches resourceOpen). `store/changes.ts`: snapshot per
  workspace, actions refetch immediately post-invoke, `wireChangesEvents`
  = 150 ms coalesced refetch on git:changed/files:changed for TRACKED
  workspaces only — no polling (smoke-verified flat call count). Discard
  confirm modal via ui `discardTarget` (untracked warns "deletes from
  disk"). `fartcode-git::stage` — stage/stage_all/unstage (unborn-HEAD →
  `git rm --cached -r` fallback)/discard (tracked→restore, untracked→fs
  delete, missing→error); commands git_stage/git_stage_all/git_unstage/
  git_discard. Row click → `openDiffTab` (lib/diff-tabs.ts): single =
  preview (one preview per pane, next preview REPLACES it), double =
  persistent; same file re-open activates (no dupe); opening preview's
  file with preview:false flips it persistent. Tab id =
  `diff:<side>:<workspaceId>:<path>` — restored tabs re-parse params from
  the id (no sidecar state); preview-ness lives in store/diffs.ts,
  restored tabs are persistent. `components/DiffView.tsx`:
  @codemirror/merge — MergeView (split) / EditorView+unifiedMergeView
  (unified), oneDark, language-data grammars, read-only; ONLY builds while
  `active` (display:none zero-measure trap), guards: binary / tooLarge /
  Added / Deleted single-doc states with badges. Mode toggle persists
  `view-state:app:diff-mode`. Browser smoke proved: panel rows/glyphs/
  rename orig→new, stage/stage-all/unstage/discard flows, event refresh,
  preview replace + persistence, unified↔split + mode persistence across
  reload, notices, diff content refresh on git:changed, restart restore
  from seeded view-state. Mock lessons: multiple `fartcode:event` listeners
  need handler ARRAYS; viewState must be seeded IN THE MOCK for reload
  tests (mock re-init wipes persisted state); scope assertions to the
  active `.tab-content` (hidden tabs stay mounted). Deps added:
  codemirror, @codemirror/{merge,language,language-data,state,view,
  theme-one-dark}. Next: **#45 E4-05** (inline-edit unstaged diffs ⌘S) or
  **#48 E4-08** (footer git actions) — both unblocked.

## Current state (2026-08-05, latest++++)

- **#42 E4-02 Git status/diff engine (worktree-scoped):** `fartcode-git` grew
  `status.rs` + `diff.rs` (crate doc already claimed status/diff — now
  true). Status: one `git --no-optional-locks status --porcelain=v2 -z
  -uall` (no-optional-locks so status never writes the index → no E4-01
  watcher feedback loop) + staged/unstaged `diff --numstat -z`;
  `StatusSnapshot { staged, unstaged, stagedAdditions/Deletions,
  truncated }` (camelCase serde, returned by commands directly); reference
  split semantics: X column → staged, Y/untracked → unstaged, conflicts
  (`u` records, AA/DD) appear in BOTH lists; renames carry `origPath`;
  untracked additions = capped line count; >10k entries → truncated=true
  w/ empty lists. Diff: NOT hunks — two-sided content payloads (`FileDiff`
  old/new content+size+exists, binary, tooLarge) because @codemirror/merge
  computes hunks from documents (reference getFileAtRef design). Sides:
  staged = HEAD:{origPath|path} ↔ :0:path; unstaged = :0: (fallback :2:
  ours during conflict, then HEAD:) ↔ worktree file. Guards: 512 KiB/side
  cap (size-checked via cat-file -s BEFORE reading — oversize blobs never
  materialize), NUL-in-8KiB binary sniff; guarded payloads keep sizes,
  drop contents. Path inputs validated lexically (no abs, no `..`).
  Commands `git_status(workspaceId)` / `git_file_diff(workspaceId, path,
  side: "staged"|"unstaged", origPath?)` in fartcode-app/src/commands/git.rs.
  22 new tests (fixture repos: conflict both-lists + :2: fallback, rename,
  spaces-in-paths, binary, oversize both paths, traversal rejection;
  synthetic parser/numstat vectors incl. rename `\0` framing). Next:
  **#43 E4-03** (Changes sidebar UI) or **#44 E4-04** (diff renderer) —
  both unblocked now.

## Current state (2026-08-05, latest+++)

- **#41 E4-01 File+git event watcher → live refresh pipeline:** E4 series
  opened (epic #40, children #41–51, milestone "Phase 1", label phase:1).
  New `fartcode-core::fs_watch`: notify-8 `FsWatchService` — one
  RecommendedWatcher, refcounted **canonical** watch roots (worktree +
  shared git common dir when it lives outside the worktree), std-thread
  dispatcher debouncing raw events into 100 ms batches → pure
  `classifier` (reference port: common-dir HEAD/refs/heads/packed-refs →
  conservative fan-out to every workspace sharing that common dir;
  superset deviation: refs/remotes + config fan out too, for ahead/behind
  freshness; per-worktree gitdir HEAD/index routed to the owning
  workspace only; worktree files excl. `.git`; objects/logs = noise) →
  bus: new `FilesChanged { workspace_id, paths (rel, ≤128) }` + existing
  `GitChanged`. `layout.rs` resolves gitdir/commondir by pure fs (no git
  binary; canonicalize everything — FSEvents reports realpaths, /tmp
  symlink trap). Lifecycle in `fartcode-app/src/watchers.rs` (indexer.rs
  pattern): boot backfill (`boot_targets`: non-archived tasks w/ local
  workspace path), TaskProvisioned → `target_for` → register,
  TaskDeleted → unregister; workspaces shared by several tasks
  refcounted. Frontend receives git:changed / files:changed via the
  established `fartcode:event` envelope (ticket's per-name Tauri events
  adjusted to the envelope convention). Service mutexes are parking_lot
  (rs-parking-lot rule; Db's std Mutex contract untouched). 19 fs_watch
  tests incl. real-FSEvents integration: burst→one batch, linked-worktree
  fan-out with index staying scoped, unregister stops events, refcounts,
  DB helper queries. Next: **#42 E4-02** (git status/diff engine).

## Current state (2026-08-05, latest++)

- **#33 E2-11-6 Chat UI — transcript renderer + permission prompts:**
  E2-11 is now 6/6. New `conversation` tab kind (tab id = conversation id;
  ⌘⇧A `open-conversation` creates/focuses with the first ACP-capable
  provider). `ConversationView` + `TranscriptItems` render the reduced
  transcript two-tier: `SettledTurn` = React.memo on (id, items.length,
  outcome.kind) — sound because committed turns are immutable — so
  streaming snapshots re-render only the active turn (verified: settled
  DOM nodes identity-stable). Permission prompts dock at the composer
  (allow*→primary / reject*→danger → `acp_resolve_permission`); transcript
  rows show a blue awaiting glyph on the gated toolCallId. Plan = docked
  strip above composer (session slice, not a transcript item). Composer:
  native textarea (editor scope — no conversation-view scope), Enter
  sends / Shift+Enter breaks, Stop→`acp_cancel`, send-while-working
  queues. States: hero, starting, closed-notice, stop-reason notices
  (max_turn_requests/max_tokens/refusal), send-error banner, conversation-
  deleted. Restore: tabs-store `reconcile` now branches per kind
  (conversation tabs restore as-is; view hydrates via `acp_history` with
  in-flight guard). tauri.ts types tightened to the exact models.rs
  discriminated unions. No Rust changes. ADR-0031. Verified per
  fartCode-frontend-browser-smoke (mock: /tmp/fartCode-mock-33.js pattern): full
  streaming+permission round-trip, task switch+return, cold-restart
  history restore, closed/error states. Next: E2-11 parent #21 can close;
  remaining Phase-2 work per issue list.

## Current state (2026-08-05, latest+)

- **#32 E2-11-5 Commands + conversation-store wiring + provider decision:**
  ACP conversations actually chat. `fartcode-app::acp_runtime::AcpRuntime`
  owns the SessionManager and spawns the adapter binary as a direct child
  per conversation (env server-resolved via keyring `resolve_env` with
  launcher process-env fallback; renderer never supplies env — ADR-0030:
  the E2-11-2 `fartcode-acp-runtime` worker stays DORMANT; the in-app runtime
  won, keeping all E2-11-4 wiring live). Commands: `create_conversation`
  (runtime type decided SERVER-SIDE from capabilities.acp — renderer never
  picks it), `list_conversations` (DTO carries derived `runtime` field,
  no DB column), `acp_start` / `acp_send_prompt` / `acp_cancel` /
  `acp_resolve_permission` / `acp_stop` / `acp_history`. Provider decision
  gate = `resolve_session_path` in exactly 2 places (create + start).
  Teardown: `delete_task` calls `AcpRuntime::stop_task` BEFORE the FK
  cascade. Frontend: `store/conversations.ts` (runtime field + `acp:*`
  subscription + `window.__conversationsStore` browser-test seam), ⌘Enter
  `send-context` command routes only when runtime==='acp' (TUI untouched).
  Boot ACP rehydration NOT wired (PTY stays byte-identical; follow-up with
  #33 chat UI). Tests: 3 E2E (fake adapter e2e, gate non-regression,
  teardown) + browser smoke. Test seam: `FARTCODE_ACP_ADAPTER` env override.
  Next: **#33 E2-11-6** (transcript renderer + permission prompts).

## Current state (2026-08-05, latest)

- **#31 E2-11-4 Transcript reducer + live models:** `fartcode-acp::transcript`
  owns the full port of the reference reducer — pure
  `(ParserState, ReducerInput) → ParserState` fold (`reducer::reduce`),
  stateful `TranscriptParser` (push/settle_turn/begin_replay/end_replay/
  replay), `SessionUpdate → NormalizedEvent` decoder, reference-format id
  synthesis, and serde-camelCase live models (reduced turns w/ message/
  thinking/tool-lifecycle/plan items, config selectors, usage, title,
  agents, plan). `SessionCell` now owns parser + `RawAcpLog` (50k-entry
  in-memory raw-traffic export); raw `Turn.updates` is GONE — history is
  reduced turns, prompt text = synthetic user-message item. Event seams =
  `SessionEvents` trait fired by the cell; `fartcode-app::acp_events::
  TauriAcpEvents` emits `acp:update` / `acp:transcript` (full LiveModels
  snapshot) / `acp:permission_request` keyed by conversationId —
  bypassing the internal bus (terminal:output precedent). ADR-0029.
  Scoped down: no EnrichHook → no subagent/search/mcp/web-fetch event
  kinds; terminal live models stay empty until the Phase-4 `terminal`
  capability. `StartInput` gained `provider_id` + `events` (replaces
  `update_sink`). Fake adapter has a `rich` prompt behavior exercising
  every slice. Tests: 6 reducer goldens + 5 browser-free event/integration
  tests. Next: **#32 E2-11-5** (commands + conversation-store wiring).

## Current state (2026-08-05, later)

- **UI redesign — Signal → "emdash world" (impeccable new-work, seed e3c1a90f):**
  full replacement of `app-frontend`'s visual world. Neutral charcoal chassis
  (`#111111` bg ramp from emdash `.emdark`), emerald primary action
  (`--accent: #00a67b`), blue selection; status hues: in_progress = amber
  `--status-in-progress`, in_review = green `--status-in-review`,
  cancelled/destructive = red. Type: Inter Variable (UI voice) + JetBrains
  Mono Variable (machine voice) via `@fontsource-variable/*`. Old
  `--bg*`/`--amber` Signal tokens are gone — don't reintroduce;
  `styles.css` `:root` is the token source (`--background*`/`--foreground*`/
  `--border*`/`--accent*`/`--status-*`/`--xterm-*`), recorded in DESIGN.md
  + `.impeccable/design.json`. Icons are drawn SVG in `components/icons.tsx`
  (no unicode glyphs). xterm theme in `lib/terminals.ts` syncs with
  `--xterm-*`. Direction contract comment lives in `index.html` body
  (survives build). Supersedes the intermediate INSTRUMENT concept
  (seed 0a35d91b, Barlow fonts) — never landed. Reviewer disposition: ship.

## Current state (2026-08-05)

- **Terminal lifecycle fix (ADR-0028):** reopen now shows every surviving
  tmux terminal automatically, and closing a tab KILLS the session (no more
  detach-survivors accumulating — a real task had grown to slots 0–10).
  Mechanics: `close` runs `kill-session` + frees the slot; `pick_slot` →
  `choose_terminal_slot` reuses the smallest live DETACHED session (never
  double-attaches); window close = `detach_all` (PTYs die, sessions live);
  restore calls new `terminal_surviving` and opens extra tabs for survivors
  beyond the persisted tabs. Real-tmux integration test
  `list_by_prefix_reports_survivors_with_attach_state` pins the listing.

- **#30 E2-11-3 SessionManager + SessionCell:** `fartcode-acp::session` owns the
  runtime (cell = state machine starting→ready→working/cancelling→closed,
  prompt queue with drain-on-settle, permission broker, rev-guarded draft,
  raw update stream per turn; manager = cells keyed by conversationId,
  routes by ACP sessionId, `start` = session/load-resume w/ fallback to
  session/new + `SessionIdStore` persistence, initial-queue dispatch).
  Persistence is a one-method trait — the real `DbConversationStore`
  adapter wires at #32. Provider decision hook =
  `fartcode_core::conversations::resolve_session_path` (ACP needs config type
  AND `capabilities.acp`; else TUI path, E2-06 launcher untouched).
  Deviations from reference in cell module docs (no quiesce timer, no
  background agents — both arrive with the E2-11-4 reducer). ADR-0027.
  Tests: `fartcode-acp/tests/session_manager_integration.rs` (9 tests vs fake
  adapter incl. restart-resume) + decision regression in
  `fartcode-core/tests/conversations_integration.rs`. Next: **#31 E2-11-4**
  (transcript reducer + live models + `acp:*` events).

## Current state (2026-08-04)

- **#36 durable terminals (ADR-0025):** with the project `tmux` setting on,
  E2-12 terminals run under tmux (`{project}:{task}:terminal:{slot}` sessions)
  — app crash/restart leaves the shell alive and the next open REATTACHES
  slot 0. Close-tab = detach; task-delete sweeps the prefix (orphans included).
  tmux binary resolved with Dock-PATH fallback; setting off/binary absent →
  plain shell unchanged. `tmux_by_default` stays false. Agent terminals
  (⌘⇧O) are always plain PTYs — slot durability is for shell terminals.
- **"Signal" UI design system (#38):** full restyle of `app-frontend`. Tokens
  live in `styles.css` `:root` (`--bg0..3`, `--line`, `--text/--muted/--faint`,
  `--amber` reserved for the ONE active signal: selected task row bar, focused
  pane's active tab, primary actions). Type: Space Grotesk = UI voice,
  JetBrains Mono = data voice (tasks, chords, terminals, meters) — bundled via
  `@fontsource/*` (imported in `main.tsx`; no CDN, Tauri stays offline-safe).
  Tab kinds carry a `glyph` in `lib/tab-registry.tsx` (terminal = `$`).
  xterm theme re-tinted in `lib/terminals.ts` (bg `#0b0d10`, cursor amber).
  Old `--navy*` AND #39's signal-box `--board/--ivory/--aspect-*` tokens are
  gone — don't reintroduce.
- **Work tracking is GitHub issues only** (`jknack0/fartCode`) — `tickets-phase0.md`
  was retired 2026-08-04; its Appendix is preserved as `phase0-checklists.md`.
  New work = new issue (`phase:0`/`phase:2` + `size:*` labels, milestone "Phase 0").
- **Terminal-only task view (2026-08-04):** chat surfaces fully removed —
  ⌘T/⌘⇧T open plain terminals; ⌘D splits right with a fresh shell; ⌘⇧O opens
  the OMP agent terminal via new `terminal_open_agent` (provider-registry
  binary resolution through `find_on_path`). Frontend `conversation` tab
  kind, ConversationView, conversations store, palette branch, and backend
  conversation commands/indexing/search are gone; `fartcode_core::conversations`
  stays (PTY launcher + boot rehydration depend on it). Scope precedence is
  now modal > editor > task-view > project-view > app-view > global.
- **Terminal lifecycle (#37) kept under the terminal-only refactor:** xterm
  sessions live outside React keyed by PTY id (`lib/terminals.ts`); PTY
  ownership is in the tab store (only ⌘W's last reference / split collapse /
  task delete kills); terminal tabs persist and respawn a fresh shell on
  restore (scrollback restart survival = future tmux work). Panes ALSO keep
  all tabs mounted (hidden, not unmounted) so tab flips never even detach.
- **Signal-box theme:** dark green-grey diagram board, ivory track lines,
  multi-aspect state colors (proceed/caution/stop/shunt), Libre Franklin +
  IBM Plex Mono (@fontsource). Terminal theme matches --inset/--ivory/
  --aspect-proceed.
- **Phase 0 is fully closed** (2026-08-04). **Phase 2 in progress:** E2-11
  broken into #28–#33; #28 (2827012), #34 (9041aad), #29 (2ca862a) done.
  **#35 E2-12 interactive task terminal done (713dfbd) + terminal-first
  default (5ea481d) + lifecycle fix (#37) + terminal-only refactor.**
  Work-inside-fartCode path for agents like omp. Next E2-11:
  **#30 E2-11-3** (SessionManager + session-id persistence).
- **HEAD (2827012, 2026-08-04):** E2-11-1 — fartcode-acp is a real ACP v1
  client: stdio JSON-RPC transport + client lifecycle (initialize/new/load/
  prompt/cancel/set_mode/set_config_option) + scoped fs handlers +
  permission surfacing. Wire types from `agent-client-protocol-schema`
  v1.6 (ADR-0024); test fixture `fartcode-acp/src/bin/fake_acp_adapter.rs`;
  8 integration tests in `fartcode-acp/tests/protocol_integration.rs`.
- **E14-01 (16b8e8f):** keybinding registry — scope precedence
  modal > editor > task-view > project-view > app-view > global
  (conversation-view scope removed with the chat surfaces),
  user overrides in `view-state:app:keybindings`. E2-10's
  `lib/shortcuts.ts` was superseded and deleted.
- **E2-08 removed the standalone conversation list** — conversations now live
  under tasks (create-task command + sidebar).
- **E2-07 shipped terminal persistence/resume** — boot rehydration orchestration,
  tmux kill, remote hook, dirty-check on worktree open (ADR-0022 for the
  sync-command decision).

## Key decisions (see decisions/ for full ADRs, 0001–0027)

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
  fns return `Result<T, fartcode_core::Error>`, commands map errors to `String` and
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
- **Terminal session lifecycle:** React effect cleanups run on task switches
  and tab flips — never kill/cleanup shared resources there. Interactive
  terminals keep their xterm instance in a module-level registry keyed by PTY
  id; the TAB owns the PTY (tab store kills on close/split-collapse/delete),
  the VIEW only attaches/detaches the DOM node. Also: one PTY drives one
  xterm surface — splitting a terminal spawns a fresh shell. (E2-12 fix #37.)
- **PTY integration tests: never gate readiness on echoed output** — the PTY
  echoes the typed command, so a sentinel inside the command self-matches
  before it runs (tmux_durability flake). Gate on files the shell writes.
- Before touching DB, PTY, SSH, or provider-spawning code, read the matching
  `reference/emdash/agents/risky-areas/*.md` page (reference impl is a clone of
  `generalaction/emdash`, Electron + TS).

## Docs map

- `AGENTS.md` — onboarding + merge gate (fmt, clippy -D warnings, cargo test).
- `ARCHITECTURE.md` — authoritative reference: traits, error type, async
  boundaries, event bus, DB schema. Ticket contradicting it → ticket loses.
- `PRD.md` — product spec + epic inventory.
- GitHub issues — the only work list (`gh issue list -R jknack0/fartCode`).
- `phase0-checklists.md` — cross-cutting Phase 0 process checklists (ex-Appendix).
- `decisions/` — ADRs 0001–0033; record new ones before merge, not after.
