-- THE AUTHORITY FLIP (E18-07, #66; ADR-0037): `issues.column_id` becomes
-- the authoritative record of a card's board placement; `issues.lane` is
-- demoted to a derived display mirror maintained only for seeded columns.
--
-- Backfill every remaining mirrorless row so the flipped code can rely on
-- a non-NULL column_id everywhere (single-join blocked derivation,
-- column-id-only delete-guard occupancy, column-position board order):
-- - a row whose lane still has its seeded column (`seed_lane` match) gets
--   that column — identical to the 0006 backfill;
-- - a row whose seeded column was DELETED falls back to the project's
--   landing column, mirroring the frontend's `columnIdForIssue` display
--   resolution (which stays as defensive rendering, not authority).
-- Rows already carrying a mirror are untouched. A project with no columns
-- at all cannot occur outside hand-built fixtures (0006 seeded every
-- existing project; project creation seeds new ones), so the COALESCE can
-- only be NULL where there is no board to be authoritative about.
UPDATE issues SET column_id = COALESCE(
    (SELECT c.id FROM board_columns c
      WHERE c.project_id = issues.project_id
        AND c.seed_lane = issues.lane),
    (SELECT l.id FROM board_columns l
      WHERE l.project_id = issues.project_id
        AND l.is_landing = 1)
) WHERE column_id IS NULL;
