//! Shared serde helpers for command request DTOs — currently the tri-state
//! patch-field deserializer both update requests (columns, issues) need.

use serde::{Deserialize, Deserializer};

/// Tri-state patch-field deserializer: field absent → `None` (keep),
/// explicit `null` → `Some(None)` (clear), value → `Some(Some(v))` (set).
///
/// Plain `Option<Option<T>>` cannot express this — serde collapses an
/// explicit `null` into `None`, silently turning "clear" into "keep" (the
/// codebase has no serde_with dependency, hence the hand-rolled helper).
/// Always pair with `#[serde(default)]` so an absent field stays `None`.
pub(crate) fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
