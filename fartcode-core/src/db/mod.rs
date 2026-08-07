//! Database layer: connection singleton, migration runner, FTS bootstrap,
//! and the versioned-JSON column helper (ARCHITECTURE.md §2).

pub mod connection;
pub mod migrations;
pub mod schema;
pub mod versioned_json;

pub use connection::{resolve_db_path, Db, SqliteDb};
pub use migrations::{ensure_fts_tables, run_migrations};
pub use versioned_json::{
    parse_versioned, read_versioned, serialize_versioned, write_versioned, Versioned,
};
