//! fartcode-core — domain modules (db, settings, projects, tasks, ...).
//!
//! This crate is the dependency leaf of the workspace: it depends only on
//! third-party crates. See ARCHITECTURE.md §2 for the module layout.

pub mod conversations;
pub mod db;
pub mod dependencies;
pub mod error;
pub mod events;
pub mod files;
pub mod fs_watch;
pub mod git;
pub mod issue_proposal;
pub mod issues;
pub mod line_comments;
pub mod projects;
pub mod provider_accounts;
pub mod pty;
pub mod resource_monitor;
pub mod search;
pub mod settings;
pub mod shell_escape;
pub mod tasks;
pub mod terminals;
pub mod view_state;

pub use error::Error;
