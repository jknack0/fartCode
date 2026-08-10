-- Step spend ledger (#82; ADR-0037 'Cost surface' consequence). One row per
-- agent-step LAUNCH (kind='launch') and one per chain-guard HOLD
-- (kind='hold'). The ledger is the durable record run-mode chains lacked:
-- who launched what, from which column, whether a human confirmed it
-- (`auto` = 0) or a settle-advance chained it (`auto` = 1), and — where the
-- provider reported any — context-window token usage, backfilled at settle.
-- This is also the substrate ADR-0038 item 7's token metrics want.
--
-- Hold rows carry `reason` ('depth'|'cycle'|'budget') and `target_column_id`
-- (the launch the guard refused); launches leave both NULL. Enum values are
-- comments + store validation, not CHECKs, matching 0002/0006.
CREATE TABLE step_ledger (
    id               TEXT PRIMARY KEY NOT NULL,
    issue_id         TEXT NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    project_id       TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    column_id        TEXT NOT NULL,
    kind             TEXT DEFAULT 'launch' NOT NULL, -- launch|hold
    auto             INTEGER DEFAULT 0 NOT NULL,     -- 1 = settle-chained (no human confirm)
    provider         TEXT,                           -- NULL on hold rows
    model            TEXT,
    reason           TEXT,                           -- hold reason: depth|cycle|budget
    target_column_id TEXT,                           -- the refused launch target (hold rows)
    tokens_used      INTEGER,                        -- provider-reported context usage, settle-backfilled
    created_at       TEXT DEFAULT (datetime('now')) NOT NULL
);
--> statement-breakpoint
CREATE INDEX idx_step_ledger_issue ON step_ledger(issue_id, created_at);
--> statement-breakpoint
CREATE INDEX idx_step_ledger_project ON step_ledger(project_id);
