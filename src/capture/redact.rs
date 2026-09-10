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

/// Compiled rules, or a load error retained for [`ensure_rules_loaded`].
static RULES: LazyLock<Result<Vec<Rule>, String>> = LazyLock::new(compile_rules);

/// Refuse to proceed when the baked-in rule table failed to load or is empty.
pub fn ensure_rules_loaded() -> crate::prelude::Result<()> {
    match &*RULES {
        Ok(rules) if !rules.is_empty() => Ok(()),
        Ok(_) => Err(crate::prelude::Error::Other(
            "capture redaction rules loaded empty — refusing to attest an empty rule set".into(),
        )),
        Err(e) => Err(crate::prelude::Error::Other(format!(
            "capture redaction rules failed to load: {e}"
        ))),
    }
}

fn compiled_rules() -> &'static [Rule] {
    match &*RULES {
        Ok(rules) => rules.as_slice(),
        Err(_) => &[],
    }
}

fn compile_rules() -> Result<Vec<Rule>, String> {
    // Co-located with this module: `src/capture/rules.toml` (not repo-root).
    let raw = include_str!("rules.toml");
    let table = raw
        .parse::<toml::Table>()
        .map_err(|e| format!("rules.toml parse: {e}"))?;
    let Some(toml::Value::Array(rules)) = table.get("rule") else {
        return Err("rules.toml missing [[rule]] array".into());
    };
    let mut out = Vec::with_capacity(rules.len());
    for entry in rules {
        let Some(table) = entry.as_table() else {
            return Err("rules.toml [[rule]] entry is not a table".into());
        };
        let name = table
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "rules.toml [[rule]] missing name".to_string())?
            .to_string();
        let pattern = table
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("rules.toml rule {name} missing pattern"))?;
        let re = Regex::new(pattern).map_err(|e| format!("rule {name}: {e}"))?;
        out.push(Rule { name, re });
    }
    Ok(out)
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
        for rule in compiled_rules() {
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
