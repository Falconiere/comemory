#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Client redaction attestation for capture / distillation.

use comemory::capture::redact::{REDACTION_RULE_SET_VERSION, merge_findings, redact_text};

#[test]
fn redacts_aws_access_key_and_attests_count() {
    let body = "deploy with AKIAIOSFODNN7EXAMPLE and keep going";
    let out = redact_text(body);
    assert!(out.text.contains("[REDACTED:aws-access-key-id]"));
    assert!(!out.text.contains("AKIAIOSFODNN7EXAMPLE"));
    assert_eq!(out.findings.len(), 1);
    assert_eq!(out.findings[0].rule, "aws-access-key-id");
    assert_eq!(out.findings[0].count, 1);

    let att = merge_findings(&[out.findings]);
    assert_eq!(att.version, REDACTION_RULE_SET_VERSION);
    assert_eq!(att.findings[0].rule, "aws-access-key-id");
}

#[test]
fn leaves_clean_prose_unchanged() {
    let body = "Chose Turso over Postgres for embedded local dev.";
    let out = redact_text(body);
    assert_eq!(out.text, body);
    assert!(out.findings.is_empty());
}
