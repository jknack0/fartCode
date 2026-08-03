# ADR-0016 — Worktree-directory validation + settings surface

Status: accepted (ticket E1-05)

## Context

E1-05 needs per-project settings editable in-app: worktree directory,
branch/remotes, lifecycle scripts, preserve patterns, workspace provider,
auto-run toggles, and share-with-team (write shareable fields to the repo
`.ade.json`, clear local). The settings store and share-with-team already
existed (E1-02); what was missing was worktree-directory validation and the
app surface (commands + panel).

## Decision

1. **Validation is a pure core fn** (`settings::worktree_directory::normalize_worktree_directory`),
   a port of the reference `worktree-directory.ts`: trim, expand
   `~`/`~/`/`~\` via the home dir, require an absolute path (posix `/`,
   windows drive `X:\`/`X:/`, or UNC `\\`), else the typed
   `invalid-worktree-directory` error. The error's `Display` carries the
   literal code so the UI can branch on it.
2. **Validate on write AND read**: the `update_project_settings` command
   normalizes before storing (so an invalid value never lands in the DB);
   `get_project_settings` re-validates on read — a stored-invalid value (or
   a `~` whose home changed) falls back to the default. This matches the
   reference's `resolveAndValidateWorktreeDirectory` on read, which also
   expands `~` (the legacy-migration test now asserts the expanded form).
3. **Commands**: `get_project_settings` / `update_project_settings`
   (full-replace, camelCase DTO) / `share_with_team` — thin, `String` errors.
4. **Panel**: modal with every field, defaults shown as placeholders for
   unset values (`~/ade/worktrees`, `main`, `origin`), save/validation
   errors, and a share-with-team button. The `defaultBranch` untagged union
   (`"main"` | `{ name, remote }`) is edited as name + remote checkbox in
   the UI.

## Consequences

- Invalid worktree directories are rejected with a clear, typed error at
  the command boundary; `~` works everywhere (write + read).
- Share-with-team moves shareable fields into `.ade.json` and clears them
  locally (existing E1-02 behavior), surfaced in the panel with a fresh
  read afterwards.
- The panel is modal-on-gear; a full settings page could reuse the same
  DTOs later.
