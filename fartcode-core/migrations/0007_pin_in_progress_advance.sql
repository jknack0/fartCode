-- Pin the seeded In Progress column's advance target to In Review
-- (E18-04/05 fix round, ADR-0037 item 4). 0006 left In Progress with
-- advance_to NULL (= next column by position), relying on In Review
-- sitting adjacent — a reorder, or deleting In Review, would silently
-- reroute the auto-flip to Done and skip the human gate. Same pin
-- pattern 0006 already used for Quick → Done. Shipping as a NEW
-- migration because 0006 has landed and applied migrations are
-- sha256-frozen (append-only policy); the Rust seed (SEED_COLUMNS)
-- carries the identical pin for projects created after this.
UPDATE board_columns SET advance_to = (
    SELECT r.id FROM board_columns r
     WHERE r.project_id = board_columns.project_id AND r.seed_lane = 'in_review'
) WHERE seed_lane = 'in_progress';
