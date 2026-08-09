-- Configurable pipeline columns (E18-01, ADR-0037): per-project board
-- columns as data — shelf / agent_step / human_gate kinds, per-column step
-- config (prompt/provider/model/effort/tools), per-column trigger
-- (`on_enter`) and settle (`on_settle`) behavior with an optional
-- `advance_to` target (NULL = next column), `counts_as_done` replacing
-- 'done' string tests, and exactly one `is_landing` target per board
-- (enforced in ColumnStore, not schema).
--
-- SPIKE, BEHIND THE SEEDED DEFAULT: the `lane` column on `issues` STAYS
-- AUTHORITATIVE for all existing behavior (board order, dispatch, flip,
-- blocked derivation). `issues.column_id` is a MIRRORED POINTER only —
-- backfilled here from the five lane strings and not yet written by any
-- production path. `seed_lane` records which legacy lane a seeded column
-- mirrors ('backlog'|'ready'|'in_progress'|'in_review'|'done'; NULL on
-- Quick and user-created columns) so occupancy checks can derive from the
-- authoritative lane instead of the unmaintained mirror. Enum values are
-- comments + store validation, not CHECKs, matching 0002.
CREATE TABLE board_columns (
    id             TEXT PRIMARY KEY NOT NULL,
    project_id     TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name           TEXT NOT NULL,
    position       INTEGER DEFAULT 0 NOT NULL,
    kind           TEXT DEFAULT 'shelf' NOT NULL,   -- shelf|agent_step|human_gate
    counts_as_done INTEGER DEFAULT 0 NOT NULL,
    is_landing     INTEGER DEFAULT 0 NOT NULL,
    on_enter       TEXT DEFAULT 'queue' NOT NULL,   -- run|queue (agent_step trigger)
    on_settle      TEXT DEFAULT 'hold' NOT NULL,    -- hold|advance (agent_step settle)
    advance_to     TEXT REFERENCES board_columns(id) ON DELETE SET NULL,
                                     -- explicit advance target; NULL = next column
    step_prompt    TEXT,            -- NULL on the seeded steps = built-in dispatch packet
    step_provider  TEXT,
    step_model     TEXT,
    step_effort    TEXT,
    step_tools     TEXT,            -- versioned JSON: StepTools; NULL = unrestricted
    seed_lane      TEXT,            -- legacy lane this seeded column mirrors (NULL = none)
    created_at     TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at     TEXT DEFAULT (datetime('now')) NOT NULL
);
--> statement-breakpoint
CREATE INDEX idx_board_columns_project ON board_columns(project_id, position);
--> statement-breakpoint
ALTER TABLE issues ADD COLUMN column_id TEXT REFERENCES board_columns(id) ON DELETE SET NULL;
--> statement-breakpoint
-- Seed the classic five plus Quick (ADR-0037 item 8) for every EXISTING
-- project: Backlog · Ready · Quick · In Progress · In Review · Done. The two
-- agentic drop targets sit adjacent; Backlog is leftmost as the landing
-- lane. In Progress carries today's dispatch semantics (on_enter run) and
-- the E17-03 auto-flip (on_settle advance, advance_to NULL = next column),
-- with NULL provider/model = the project default; Quick is the gateless
-- small-work lane, pinned to the cheap agent (design handoff v3:
-- claude · haiku — per-issue override convention: provider registry id +
-- bare Claude model alias) and advancing straight to Done (ADR item 4 —
-- without the target a Quick card would walk into In Progress and fire a
-- second unconfirmed dispatch); In Review is the human gate; Done counts
-- as done. (New projects get the identical set from seed_default_columns
-- at creation.)
INSERT INTO board_columns (id, project_id, name, position, kind, counts_as_done, is_landing, on_enter, on_settle, seed_lane)
SELECT 'col_' || lower(hex(randomblob(16))), id, 'Backlog', 0, 'shelf', 0, 1, 'queue', 'hold', 'backlog' FROM projects;
--> statement-breakpoint
INSERT INTO board_columns (id, project_id, name, position, kind, counts_as_done, is_landing, on_enter, on_settle, seed_lane)
SELECT 'col_' || lower(hex(randomblob(16))), id, 'Ready', 1, 'shelf', 0, 0, 'queue', 'hold', 'ready' FROM projects;
--> statement-breakpoint
INSERT INTO board_columns (id, project_id, name, position, kind, counts_as_done, is_landing, on_enter, on_settle, seed_lane, step_provider, step_model)
SELECT 'col_' || lower(hex(randomblob(16))), id, 'Quick', 2, 'agent_step', 0, 0, 'run', 'advance', NULL, 'claude', 'haiku' FROM projects;
--> statement-breakpoint
INSERT INTO board_columns (id, project_id, name, position, kind, counts_as_done, is_landing, on_enter, on_settle, seed_lane)
SELECT 'col_' || lower(hex(randomblob(16))), id, 'In Progress', 3, 'agent_step', 0, 0, 'run', 'advance', 'in_progress' FROM projects;
--> statement-breakpoint
INSERT INTO board_columns (id, project_id, name, position, kind, counts_as_done, is_landing, on_enter, on_settle, seed_lane)
SELECT 'col_' || lower(hex(randomblob(16))), id, 'In Review', 4, 'human_gate', 0, 0, 'queue', 'hold', 'in_review' FROM projects;
--> statement-breakpoint
INSERT INTO board_columns (id, project_id, name, position, kind, counts_as_done, is_landing, on_enter, on_settle, seed_lane)
SELECT 'col_' || lower(hex(randomblob(16))), id, 'Done', 5, 'shelf', 1, 0, 'queue', 'hold', 'done' FROM projects;
--> statement-breakpoint
-- Quick advances straight to its project's Done column (ADR-0037 item 4).
-- Only the seeded columns exist at this point, so matching on seed_lane is
-- exact.
UPDATE board_columns SET advance_to = (
    SELECT d.id FROM board_columns d
     WHERE d.project_id = board_columns.project_id AND d.seed_lane = 'done'
) WHERE name = 'Quick' AND kind = 'agent_step' AND seed_lane IS NULL;
--> statement-breakpoint
-- Backfill the mirror pointer by joining lane to the seeded column that
-- mirrors it (`seed_lane`). Lane remains the source of truth; every lane
-- value Lane::parse admits has a seeded column, so no issue is left
-- unmapped.
UPDATE issues SET column_id = (
    SELECT c.id FROM board_columns c
     WHERE c.project_id = issues.project_id
       AND c.seed_lane = issues.lane
);
