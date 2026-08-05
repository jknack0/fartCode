//! Tauri command modules (E1-04): thin wrappers over the domain services.
//! Commands map errors to `String` and return DTOs (AGENTS.md: commands are
//! thin).

pub mod conversations;
pub mod git;
pub mod projects;
pub mod provider_accounts;
pub mod search;
pub mod settings;
pub mod tasks;
pub mod terminals;
pub mod view_state;
