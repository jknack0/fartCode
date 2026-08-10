-- Per-project worktree pool segment (#81). The legacy pool segment was
-- `safePathSegment(name)` only, so two projects sharing a basename shared a
-- pool — deleting one removed the other's worktrees. The segment is now
-- stored on the row: NULL on upgrade, stamped by the one-shot adoption pass
-- (legacy value when unique, `<segment>-<hash8>` on collision) or lazily by
-- the pool resolver on first use.
ALTER TABLE projects ADD COLUMN worktree_pool_segment TEXT;
