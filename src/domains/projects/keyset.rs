//! The `<epochMillis>:<uuid>` keyset cursor every project page uses, ported
//! exactly from the platform's `keyset-cursor.ts`. It encodes the
//! `(<timestamp>, id)` sort key of a page's last row — read `DESC` by every
//! page, and `ASC` too by `project activity --order asc` — so a walk stays
//! stable under concurrent inserts where an offset would shift.
//!
//! A malformed cursor is a schema-edge refusal: `400 invalid_request`,
//! `cursor is invalid`, distinct from the `422` invariant refusals.

use crate::prelude::*;
use crate::utilities::project_error::ProjectError;

/// A decoded cursor: the sort timestamp and the tie-breaking row id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    /// Epoch milliseconds of the last row on the page.
    pub at_ms: i64,
    /// That row's id.
    pub id: String,
}

/// The cursor for the last row of a page.
#[must_use]
pub fn encode(at_ms: i64, id: &str) -> String {
    format!("{at_ms}:{id}")
}

/// A row a keyset page walks, by its `(created_at, id)` position.
pub trait Positioned {
    /// The row's `created_at` epoch milliseconds and id.
    fn position(&self) -> (i64, &str);
}

/// Split `cursor`, refusing anything outside `^\d{1,15}:[0-9a-f-]{36}$`
/// (ASCII digits, lowercase hex).
pub fn decode(cursor: &str) -> Result<Cursor> {
    parse(cursor).ok_or_else(|| ProjectError::invalid_field("cursor", "invalid").into())
}

/// The shape check and split behind [`decode`].
fn parse(cursor: &str) -> Option<Cursor> {
    let (at, id) = cursor.split_once(':')?;
    let digits = (1..=15).contains(&at.len()) && at.bytes().all(|b| b.is_ascii_digit());
    let hex = id.len() == 36
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-');
    if !(digits && hex) {
        return None;
    }
    Some(Cursor {
        at_ms: at.parse().ok()?,
        id: id.to_string(),
    })
}

#[cfg(test)]
#[path = "tests/keyset.rs"]
mod tests;
