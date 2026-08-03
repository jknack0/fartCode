//! Versioned JSON column helper (ARCHITECTURE.md §2, §3).
//!
//! Wire format: `{"version": <u32>, "data": { ...fields }}`.
//!
//! Contract (mirrors the reference's `versioned-column.ts`):
//! - **Reads never throw.** NULL/empty, corrupt JSON, a missing/invalid
//!   `version` field, a *future* version, and *older* versions (which would
//!   need an upgrade chain — none exist in Phase 0) all return `None` with a
//!   warning logged. The only error path is I/O-level (DB error).
//! - **Writes always serialize the latest version** (`T::VERSION`), so the
//!   stored cell is always self-describing.
//!
//! Future phases: add an upgrade chain (`up(v) -> Result<T, NeedsContext>`)
//! between versions; the "older version" arm below becomes the hook for it.

use std::cmp::Ordering;

use rusqlite::{Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};

use crate::Error;

/// Marker trait for versioned JSON payloads stored in a versioned column.
pub trait Versioned: Serialize + DeserializeOwned {
    const VERSION: u32;
}

/// Serializes `value` as the latest version: `{"version": N, "data": {...}}`.
pub fn serialize_versioned<T: Versioned>(value: &T) -> Result<String, Error> {
    let obj = serde_json::json!({ "version": T::VERSION, "data": value });
    serde_json::to_string(&obj).map_err(|e| Error::VersionedJson {
        column: "?".into(),
        reason: e.to_string(),
    })
}

/// Parses a versioned JSON cell. **Never panics, never returns `Err` for bad
/// data** — corrupt/future/older values log a warning and yield `None`.
pub fn parse_versioned<T: Versioned>(column: &str, cell: Option<&str>) -> Option<T> {
    let cell = cell.map(str::trim).filter(|s| !s.is_empty())?;

    let value: serde_json::Value = match serde_json::from_str(cell) {
        Ok(value) => value,
        Err(e) => {
            tracing::warn!(column, error = %e, "versioned JSON cell is corrupt");
            return None;
        }
    };

    let Some(obj) = value.as_object() else {
        tracing::warn!(column, "versioned JSON cell is not an object");
        return None;
    };

    let Some(version) = obj.get("version").and_then(|v| v.as_u64()) else {
        tracing::warn!(
            column,
            "versioned JSON cell is missing a numeric 'version' field"
        );
        return None;
    };
    let Some(version) = u32::try_from(version).ok() else {
        tracing::warn!(column, "versioned JSON cell version out of range");
        return None;
    };

    match version.cmp(&T::VERSION) {
        Ordering::Equal => {
            let Some(data) = obj.get("data") else {
                tracing::warn!(
                    column,
                    version,
                    "versioned JSON cell is missing its 'data' field"
                );
                return None;
            };
            match serde_json::from_value::<T>(data.clone()) {
                Ok(value) => Some(value),
                Err(e) => {
                    tracing::warn!(column, version, error = %e, "versioned JSON data does not match the declared schema");
                    None
                }
            }
        }
        Ordering::Greater => {
            tracing::warn!(
                column,
                stored_version = version,
                current_version = T::VERSION,
                "versioned JSON cell has a future version"
            );
            None
        }
        Ordering::Less => {
            tracing::warn!(
                column,
                stored_version = version,
                current_version = T::VERSION,
                "versioned JSON cell is an older version; no upgrade chain exists yet"
            );
            None
        }
    }
}

/// Reads a versioned column for the row with the given `id`.
///
/// `table`/`column` are compile-time constants from our own schema — never
/// user input. Missing rows and unparseable cells both yield `None`.
pub fn read_versioned<T: Versioned>(
    conn: &Connection,
    table: &str,
    column: &str,
    id: &str,
) -> Result<Option<T>, Error> {
    let sql = format!("SELECT {column} FROM {table} WHERE id = ?1");
    let cell: Option<String> = conn.query_row(&sql, [id], |row| row.get(0)).optional()?;
    Ok(parse_versioned::<T>(column, cell.as_deref()))
}

/// Writes `value` (serialized at its latest version) into a versioned column
/// for the row with the given `id`.
pub fn write_versioned<T: Versioned>(
    conn: &Connection,
    table: &str,
    column: &str,
    id: &str,
    value: &T,
) -> Result<(), Error> {
    let sql = format!("UPDATE {table} SET {column} = ?1 WHERE id = ?2");
    let serialized = serialize_versioned(value).map_err(|e| match e {
        Error::VersionedJson { reason, .. } => Error::VersionedJson {
            column: column.into(),
            reason,
        },
        other => other,
    })?;
    conn.execute(&sql, rusqlite::params![serialized, id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Payload {
        name: String,
    }

    impl Versioned for Payload {
        const VERSION: u32 = 2;
    }

    #[test]
    fn parse_handles_empty_and_null_cells() {
        assert_eq!(parse_versioned::<Payload>("col", None), None);
        assert_eq!(parse_versioned::<Payload>("col", Some("")), None);
        assert_eq!(parse_versioned::<Payload>("col", Some("   ")), None);
    }

    #[test]
    fn parse_handles_corrupt_json() {
        assert_eq!(parse_versioned::<Payload>("col", Some("{nope")), None);
        assert_eq!(parse_versioned::<Payload>("col", Some("42")), None); // not an object
    }

    #[test]
    fn parse_handles_missing_or_invalid_version() {
        assert_eq!(
            parse_versioned::<Payload>("col", Some(r#"{"data":{"name":"x"}}"#)),
            None
        );
        assert_eq!(
            parse_versioned::<Payload>("col", Some(r#"{"version":"abc","data":{"name":"x"}}"#)),
            None
        );
        assert_eq!(
            parse_versioned::<Payload>("col", Some(r#"{"version":2}"#)),
            None // missing data field
        );
    }

    #[test]
    fn parse_rejects_future_and_older_versions() {
        let future = r#"{"version":3,"data":{"name":"x"}}"#;
        assert_eq!(parse_versioned::<Payload>("col", Some(future)), None);

        let older = r#"{"version":1,"data":{"name":"x"}}"#;
        assert_eq!(parse_versioned::<Payload>("col", Some(older)), None);
    }

    #[test]
    fn parse_accepts_current_version() {
        let json = r#"{"version":2,"data":{"name":"x"}}"#;
        assert_eq!(
            parse_versioned::<Payload>("col", Some(json)),
            Some(Payload { name: "x".into() })
        );
    }

    #[test]
    fn serialize_writes_latest_version_and_roundtrips() {
        let value = Payload { name: "y".into() };
        let json = serialize_versioned(&value).unwrap();
        assert!(
            json.contains("\"version\":2"),
            "writes must be latest version: {json}"
        );
        assert_eq!(parse_versioned::<Payload>("col", Some(&json)), Some(value));
    }
}
