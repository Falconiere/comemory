//! Tests for the pure vector-payload decoding split out of `cli` by #166.
//!
//! The `--vector-stdin` 8 MiB cap and the mutually-exclusive-flag rule need a
//! real process stdin, so they stay covered by the real-binary `cli__save*` /
//! `cli__search*` suites; this file pins the decoding boundaries the split
//! moved.

use comemory::utilities::embedding_input::{parse_csv, parse_payload};

#[test]
fn parse_csv_trims_each_component() {
    assert_eq!(
        parse_csv("1.0, 2.5 ,-3").expect("csv parses"),
        vec![1.0_f32, 2.5, -3.0]
    );
}

#[test]
fn parse_csv_reports_the_offending_value() {
    let err = parse_csv("1.0,x").expect_err("non-float is rejected");
    assert!(
        err.to_string().contains("--vector parse:"),
        "unexpected message: {err}"
    );
}

#[test]
fn parse_payload_accepts_surrounding_whitespace() {
    assert_eq!(
        parse_payload("  {\"embedding\":[0.5,0.25]}\n").expect("payload parses"),
        vec![0.5_f32, 0.25]
    );
}

#[test]
fn parse_payload_rejects_an_unknown_field() {
    let err = parse_payload("{\"embedding\":[1.0],\"extra\":1}")
        .expect_err("deny_unknown_fields rejects the stray key");
    assert!(
        err.to_string().contains("extra"),
        "unexpected message: {err}"
    );
}

#[test]
fn parse_payload_rejects_a_missing_embedding_field() {
    let err = parse_payload("{}").expect_err("the embedding field is required");
    assert!(
        err.to_string().contains("embedding"),
        "unexpected message: {err}"
    );
}
