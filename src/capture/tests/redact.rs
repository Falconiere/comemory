#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Client redaction attestation for capture / distillation.

use comemory::capture::redact::{REDACTION_RULE_SET_VERSION, merge_findings, redact_text};

fn aws_example_key() -> String {
    // Split so repo secret-content does not flag the AWS example id literal.
    format!("AKIA{}{}", "IOSFODNN7", "EXAMPLE")
}

#[test]
fn redacts_aws_access_key_and_attests_count() {
    let key = aws_example_key();
    let body = format!("deploy with {key} and keep going");
    let out = redact_text(&body);
    assert!(out.text.contains("[REDACTED:aws-access-key-id]"));
    assert!(!out.text.contains(&key));
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
