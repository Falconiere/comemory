#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The distill use case over the committed real transcript: what a run needs
//! before it may POST, and what a dry run produces without one.

use std::path::PathBuf;

use comemory::domains::capture::distill::{DistillRequest, build_batch, requires_credentials, run};

fn fixture() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/claude-code-session-saves.jsonl"
    ))
}

fn request(dry_run: bool) -> DistillRequest {
    DistillRequest {
        session_id: "8e9f54e3-a984-46ab-8403-135ee920cbca".into(),
        transcript: fixture(),
        dry_run,
        api_url: None,
    }
}

#[test]
fn a_dry_run_needs_no_credentials_and_every_other_run_does() {
    assert!(
        !requires_credentials(&request(true)),
        "a dry run must stay usable on a machine that never logged in"
    );
    assert!(
        requires_credentials(&request(false)),
        "a run that POSTs needs the org key"
    );
}

#[test]
fn a_dry_run_builds_the_real_fixture_batch_without_auth_and_without_posting() {
    let req = request(true);
    let report = run(None, &req).expect("a dry run must not require an AuthFile");

    assert!(report.dry_run);
    assert!(report.response.is_none(), "a dry run posts nothing");
    assert_eq!(report.session_id, req.session_id);

    let (extracted, batch) = build_batch(&fixture()).expect("batch");
    assert_eq!(
        report.extracted, extracted,
        "the use case reports what the extractor recovered"
    );
    assert_eq!(report.batch.extractor, "claude-code-explicit-save");
    assert_eq!(report.batch.extractor_version, 1);
    assert_eq!(report.batch.redaction.version, 1);

    // Identity, not just the count: a fixture change must name which claim moved.
    let identity: Vec<(&str, &str)> = report
        .batch
        .candidates
        .iter()
        .map(|c| (c.kind.as_str(), c.title.as_str()))
        .collect();
    assert_eq!(
        identity,
        vec![
            ("fact", "comemory.io feature gap map (2026-09-10)"),
            ("fact", "Feature-gap issues filed on GitHub (#45-#59)"),
            (
                "fact",
                "Engine sync routes landed in 0.21.0; author absent from HTTP API"
            ),
            (
                "fact",
                "comemory.io repo needs bun installed; local console suite fails on macOS+Node 26"
            ),
            (
                "fact",
                "Console tests pass locally with --localstorage-file; docs ingest impossible from a Worker"
            ),
            (
                "pattern",
                "Gate every merge on threads+label+checks, not just the bot label"
            ),
        ],
        "the six real saves, in transcript order, with their product kinds"
    );
    assert_eq!(
        report.batch.candidates.len(),
        batch.candidates.len(),
        "the use case posts exactly the batch build_batch produced"
    );
}

#[test]
fn a_live_run_without_credentials_is_a_usage_error_naming_the_login_command() {
    let err = run(None, &request(false)).expect_err("a POST needs credentials");
    assert!(
        err.to_string().contains("not logged in"),
        "the message must point at `comemory auth login`: {err}"
    );
}
