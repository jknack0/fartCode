-- DONE REPLACES CLOSED (supersedes 0013): one terminal shelf per board,
-- named `Done`. 0013 appended `Closed` to every board that existed when it
-- ran, but SEED_COLUMNS never carried a terminal shelf, so every project
-- created after 0013 got Idea..Ship and no resting place at all — the
-- migration comment's claim that "new projects get it from
-- seed_default_columns" was false until this migration's companion seed
-- entry. Same shape either way: `shelf`, `counts_as_done` (so blockers
-- resting there still read as finished, ADR-0037 item 6), no `seed_lane`
-- (entry leaves the legacy display lane untouched — lane sync only follows
-- seeded columns).
--
-- Three populations, in order: boards carrying BOTH (a legacy 0006 board,
-- whose 'done'-lane column IS the shelf, or a user who made their own)
-- merge and drop; boards carrying only `Closed` are renamed in place,
-- which keeps the id, its cards and any `advance_to` pins aimed at it;
-- boards carrying neither get one appended at MAX(position)+1. Replay-safe:
-- after the first pass no `Closed` remains and every board has a `Done`, so
-- all four statements match nothing.

-- Cards resting in a redundant `Closed` move to the board's `Done`.
UPDATE issues SET column_id = (
    SELECT d.id FROM board_columns d
     WHERE d.project_id = issues.project_id AND d.name = 'Done'
) WHERE column_id IN (
    SELECT c.id FROM board_columns c
     WHERE c.name = 'Closed' AND c.is_landing = 0
       AND EXISTS (SELECT 1 FROM board_columns d
                    WHERE d.project_id = c.project_id AND d.name = 'Done')
);
--> statement-breakpoint
-- Any step pinned to advance into that `Closed` follows the cards.
UPDATE board_columns SET advance_to = (
    SELECT d.id FROM board_columns d
     WHERE d.project_id = board_columns.project_id AND d.name = 'Done'
) WHERE advance_to IN (
    SELECT c.id FROM board_columns c
     WHERE c.name = 'Closed' AND c.is_landing = 0
       AND EXISTS (SELECT 1 FROM board_columns d
                    WHERE d.project_id = c.project_id AND d.name = 'Done')
);
--> statement-breakpoint
-- The emptied duplicate goes. `is_landing = 0` is a guard, not a
-- formality: deleting a landing column would leave the board with no
-- landing target, which ColumnStore treats as unrepresentable. A landing
-- `Closed` is hand-made and vanishingly rare; it keeps its name rather
-- than collide with the `Done` beside it.
DELETE FROM board_columns
 WHERE name = 'Closed' AND is_landing = 0
   AND EXISTS (SELECT 1 FROM board_columns d
                WHERE d.project_id = board_columns.project_id AND d.name = 'Done');
--> statement-breakpoint
-- Every remaining `Closed` is its board's only terminal shelf: rename in
-- place so ids, cards and pins survive.
UPDATE board_columns SET name = 'Done' WHERE name = 'Closed';
--> statement-breakpoint
-- Boards seeded after 0013 have no terminal shelf at all.
INSERT INTO board_columns (id, project_id, name, position, kind, counts_as_done, is_landing, on_enter, on_settle, seed_lane)
SELECT 'col_' || lower(hex(randomblob(16))), p.id, 'Done',
       (SELECT COALESCE(MAX(bc.position), -1) + 1 FROM board_columns bc WHERE bc.project_id = p.id),
       'shelf', 1, 0, 'queue', 'hold', NULL
  FROM projects p
 WHERE NOT EXISTS (
     SELECT 1 FROM board_columns c
      WHERE c.project_id = p.id AND c.name = 'Done'
 );
