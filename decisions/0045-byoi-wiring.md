# ADR-0045: BYOI wiring — the workspace row is the state, the gate is at provision

- **Status:** Accepted
- **Date:** 2026-08-11
- **Issue:** #92 (E12-10)
- **Relates to:** ADR-0043 (connection pool), ADR-0044 (BYOI contract), ADR-0042 (remote workspace rows)

## Context

E12-07 landed the BYOI contract and both runners with nothing calling them.
Wiring raised three questions: what marks a task as BYOI, where the provisioned
machine's identity lives, and how a half-built feature ships without exposing
users to scripts that spend money on infrastructure.

## Decision

- **The workspace row decides.** `workspaces.kind = 'byoi'` is the trigger, the
  same rule E12-04 set for remote projects (`location = 'remote'` routes
  terminals). No call-site flag, no settings lookup to know what a task is.
- **Provisioning writes to that row**: `remoteWorkspaceId` into the versioned
  config, `ssh_connection_id = task:<task id>`, `path` from `worktreePath`,
  `location = 'remote'`. Row state, not registry state — teardown after a
  restart still knows which machine to destroy, and terminals route over the
  existing E12-04 rule without a second one.
- **`ssh_connection_id` is the "already provisioned" test**, not
  `remoteWorkspaceId`: a script may legitimately return a blank id, and
  re-running a provisioner that boots a VM would leak the first one.
- **The pool takes transient connections.** `RemotePtyRegistry` gained an
  in-memory `ConnectionParams` map consulted before the profile store, so a
  BYOI machine gets states, watchdog, backoff and manual-disconnect intent
  for free. Its credential came from a script's stdout and stays in memory —
  it is never written to a row or the keyring, unlike saved profiles.
- **The gate is a cargo feature (`remote-tasks`, default off) enforced at
  provision**, not at settings-write. Provision is the one call that spends
  money on someone else's infrastructure. Enforcing there means a project
  configured on an enabled build stays readable — and terminable — on a
  disabled one.
- **Teardown ignores the gate and cannot fail.** A build without the feature
  must still clean up what an enabled build created; a machine that refuses to
  die warns and leaks rather than making the task undeletable.

## Consequences

- Scripts run where the PROJECT lives (local runner for a local project, the
  project's pooled SSH host for a remote one) — never on the provisioned
  machine, which does not exist yet at provision time and should not be asked
  to destroy itself at terminate time.
- A BYOI machine outlives its session but not its task: `forget_transient`
  drops params, session and state on deletion, and nothing can re-dial it.
- Deleting a task while the terminate script is slow blocks that deletion for
  up to ten minutes (the E12-07 budget). Acceptable: the alternative is
  detaching teardown from the delete that ordered it.
- The gate is compile-time, so the disabled build still compiles every BYOI
  path — rot shows up as a build failure, not as dead code discovered when
  someone flips the flag.
