#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

use std::path::Path;

use comemory::domains::architecture::current::parse_body;
use comemory::domains::architecture::model::Source;

fn fixture_json() -> String {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/common/fixtures/architecture/valid.json");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn a_model_is_read_out_of_the_fenced_body_a_save_writes() {
    let body = format!(
        "Architecture model — comemory\n\nTwo components.\n\n```json\n{}\n```\n",
        fixture_json()
    );
    let model = parse_body(&body).unwrap();
    assert_eq!(model.repo, "comemory");
    assert_eq!(model.source, Source::Agent);
    assert_eq!(model.components.len(), 3);
}

#[test]
fn a_bare_json_body_is_still_read() {
    let model = parse_body(&fixture_json()).unwrap();
    assert_eq!(model.components.len(), 3);
}

#[test]
fn a_body_with_no_model_is_a_json_error_not_a_panic() {
    let err = parse_body("Architecture model — comemory\n\nnothing here\n").unwrap_err();
    assert!(err.to_string().starts_with("json:"), "{err}");
}
