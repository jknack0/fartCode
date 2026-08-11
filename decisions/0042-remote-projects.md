# ADR-0042: Remote projects — `RemoteHost` trait in core, worktrees inside the project

- **Status:** Accepted
- **Date:** 2026-08-11
- **Issue:** #88 (E12-04)
- **Relates to:** ADR-0003 (trait placement), ADR-0039 (worktree pool segment), ARCHITECTURE.md §6.4 / §11

## Context

E12-04 puts a project's repository on an SSH host. The create flow needs
remote filesystem facts (realpath, stat, mkdir) and remote git (`rev-parse`,
`clone`, `worktree add`) — all of which live in `fartcode-ssh`, which depends
on `fartcode-core`. `fartcode-core` is the leaf crate, so it cannot import the
SSH machinery it needs.

Three further questions had no precedent: where remote worktrees live, how a
remote project is identified, and how remote command lines are built.

## Decision

- **`RemoteHost` trait in `fartcode_core::projects::remote`**, implemented by
  `fartcode_ssh::host::SshRemoteHost` — the same split ADR-0003 chose for
  `GitOps`. Five operations (`realpath`, `list_dir`, `stat`, `mkdir_all`,
  `remove_dir_all`, `run`) are enough for the whole flow, and they make the
  domain logic testable against a fake instead of a live host.
- **Remote worktrees live inside the project**:
  `<project>/.fartCode/worktrees/<segment>`, not a sibling pool like local.
  One SSH root, one thing to remove, and `.fartCode/` is already git-excluded.
  The segment hashes the **workspace key** (`ssh:<conn>:<path>`), extending
  #81's "never key a pool on a name" rule to remote.
- **Project identity is (connection, path)**, enforced by migration 0012:
  `idx_projects_path` becomes `UNIQUE(path, COALESCE(ssh_connection_id, ''))`.
  `/srv/repos/app` on two hosts is two projects; the `COALESCE` keeps local
  rows colliding with each other (a plain two-column index would treat every
  NULL connection as distinct and allow duplicate local projects).
- **Every remote command is an argv array** rendered by `remote_command_line`
  through `shell_escape::single_quote`. Nothing is interpolated into a shell
  string anywhere in the remote path.
- **Remote projects skip the local open flow.** `.git/info/exclude` writes and
  worktree re-detection run against the local filesystem; for a remote project
  that is the wrong machine. `RemoteProjectStore` reuses only the shared
  `insert_project_row` + `ensure_repository_workspace` + `project:added` tail.
- **Repository workspace rows record the host**: `type='project-ssh'`,
  `location='remote'`, `ssh_connection_id` set — a rehydrate (E12-06) needs a
  way back to the machine the files are on.
- **Base ref resolution is two commands** (`git remote`, `git symbolic-ref`),
  with the same `computeBaseRef` normalization as local: a plain branch takes
  the remote prefix, a slash-carrying branch stays bare. No `git remote show`
  — same hang risk ADR-0003 rejected.
- **Connections are per command, not pooled.** Each remote command opens and
  drops its own `SshClient`; states, backoff, and rehydrate are E12-06's job,
  and a throwaway lifecycle here would have to be unbuilt to get there.

## Consequences

- `fartcode-core` stays a leaf and gains no SSH dependency; the remote flow is
  unit-testable with an in-memory `RemoteHost`.
- Remote worktrees are inside the repo directory, so a remote `git clean -xdf`
  in the project root can destroy them. Acceptable: they are recreatable, and
  `.fartCode/` is excluded, not ignored-and-cleaned.
- Path containment stays **lexical** (E12-02's caveat): a remote symlink out of
  the pool is still not caught.
- Migration 0012 rewrites a uniqueness constraint that existed since 0000. Old
  DBs keep their rows; only the index changes.
- Each remote command pays a fresh SSH handshake until E12-06 lands pooling.
