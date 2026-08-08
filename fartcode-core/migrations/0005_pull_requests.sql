-- PR sync storage (E4-09, #49): one row per PR URL. Scalar columns cover
-- the query paths (workspace scope, branch guard); the full denormalized
-- view (files/commits/checks/comments) rides in the versioned-JSON `data`
-- column — the §11 pattern for list-shaped JSON, chosen over four
-- normalized sub-tables (decisions/0034).
CREATE TABLE pull_requests (
    url            TEXT PRIMARY KEY NOT NULL,
    workspace_id   TEXT,
    owner          TEXT NOT NULL,
    repo           TEXT NOT NULL,
    number         INTEGER NOT NULL,
    title          TEXT NOT NULL,
    status         TEXT NOT NULL,
    draft          INTEGER DEFAULT 0 NOT NULL,
    base_ref       TEXT,
    head_ref       TEXT NOT NULL,
    head_oid       TEXT,
    synced_at      TEXT DEFAULT (datetime('now')) NOT NULL,
    data           TEXT NOT NULL
);
CREATE INDEX idx_pull_requests_workspace ON pull_requests(workspace_id);
CREATE INDEX idx_pull_requests_repo_head ON pull_requests(owner, repo, head_ref);
