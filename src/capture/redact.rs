//! Client redaction for capture / distillation: redact matches and attest.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Bump when a rule is added, removed, or tightened; the platform enforces a minimum.
pub const REDACTION_RULE_SET_VERSION: u32 = 1;

/// One compiled redaction rule from [`rules.toml`](rules.toml).
struct Rule {
    name: String,
    re: Regex,
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(compile_rules);

/// Refuse to proceed when the baked-in rule table failed to load.
pub fn ensure_rules_loaded() -> crate::prelude::Result<()> {
    if RULES.is_empty() {
        return Err(crate::prelude::Error::Other(
            "capture redaction rules failed to load — refusing to attest an empty rule set".into(),
        ));
    }
    Ok(())
}

fn compile_rules() -> Vec<Rule> {
    let raw = include_str!("rules.toml");
    let Ok(table) = raw.parse::<toml::Table>() else {
        tracing::error!("capture rules.toml failed to parse");
        return Vec::new();
    };
    let Some(toml::Value::Array(rules)) = table.get("rule") else {
        tracing::error!("capture rules.toml missing [[rule]] array");
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
                    tracing::error!(rule = %name, %e, "invalid capture redact rule");
                    None
                }
            }
        })
        .collect()
}

/// One finding in a redaction attestation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionFinding {
    /// Stable wire id (e.g. `aws-access-key-id`).
    pub rule: String,
    /// How many non-overlapping matches this rule contributed.
    pub count: u32,
}

/// Client attestation required on capture receipts and candidate batches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionAttestation {
    /// Client rule-set version (must be ≥ platform minimum).
    pub version: u32,
    /// Per-rule match counts (≤ 32 on the wire).
    pub findings: Vec<RedactionFinding>,
}

/// Result of redacting one free-text field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactOutcome {
    /// Text with matches replaced by `[REDACTED:<rule>]`.
    pub text: String,
    /// Per-rule counts from this field alone.
    pub findings: Vec<RedactionFinding>,
}

/// Redact `input` against the client rule set (first matching rule wins per span).
pub fn redact_text(input: &str) -> RedactOutcome {
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;

    while cursor < input.len() {
        let rest = &input[cursor..];
        let mut best: Option<(usize, usize, &str)> = None;
        for rule in RULES.iter() {
            if let Some(m) = rule.re.find(rest) {
                let start = m.start();
                let end = m.end();
                let take = match best {
                    None => true,
                    Some((b_start, _, _)) => start < b_start,
                };
                if take {
                    best = Some((start, end, rule.name.as_str()));
                }
            }
        }
        let Some((start, end, name)) = best else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        out.push_str("[REDACTED:");
        out.push_str(name);
        out.push(']');
        *counts.entry(name.to_string()).or_default() += 1;
        cursor += end;
    }

    let findings = counts
        .into_iter()
        .map(|(rule, count)| RedactionFinding { rule, count })
        .collect();
    RedactOutcome {
        text: out,
        findings,
    }
}

/// Merge per-field finding maps into one attestation (caps at 32 findings).
pub fn merge_findings(parts: &[Vec<RedactionFinding>]) -> RedactionAttestation {
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for part in parts {
        for finding in part {
            *counts.entry(finding.rule.clone()).or_default() += finding.count;
        }
    }
    let findings: Vec<RedactionFinding> = counts
        .into_iter()
        .map(|(rule, count)| RedactionFinding { rule, count })
        .take(32)
        .collect();
    RedactionAttestation {
        version: REDACTION_RULE_SET_VERSION,
        findings,
    }
}

#[cfg(test)]
#[path = "tests/redact.rs"]
mod tests;
