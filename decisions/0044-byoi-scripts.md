# ADR-0044: BYOI scripts — one runner trait, a strict descriptor, a terminate that cannot fail

- **Status:** Accepted
- **Date:** 2026-08-11
- **Issue:** #91 (E12-07)
- **Relates to:** ADR-0003 (trait placement), ADR-0042 (`RemoteHost`), ADR-0043 (connection lifecycle), PRD E12-07 / E12-10

## Context

A project can delegate workspace creation to two commands: *provision* prints
a JSON descriptor of a machine, *terminate* destroys it. The scripts belong to
the project, so they run where the project lives — this laptop for a local
project, the SSH host for a remote one. The reference implements the flow
twice-shaped (`IExecutionContext` with a local and an SSH implementation) and
validates the descriptor with zod.

Three things needed deciding for the Rust port: where the abstraction sits,
how strict the descriptor is, and what happens when teardown goes wrong.

## Decision

- **`ScriptRunner` in `fartcode-core`, implementations per machine.** One
  method (`run_script(command, env, timeout)`); the local implementation is
  `tokio::process` with `kill_on_drop`, the remote one is `fartcode-ssh`'s
  `SshScriptRunner` over `RemoteHost::run`. Same split as `RemoteHost` and
  `GitOps`, and it keeps the flow unit-testable against a fake.
- **The descriptor is validated, not trusted.** `id` and a non-empty `host`
  are required; unknown fields are ignored (a script may print bookkeeping).
  Empty output, non-JSON output, and a hostless descriptor are three separate
  errors quoting the script's own output, capped at 200 characters.
- **`password` is redacted in `Debug`.** `ProvisionOutput` hand-writes
  `Debug`, the same defense `AuthMethod` already carries: a tracing field or a
  test dump cannot leak a provisioned machine's credential.
- **Terminate returns `()`, never `Err`.** It runs during task teardown; a
  machine that is already gone, a script that exits nonzero, and a host that
  stopped answering all warn and continue. A teardown that can fail is a task
  that can get stuck half-deleted.
- **Ten minutes for both scripts** — a provisioner may boot a VM, and
  terminate waits on the same infrastructure. A timeout is an error naming the
  budget; the local child dies with the dropped future.
- **Refusals over silent degradation at connect time**: `forwardAgent: true`
  with no `SSH_AUTH_SOCK`, and a descriptor with neither password nor
  available agent, are errors. Both would otherwise surface as an opaque auth
  failure against a machine the user is already being billed for.
- **Env goes through `script_command_line`**: values single-quoted, keys
  validated as plain identifiers (a shell assignment cannot quote its left
  side, so a bad key is our bug, not user data).

## Consequences

- Nothing calls this yet. The provision flow, the feature gate, and
  registering the provisioned host in the E12-06 pool are E12-10; this ticket
  is the contract plus both runners, so E12-10 is wiring rather than design.
- A BYOI machine's password lives in `ProvisionOutput` in memory only — it is
  never persisted, unlike saved connection profiles (keyring, ADR-0041).
- `LocalScriptRunner` passes env through the process API while
  `SshScriptRunner` builds a quoted shell line; the observable contract is the
  same, but only the remote path has a quoting risk, and only it is tested for
  injection.
- Terminate's silence means a leaked machine is possible when a script
  misbehaves. The warning is the only signal; surfacing it in the UI is
  E12-10's call.
