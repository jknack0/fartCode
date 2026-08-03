//! ade-core — domain modules (db, settings, projects, tasks, ...).
//!
//! This crate is the dependency leaf of the workspace: it depends only on
//! third-party crates. See ARCHITECTURE.md §2 for the module layout.

pub mod db;
pub mod error;

pub use error::Error;
