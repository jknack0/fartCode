//! ade-core — domain modules (db, settings, projects, tasks, ...).
//!
//! This crate is the dependency leaf of the workspace: it depends only on
//! third-party crates. See ARCHITECTURE.md §2 for the module layout.

pub mod conversations;
pub mod db;
pub mod dependencies;
pub mod error;
pub mod events;
pub mod git;
pub mod projects;
pub mod pty;
pub mod settings;
pub mod shell_escape;
pub mod tasks;

pub use error::Error;
