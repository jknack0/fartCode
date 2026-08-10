//! Tauri command modules (E1-04): thin wrappers over the domain services.
//! Commands map errors to `String` and return DTOs (AGENTS.md: commands are
//! thin).

pub mod columns;
pub mod conversations;
pub mod dependencies;
pub mod dossiers;
pub mod files;
pub mod git;
pub mod github;
pub mod issue_proposals;
pub mod issues;
pub mod lifecycle;
pub mod line_comments;
pub mod projects;
pub mod provider_accounts;
pub mod search;
pub mod settings;
pub mod steps;
pub mod tasks;
pub mod telemetry;
pub mod terminals;
pub mod view_state;
