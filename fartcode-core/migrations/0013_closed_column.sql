-- Closed column (post-Done cleanup shelf): Done keeps the worktree; the
-- move into Done offers the delete-task/worktree confirm, and a card whose
-- worktree is gone rests in Closed. counts_as_done so blockers sitting in
-- Closed still read as finished (ADR-0037 item 6); no seed_lane — the
-- legacy lane is untouched by entry (lane sync only follows seeded
-- columns). Appended at the end of every existing board that lacks one;
-- new projects get it from seed_default_columns.
INSERT INTO board_columns (id, project_id, name, position, kind, counts_as_done, is_landing, on_enter, on_settle, seed_lane)
SELECT 'col_' || lower(hex(randomblob(16))), p.id, 'Closed',
       (SELECT COALESCE(MAX(bc.position), -1) + 1 FROM board_columns bc WHERE bc.project_id = p.id),
       'shelf', 1, 0, 'queue', 'hold', NULL
  FROM projects p
 WHERE NOT EXISTS (
     SELECT 1 FROM board_columns c
      WHERE c.project_id = p.id AND c.name = 'Closed'
 );
