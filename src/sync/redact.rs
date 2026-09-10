//! Secret redaction scan before sync push and session capture.
//!
//! Rule ids for the format-anchored set match the platform Slice 3 catalogue
//! (`docs/toolu/specs/2026-09-10-cli-session-capture-design.md`). The client
//! also runs `generic-high-entropy`, which the server deliberately omits.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::prelude::*;
use crate::store::Connection;
use crate::store::sync_binding;

/// Client rule-set version attested on every capture receipt.
pub const RULE_SET_VERSION: u32 = 1;

/// One finding: a rule id and how many times it matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// Rule id (platform catalogue name, or `generic-high-entropy`).
    pub rule: String,
    /// Match count in the scanned text.
    pub count: u32,
}

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

/// Aggregate every rule hit in `body` into `{rule, count}` findings.
pub fn findings(body: &str) -> Vec<Finding> {
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for rule in RULES.iter() {
        let n = u32::try_from(rule.re.find_iter(body).count()).unwrap_or(u32::MAX);
        if n > 0 {
            counts.insert(rule.name.clone(), n);
        }
    }
    counts
        .into_iter()
        .map(|(rule, count)| Finding { rule, count })
        .collect()
}

/// Replace every rule match in `body` with `[REDACTED:<rule>]` and return
/// the redacted text plus aggregated findings (counts from the original).
pub fn redact(body: &str) -> (String, Vec<Finding>) {
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    let mut out = body.to_string();
    for rule in RULES.iter() {
        // Count against the original body so replacements cannot inflate hits.
        let n = u32::try_from(rule.re.find_iter(body).count()).unwrap_or(u32::MAX);
        if n > 0 {
            counts.insert(rule.name.clone(), n);
        }
        let replacement = format!("[REDACTED:{}]", rule.name);
        out = rule.re.replace_all(&out, replacement.as_str()).into_owned();
    }
    let found = counts
        .into_iter()
        .map(|(rule, count)| Finding { rule, count })
        .collect();
    (out, found)
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
