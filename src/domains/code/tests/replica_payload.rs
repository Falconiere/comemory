#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`crate::domains::code::replica_payload`] — the wire shape of one code
//! generation, its content-derived identity, and the round trip back into a
//! projection.
//!
//! The projections here come from the real generation planner running over a
//! real indexed repository, so the payload is exercised against the shape the
//! engine actually produces rather than a hand-written one.

use crate::test_common::git_sample;

use comemory::config::{Config, Paths};
use comemory::domains::code::index_code::{IndexMode, Request};
use comemory::store::connection;
use comemory::store::remote_code::Projection;
use comemory::utilities::context::Ctx;
use tempfile::tempdir;

use crate::domains::code::replica_payload::{CodeGenerationV1, ID_LEN};

const REPO: &str = "sample";

/// A projection read off a real indexed repository.
fn real_projection() -> (Projection, String) {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo_path = git_sample::build_sample_repo(workspace.path());
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        crate::domains::code::index_code::run(
            &mut ctx,
            Request {
                repo: REPO.into(),
                path: repo_path.to_str().expect("utf8").to_string(),
                mode: IndexMode::Incremental,
            },
        )
        .expect("index run");
    }
    let planned = crate::domains::code::generation::plan(&conn, REPO)
        .expect("plan")
        .expect("row");
    (planned.projection, planned.generation.head)
}

fn payload() -> CodeGenerationV1 {
    let (projection, head) = real_projection();
    let base = CodeGenerationV1::new("", None, &head, Some("mined-1"), &projection);
    let id = base.mint_id().expect("mint");
    base.with_id(&id)
}

#[test]
fn a_payload_round_trips_through_its_canonical_bytes() {
    let original = payload();

    let (bytes, _) = original.canonical().expect("canonical");
    let decoded = CodeGenerationV1::decode(&bytes).expect("decode");

    assert_eq!(decoded, original, "the wire shape loses nothing");
}

#[test]
fn a_payload_owns_the_id_its_contents_earn() {
    let original = payload();

    assert_eq!(original.generation_id.len(), ID_LEN);
    assert!(
        original.owns_its_id().expect("owns"),
        "a freshly minted id is derived from these exact contents"
    );
}

#[test]
fn an_edited_payload_no_longer_owns_its_id() {
    let mut tampered = payload();
    // One symbol renamed, the id left claiming the original contents — the
    // same identity break a memory body edit produces.
    if let Some(symbol) = tampered.symbols.first_mut() {
        symbol.symbol = format!("{}_tampered", symbol.symbol);
    } else {
        tampered.head = "a-different-head".to_string();
    }

    assert!(
        !tampered.owns_its_id().expect("owns"),
        "the id is content-derived, so edited contents cannot keep it"
    );
}

#[test]
fn a_different_head_over_the_same_files_is_a_different_generation() {
    let (projection, head) = real_projection();
    let first = CodeGenerationV1::new("", None, &head, None, &projection);
    let moved = CodeGenerationV1::new("", None, "a-later-commit", None, &projection);

    assert_ne!(
        first.mint_id().expect("mint"),
        moved.mint_id().expect("mint"),
        "a peer must be able to tell 'nothing changed' from 'same files, new head'"
    );
}

#[test]
fn the_projection_survives_the_round_trip_unchanged() {
    let (projection, head) = real_projection();
    let wire = CodeGenerationV1::new("", None, &head, None, &projection);

    let (bytes, _) = wire.canonical().expect("canonical");
    let back = CodeGenerationV1::decode(&bytes)
        .expect("decode")
        .projection();

    assert_eq!(back, projection, "what was sent is what lands");
}

#[test]
fn the_payload_has_nowhere_to_put_source_text() {
    let original = payload();
    let (bytes, _) = original.canonical().expect("canonical");

    // The fixture repo's source contains this; the projection must not.
    assert!(
        !bytes.contains("fn main"),
        "a snippet-free payload cannot carry a function body: {bytes}"
    );
    assert!(
        !bytes.contains("snippet"),
        "no field named snippet exists on the wire: {bytes}"
    );
}

#[test]
fn a_payload_with_an_unknown_field_is_refused_rather_than_half_read() {
    let original = payload();
    let (bytes, _) = original.canonical().expect("canonical");
    let mut value: serde_json::Value = serde_json::from_str(&bytes).expect("json");
    value["snippet"] = serde_json::json!("fn main() { }");
    let tampered = serde_json::to_string(&value).expect("string");

    let refused = CodeGenerationV1::decode(&tampered).expect_err("unknown field");

    assert!(
        matches!(refused, comemory::errors::Error::BadRequest(_)),
        "got {refused:?}"
    );
}
