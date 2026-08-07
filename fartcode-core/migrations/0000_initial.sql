-- 0000_initial — Phase 0 consolidated schema (ARCHITECTURE.md §11).
--
-- Mirrors the reference's `0000_cuddly_scarecrow.sql` with later additions
-- pulled forward for Phase 0 convenience. Append-only: never hand-edit an
-- applied migration — add a new numbered file instead.
--
-- FTS tables (search_index, workspace_file_index, workspace_file_index_meta)
-- are deliberately NOT here — they are created outside migrations and
-- version-gated via the `kv` table (see db/migrations.rs ensure_fts_tables).

-- Settings
CREATE TABLE app_settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL,
    updated_at INTEGER DEFAULT (unixepoch()) NOT NULL
);
--> statement-breakpoint

CREATE TABLE kv (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL,
    updated_at INTEGER DEFAULT (unixepoch()) NOT NULL
);
--> statement-breakpoint

-- Projects
CREATE TABLE projects (
    id                 TEXT PRIMARY KEY NOT NULL,
    name               TEXT NOT NULL,
    path               TEXT NOT NULL,
    workspace_provider TEXT DEFAULT 'local' NOT NULL,
    base_ref           TEXT,
    ssh_connection_id  TEXT,
    repository_workspace_id TEXT,
    created_at         TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at         TEXT DEFAULT (datetime('now')) NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX idx_projects_path ON projects(path);
--> statement-breakpoint

CREATE TABLE project_remotes (
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    remote_name TEXT NOT NULL,
    remote_url  TEXT NOT NULL,
    PRIMARY KEY (project_id, remote_name)
);
--> statement-breakpoint

CREATE TABLE project_settings (
    project_id                   TEXT PRIMARY KEY NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    base_project_settings_json   TEXT NOT NULL DEFAULT '{}',
    shareable_project_settings_json TEXT NOT NULL DEFAULT '{}',
    legacy_config_migrated_at    TEXT,
    created_at                   TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at                   TEXT DEFAULT (datetime('now')) NOT NULL
);
--> statement-breakpoint

-- Tasks
CREATE TABLE tasks (
    id                  TEXT PRIMARY KEY NOT NULL,
    project_id          TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name                TEXT NOT NULL,
    status              TEXT NOT NULL,
    linked_issue        TEXT,
    archived_at         TEXT,
    created_at          TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at          TEXT DEFAULT (datetime('now')) NOT NULL,
    last_interacted_at  TEXT,
    status_changed_at   TEXT DEFAULT (datetime('now')) NOT NULL,
    is_pinned           INTEGER DEFAULT 0 NOT NULL,
    created_by          TEXT DEFAULT 'user' NOT NULL,  -- 'user' | 'agent:<conversation_id>'
    workspace_id        TEXT,
    workspace_intent    TEXT,
    type                TEXT DEFAULT 'task' NOT NULL,
    automation_run_id   TEXT
);
--> statement-breakpoint
CREATE INDEX idx_tasks_project_id ON tasks(project_id);
--> statement-breakpoint

-- Workspaces
CREATE TABLE workspaces (
    id                TEXT PRIMARY KEY NOT NULL,
    key               TEXT,
    type              TEXT,  -- local | project-ssh | byoi
    kind              TEXT,  -- worktree | project-root | byoi
    location          TEXT,  -- local | remote
    ssh_connection_id TEXT,
    data              TEXT,  -- versioned JSON
    path              TEXT,
    config            TEXT,  -- versioned JSON
    branch_name       TEXT,
    lines_added       INTEGER,
    lines_deleted     INTEGER,
    created_at        TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at        TEXT DEFAULT (datetime('now')) NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX idx_workspaces_key ON workspaces(key) WHERE key IS NOT NULL;
--> statement-breakpoint

-- Conversations
CREATE TABLE conversations (
    id                       TEXT PRIMARY KEY NOT NULL,
    project_id               TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    task_id                  TEXT REFERENCES tasks(id) ON DELETE CASCADE,  -- NULL for project-scoped
    scope                    TEXT DEFAULT 'task' NOT NULL,  -- 'task' | 'project'
    title                    TEXT NOT NULL,
    provider                 TEXT,
    config                   TEXT,  -- versioned JSON
    created_at               TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at               TEXT DEFAULT (datetime('now')) NOT NULL,
    last_interacted_at       TEXT,
    is_initial_conversation  INTEGER,
    session_id               TEXT,
    agent_status             TEXT,
    agent_status_seen        INTEGER DEFAULT 1,
    type                     TEXT
);
--> statement-breakpoint
CREATE INDEX idx_conversations_task_id ON conversations(task_id);
--> statement-breakpoint

-- Terminals
CREATE TABLE terminals (
    id         TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    task_id    TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    ssh        INTEGER DEFAULT 0 NOT NULL,
    name       TEXT NOT NULL,
    shell_id   TEXT NOT NULL DEFAULT 'system',
    created_at TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at TEXT DEFAULT (datetime('now')) NOT NULL
);
--> statement-breakpoint
CREATE INDEX idx_terminals_task_id ON terminals(task_id);
--> statement-breakpoint

-- Messages (legacy — read-only in Phase 0, useful for migration path)
CREATE TABLE messages (
    id              TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    content         TEXT NOT NULL,
    sender          TEXT NOT NULL,
    timestamp       TEXT DEFAULT (datetime('now')) NOT NULL,
    metadata        TEXT
);
--> statement-breakpoint
CREATE INDEX idx_messages_conversation_id ON messages(conversation_id);
--> statement-breakpoint

-- Editor buffers
CREATE TABLE editor_buffers (
    id           TEXT PRIMARY KEY NOT NULL,  -- {project_id}:{workspace_id}:{file_path}
    project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    file_path    TEXT NOT NULL,
    content      TEXT NOT NULL,
    updated_at   INTEGER NOT NULL
);
--> statement-breakpoint
CREATE INDEX idx_editor_buffers_workspace_file ON editor_buffers(workspace_id, file_path);
--> statement-breakpoint

-- Automations (Phase 2, table exists for FK references)
CREATE TABLE automations (
    id                  TEXT PRIMARY KEY NOT NULL,
    name                TEXT NOT NULL,
    project_id          TEXT REFERENCES projects(id) ON DELETE SET NULL,
    trigger_config      TEXT,  -- versioned JSON
    conversation_config TEXT,  -- versioned JSON
    task_config         TEXT,  -- versioned JSON
    enabled             INTEGER DEFAULT 1 NOT NULL,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    deleted_at          INTEGER
);
--> statement-breakpoint

CREATE TABLE automation_runs (
    id                            TEXT PRIMARY KEY NOT NULL,
    automation_id                 TEXT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
    scheduled_at                  INTEGER,
    deadline_at                   INTEGER,
    started_at                    INTEGER,
    task_created_at               INTEGER,
    launched_at                   INTEGER,
    finished_at                   INTEGER,
    status                        TEXT NOT NULL,
    error                         TEXT,
    trigger_kind                  TEXT NOT NULL,
    trigger_config_snapshot       TEXT NOT NULL DEFAULT '{}',
    conversation_config_snapshot  TEXT NOT NULL DEFAULT '{}',
    task_config_snapshot          TEXT,
    generated_task_name           TEXT
);
--> statement-breakpoint

-- SSH connections (Phase 3, table exists for FK references)
CREATE TABLE ssh_connections (
    id               TEXT PRIMARY KEY NOT NULL,
    name             TEXT NOT NULL,
    host             TEXT NOT NULL,
    port             INTEGER DEFAULT 22 NOT NULL,
    username         TEXT NOT NULL,
    auth_type        TEXT DEFAULT 'agent' NOT NULL,
    private_key_path TEXT,
    use_agent        INTEGER DEFAULT 0 NOT NULL,
    metadata         TEXT,  -- versioned JSON
    created_at       TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at       TEXT DEFAULT (datetime('now')) NOT NULL
);
--> statement-breakpoint

-- Provider accounts (Phase 2, table exists for FK references)
CREATE TABLE provider_accounts (
    id             TEXT PRIMARY KEY NOT NULL,
    provider_id    TEXT NOT NULL,
    account_id     TEXT NOT NULL,
    credential_ref TEXT NOT NULL,
    is_default     INTEGER DEFAULT 0 NOT NULL,
    meta           TEXT,  -- versioned JSON
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);
--> statement-breakpoint

-- Diff line comments (Phase 1 diff view; base table kept from the reference's
-- 0000 per ARCHITECTURE.md §14 — the reference later dropped it in 0004).
CREATE TABLE line_comments (
    id           TEXT PRIMARY KEY NOT NULL,
    task_id      TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    file_path    TEXT NOT NULL,
    line_number  INTEGER NOT NULL,
    line_content TEXT,
    content      TEXT NOT NULL,
    created_at   TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at   TEXT DEFAULT (datetime('now')) NOT NULL,
    sent_at      TEXT
);
--> statement-breakpoint
CREATE INDEX idx_line_comments_task_file ON line_comments(task_id, file_path);
