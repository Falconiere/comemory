//! Comma-separated id lists shared by the flag surfaces and the command cores.
//!
//! [`csv_unique`] is the de-duplication rule every CSV flag uses; the two
//! parsers add the id validation their flag needs. The *policy* each parser
//! delegates to stays with its owner — memory-id shape is
//! [`crate::domains::memories::id::is_valid_memory_id`] — so this file only splits,
//! trims, de-duplicates, and reports (#166).

use crate::prelude::*;

/// Split a comma-separated flag value into trimmed, non-empty, de-duplicated
/// entries preserving first-mention order. Shared by `save` (`--tags`,
/// `--supersedes`) and `feedback` (`--used`, `--irrelevant`) so every CSV
/// flag tolerates `a,,a , b` style input identically.
pub(crate) fn csv_unique(raw: &str) -> Vec<String> {
    if raw.is_empty() {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    raw.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty() && seen.insert(t.clone()))
        .collect()
}

/// Parse a CSV of memory ids via [`csv_unique`] and validate every entry
/// against [`crate::domains::memories::id::is_valid_memory_id`], naming the offending
/// `flag` in the error. Shared by `save --supersedes` and the `feedback`
/// id flags so malformed ids are rejected identically everywhere.
pub(crate) fn parse_id_csv(raw: &str, flag: &str) -> Result<Vec<String>> {
    let ids = csv_unique(raw);
    for entry in &ids {
        if !crate::domains::memories::id::is_valid_memory_id(entry) {
            return Err(Error::Config(format!(
                "{flag}: invalid memory id `{entry}` (expected 8 lowercase hex chars)"
            )));
        }
    }
    Ok(ids)
}

/// Parse a CSV of code-symbol ids via [`csv_unique`] and validate every
/// entry as a positive integer (`code_symbols.id` is an INTEGER rowid; 0
/// and negatives never name a row), naming the offending `flag` in the
/// error. De-duplicates again on the parsed value so `07,7` cannot
/// double-count a counter. Sibling of [`parse_id_csv`] for the
/// `feedback --used-code` / `--irrelevant-code` flags.
pub(crate) fn parse_symbol_id_csv(raw: &str, flag: &str) -> Result<Vec<i64>> {
    let mut ids: Vec<i64> = Vec::new();
    for entry in csv_unique(raw) {
        let bad = || {
            Error::Config(format!(
                "{flag}: invalid symbol id `{entry}` (expected a positive integer)"
            ))
        };
        let id: i64 = entry.parse().map_err(|_| bad())?;
        if id <= 0 {
            return Err(bad());
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

#[cfg(test)]
#[path = "tests/id_list.rs"]
mod tests;
