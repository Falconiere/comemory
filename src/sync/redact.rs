//! Secret redaction scan before sync push (`2026-09-02-memory-sync-design.md`).

use std::sync::LazyLock;

use regex::Regex;
use rusqlite::Connection;

use crate::prelude::*;
use crate::store::sync_binding;

/// One compiled redaction rule from [`rules.toml`](rules.toml).
struct Rule {
    name: String,
    re: Regex,
}

/// Parsed rule table baked in at compile time.
static RULES: LazyLock<Vec<Rule>> = LazyLock::new(compile_rules);

fn compile_rules() -> Vec<Rule> {
    let raw = include_str!("rules.toml");
    let Ok(table) = raw.parse::<toml::Table>() else {
        tracing::error!("redact rules.toml failed to parse");
        return Vec::new();
    };
    let Some(toml::Value::Array(rules)) = table.get("rule") else {
        tracing::error!("redact rules.toml missing [[rule]] array");
        return Vec::new();
    };
    rules
        .iter()
        .filter_map(|entry| {
            let table = entry.as_table()?;
            let name = table.get("name")?.as_str()?.to_string();
            let pattern = table.get("pattern")?.as_str()?;
            match Regex::new(pattern) {
                Ok(re) => Some(Rule { name, re }),
                Err(e) => {
                    tracing::error!(rule = %name, %e, "invalid redact rule pattern");
                    None
                }
            }
        })
        .collect()
}

/// Scan `body` against the curated rule set; returns the first matching rule name.
pub fn scan(body: &str) -> Option<String> {
    for rule in RULES.iter() {
        if rule.re.is_match(body) {
            return Some(rule.name.clone());
        }
    }
    None
}

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
