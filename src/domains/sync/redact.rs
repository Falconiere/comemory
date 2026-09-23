//! Secret scan on the sync push path, plus the per-memory override.
//!
//! The rule set itself lives in [`crate::utilities::secret_scan`] — shared,
//! because the documents capability scans extracted text through the same
//! rules and neither capability may depend on the other. Re-exported here so
//! every existing caller keeps its import.

pub use crate::utilities::secret_scan::{Finding, RULE_SET_VERSION, findings, redact, scan};

use crate::prelude::*;
use crate::store::Connection;
use crate::store::sync_binding;

/// Scan `body` unless `sync_binding` records an explicit `--allow-secret` override.
pub fn scan_with_override(
    conn: &Connection,
    memory_id: &str,
    body: &str,
) -> Result<Option<String>> {
    if sync_binding::has_secret_override(conn, memory_id)? {
        return Ok(None);
    }
    Ok(scan(body))
}

#[cfg(test)]
#[path = "tests/redact.rs"]
mod tests;
