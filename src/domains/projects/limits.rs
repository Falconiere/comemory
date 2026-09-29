//! The platform's charter and paging caps (`project-limits.ts`), and the
//! checks that turn a breach into a `422 invalid_request` naming the field,
//! the reason and the limit.
//!
//! Lengths are UTF-16 code units, the platform's `String.length`, so a
//! string at the cap there is at the cap here.

use crate::prelude::*;
use crate::utilities::project_error::{ProjectError, RequestEdge};

/// `name` length cap.
pub const NAME_MAX: usize = 120;
/// `outcome` length cap.
pub const OUTCOME_MAX: usize = 2000;
/// One constraint's or non-goal's length cap.
pub const CONSTRAINT_MAX: usize = 500;
/// Constraints per project.
pub const CONSTRAINTS_MAX_COUNT: usize = 50;
/// Non-goals per project.
pub const NON_GOALS_MAX_COUNT: usize = 50;
/// Repositories per project, counted before de-duplication.
pub const REPOSITORIES_MAX: usize = 50;
/// Project-level success criteria.
pub const CRITERIA_MAX: usize = 50;
/// One criterion description's length cap.
pub const CRITERION_DESCRIPTION_MAX: usize = 500;
/// A lifecycle, review or cancellation reason's length cap: the platform's
/// `PROJECT_RATIONALE_MAX`.
pub const RATIONALE_MAX: usize = 4000;
/// Shortest key prefix.
pub const KEY_PREFIX_MIN: usize = 2;
/// Longest key prefix.
pub const KEY_PREFIX_MAX: usize = 10;
/// Evidence `source` length cap.
pub const EVIDENCE_SOURCE_MAX: usize = 120;
/// Evidence `externalId` and `commitSha` length cap.
pub const EVIDENCE_EXTERNAL_ID_MAX: usize = 256;
/// Evidence `url` length cap.
pub const EVIDENCE_URL_MAX: usize = 2048;
/// Encoded evidence `metadata` cap, in bytes.
pub const EVIDENCE_METADATA_BYTES_MAX: usize = 16 * 1024;
/// Criteria one piece of evidence may link (the platform's
/// `WORK_ITEM_CRITERIA_MAX`).
pub const EVIDENCE_CRITERIA_MAX: usize = 20;
/// Largest page a list may ask for.
pub const PAGE_MAX: i64 = 100;
/// The page size when a list names none.
pub const PAGE_DEFAULT: i64 = 20;

/// `value`'s length in UTF-16 code units.
#[must_use]
pub fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

/// Refuse `value` shorter than `min` or longer than `max`.
pub fn text(field: &str, value: &str, min: usize, max: usize) -> Result<()> {
    let len = utf16_len(value);
    if len < min {
        return Err(ProjectError::over_limit(field, "too_short", min).into());
    }
    if len > max {
        return Err(ProjectError::over_limit(field, "too_long", max).into());
    }
    Ok(())
}

/// Refuse more than `max_count` items, then any item outside `1..=item_max`,
/// naming it `field.<index>` (the platform's path join).
pub fn list(field: &str, items: &[String], max_count: usize, item_max: usize) -> Result<()> {
    if items.len() > max_count {
        return Err(ProjectError::over_limit(field, "too_many", max_count).into());
    }
    items
        .iter()
        .enumerate()
        .try_for_each(|(index, item)| text(&format!("{field}.{index}"), item, 1, item_max))
}

/// `keyPrefix`: 2–10 characters, then `^[A-Z][A-Z0-9]*$`.
pub fn key_prefix(value: &str) -> Result<()> {
    text("keyPrefix", value, KEY_PREFIX_MIN, KEY_PREFIX_MAX)?;
    let mut bytes = value.bytes();
    let shaped = bytes.next().is_some_and(|b| b.is_ascii_uppercase())
        && bytes.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
    if shaped {
        Ok(())
    } else {
        Err(invariant("keyPrefix", "invalid_format"))
    }
}

/// A list page size: [`PAGE_DEFAULT`] when absent, else `1..=PAGE_MAX`.
pub fn page(limit: Option<i64>) -> Result<i64> {
    page_within(limit, PAGE_DEFAULT, PAGE_MAX)
}

/// A page size: `default` when absent, else `1..=max`.
pub fn page_within(limit: Option<i64>, default: i64, max: i64) -> Result<i64> {
    match limit {
        None => Ok(default),
        Some(n) if n < 1 => Err(ProjectError::over_limit("limit", "too_small", 1).into()),
        Some(n) if n > max => {
            Err(ProjectError::over_limit("limit", "too_large", max as usize).into())
        }
        Some(n) => Ok(n),
    }
}

/// An invariant-edge (`422`) `invalid_request` for `field`:
/// `"<field> is <reason>"`, details `{field, reason}`.
pub fn invariant(field: &str, reason: &str) -> Error {
    ProjectError::invalid_field(field, reason)
        .at(RequestEdge::Invariant, None)
        .into()
}

#[cfg(test)]
#[path = "tests/limits.rs"]
mod tests;
