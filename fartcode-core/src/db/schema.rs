//! CREATE TABLE statements (ARCHITECTURE.md §2 — mirrors `drizzle/0000_*.sql`).
//!
//! The canonical SQL lives on disk under `migrations/` and is embedded at
//! build time (`include_str!`), so the compiled binary is self-contained and
//! the runner can hash-verify applied migrations ("hand-edit of a numbered
//! migration is not possible by design — embedded + hashed").

/// Full text of the initial migration (`migrations/0000_initial.sql`).
pub const INITIAL_MIGRATION_SQL: &str = include_str!("../../migrations/0000_initial.sql");

/// Domain tables created by `0000_initial.sql` (FTS tables are created
/// outside migrations by `ensure_fts_tables`).
pub const DOMAIN_TABLES: &[&str] = &[
    "app_settings",
    "kv",
    "projects",
    "project_remotes",
    "project_settings",
    "tasks",
    "workspaces",
    "conversations",
    "terminals",
    "messages",
    "editor_buffers",
    "automations",
    "automation_runs",
    "ssh_connections",
    "provider_accounts",
    "line_comments",
    "issues",
    "pull_requests",
    "board_columns",
];

/// FTS tables created by the version-gated bootstrap (`ensure_fts_tables`).
pub const FTS_TABLES: &[&str] = &[
    "search_index",
    "workspace_file_index",
    "workspace_file_index_meta",
];
