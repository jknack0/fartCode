-- Project board issues (E17-01, #55; ARCHITECTURE.md §13, ADR-0032):
-- local-first issue store + blocked-by edges. Blocked status is derived at
-- read time (any blocker lane != 'done'), never stored; both edge endpoints
-- cascade on issue delete; linked_task_id clears when its task is deleted.
CREATE TABLE issues (
    id              TEXT PRIMARY KEY NOT NULL,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    title           TEXT NOT NULL,
    body            TEXT,
    acceptance      TEXT,            -- versioned JSON: AcceptanceCriteria
    lane            TEXT DEFAULT 'backlog' NOT NULL,  -- backlog|ready|in_progress|in_review|done
    position        INTEGER DEFAULT 0 NOT NULL,
    provider        TEXT,
    model           TEXT,
    prd_path        TEXT,
    prd_section     TEXT,
    linked_task_id  TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    created_at      TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at      TEXT DEFAULT (datetime('now')) NOT NULL
);
--> statement-breakpoint
CREATE INDEX idx_issues_project_lane ON issues(project_id, lane, position);
--> statement-breakpoint
CREATE TABLE issue_dependencies (
    issue_id      TEXT NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    blocked_by_id TEXT NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    PRIMARY KEY (issue_id, blocked_by_id)
);
