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

/// Split `raw` via [`csv_unique`] and convert every entry with `parse`,
/// naming the offending `flag`, the kind of id (`noun`) and what was
/// `expected` in the error. De-duplicates again on the parsed value so two
/// spellings of one id (`07,7`) cannot double-count. The one split, validate
/// and de-duplicate loop behind both parsers below.
fn parse_entries<T: PartialEq>(
    raw: &str,
    flag: &str,
    noun: &str,
    expected: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<Vec<T>> {
    let mut ids = Vec::new();
    for entry in csv_unique(raw) {
        let Some(id) = parse(&entry) else {
            return Err(Error::Config(format!(
                "{flag}: invalid {noun} `{entry}` (expected {expected})"
            )));
        };
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Parse a CSV of memory ids via [`csv_unique`] and validate every entry
/// against [`crate::domains::memories::id::is_valid_memory_id`], naming the offending
/// `flag` in the error. Shared by `save --supersedes` and the `feedback`
/// id flags so malformed ids are rejected identically everywhere.
pub(crate) fn parse_id_csv(raw: &str, flag: &str) -> Result<Vec<String>> {
    parse_entries(raw, flag, "memory id", "8 lowercase hex chars", |entry| {
        crate::domains::memories::id::is_valid_memory_id(entry).then(|| entry.to_string())
    })
}

/// Parse a CSV of code-symbol ids via [`csv_unique`] and validate every
/// entry as a positive integer (`code_symbols.id` is an INTEGER rowid; 0
/// and negatives never name a row), naming the offending `flag` in the
/// error. De-duplicates again on the parsed value so `07,7` cannot
/// double-count a counter. Sibling of [`parse_id_csv`] for the
/// `feedback --used-code` / `--irrelevant-code` flags.
pub(crate) fn parse_symbol_id_csv(raw: &str, flag: &str) -> Result<Vec<i64>> {
    parse_entries(raw, flag, "symbol id", "a positive integer", |entry| {
        entry.parse::<i64>().ok().filter(|id| *id > 0)
    })
}

#[cfg(test)]
#[path = "tests/id_list.rs"]
mod tests;
