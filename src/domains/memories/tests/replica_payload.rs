#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `MemoryPayloadV1` — what a memory is on the wire, and what its digest
//! covers. Built from a real saved record rather than a hand-made struct, so
//! a frontmatter field that stops being replicated fails here.

use comemory::config::{Config, Paths};
use comemory::domains::memories::replica_payload::MemoryPayloadV1;
use comemory::domains::memories::{self, Kind, MemoryStore};
use comemory::store::connection;
use comemory::utilities::context::Ctx;

const BODY: &str = "comemory keeps a durable, searchable memory of the \
    decisions, bugs and conventions a codebase accumulates.";

/// Save one real memory and hand back its loaded record.
fn saved_record(tags: &[&str]) -> (tempfile::TempDir, memories::MemoryRecord) {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let id = memories::save::run(
        &mut ctx,
        memories::save::Request {
            body: BODY.to_string(),
            title: None,
            kind: Kind::Decision,
            repo: "Falconiere/comemory".to_string(),
            tags: tags.iter().map(|t| (*t).to_string()).collect(),
            author: "tester".to_string(),
            quality: 4,
            supersedes: Vec::new(),
            vector: None,
            ref_file: Vec::new(),
            ref_symbol: Vec::new(),
        },
        false,
        None,
    )
    .expect("save")
    .id;
    let record = MemoryStore::new(paths.clone()).load(&id).expect("load");
    (dir, record)
}

#[test]
fn a_payload_round_trips_through_its_canonical_bytes() {
    let (_dir, record) = saved_record(&["sync", "journal"]);
    let payload = MemoryPayloadV1::from_record(&record).expect("payload");
    let (bytes, digest) = payload.canonical().expect("canonical");

    let decoded = MemoryPayloadV1::decode(&bytes).expect("decode");
    assert_eq!(decoded, payload);
    assert_eq!(decoded.canonical().expect("recanonical").1, digest);
    assert_eq!(decoded.body, record.body);
    assert_eq!(decoded.id, record.frontmatter.id);
    assert_eq!(
        decoded.tags,
        vec!["sync".to_string(), "journal".to_string()],
        "tag order is payload content, preserved exactly as saved"
    );
    assert_eq!(
        decoded.created_at().expect("created"),
        record.frontmatter.created
    );
}

#[test]
fn the_payload_carries_no_author() {
    let (_dir, record) = saved_record(&["sync"]);
    assert_eq!(record.frontmatter.author, "tester");
    let (bytes, _) = MemoryPayloadV1::from_record(&record)
        .expect("payload")
        .canonical()
        .expect("canonical");
    assert!(
        !bytes.contains("tester"),
        "the accepting side stamps the author; the payload must not carry it: {bytes}"
    );
}

#[test]
fn changing_only_the_tags_changes_the_digest() {
    let (_dir, record) = saved_record(&["sync"]);
    let before = MemoryPayloadV1::from_record(&record).expect("payload");
    let mut after = before.clone();
    after.tags.push("journal".to_string());

    assert_eq!(before.body, after.body);
    assert_eq!(before.content_hash, after.content_hash);
    assert_ne!(
        before.canonical().expect("before").1,
        after.canonical().expect("after").1,
        "a tags-only edit is a new revision"
    );
}

#[test]
fn an_unknown_field_is_refused_rather_than_silently_dropped() {
    let (_dir, record) = saved_record(&["sync"]);
    let (bytes, _) = MemoryPayloadV1::from_record(&record)
        .expect("payload")
        .canonical()
        .expect("canonical");
    let widened = bytes.replacen('{', r#"{"future_field":true,"#, 1);

    let decoded = MemoryPayloadV1::decode(&widened);
    assert!(
        decoded.is_err(),
        "a payload from a newer schema must be refused, not half-applied"
    );
}
