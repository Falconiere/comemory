#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Accepting a document revision against a real migrated database, over this
//! repository's own documentation run through the real extractor: the one
//! instant a revision becomes visible, and the ways an altered one does not.

use crate::domains::documents::document::extract::extract;
use crate::domains::documents::document::{DocumentFormat, ExtractedDocument};
use crate::domains::documents::replica_payload::{
    ChunkWire, DOCUMENT_ENTITY_KIND, DOCUMENT_PAYLOAD_VERSION, DocumentRevisionV1, LinkWire,
};
use crate::domains::documents::share;
use crate::domains::sync::replica::accept;
use crate::domains::sync::replica::contract::{Disposition, Operation};
use crate::store::replica_journal::ReplicaOp;
use crate::store::{remote_document, replica_read};

use crate::domains::sync::replica::test_support as support;

use support::{Home, envelope};

const REPO: &str = "Falconiere/comemory";
const GUIDE: &str = "docs/guides/cloud-sync.md";

/// A real repository document, extracted by the shipped extractor.
fn extracted(path: &str) -> ExtractedDocument {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    let body = std::fs::read(&file).unwrap_or_else(|e| panic!("read real {path}: {e}"));
    extract(DocumentFormat::Markdown, &body, "doc").expect("real extraction")
}

/// The payload a peer would send for `path` at `hash`.
fn payload(path: &str, hash: &str) -> DocumentRevisionV1 {
    let doc = extracted(path);
    DocumentRevisionV1 {
        shared_id: share::shared_id(REPO, path),
        repo: REPO.to_string(),
        path: path.to_string(),
        title: doc.title.clone(),
        format: "markdown".to_string(),
        revision_hash: hash.to_string(),
        chunks: doc
            .chunks
            .iter()
            .map(|c| ChunkWire {
                ordinal: c.ordinal as i64,
                heading_path: c.heading_path.join(" > "),
                char_start: c.char_range.0 as i64,
                char_end: c.char_range.1 as i64,
                line_start: c.line_range.0 as i64,
                line_end: c.line_range.1 as i64,
                simhash: c.simhash as i64,
                text: c.text.clone(),
            })
            .collect(),
        links: vec![LinkWire {
            ordinal: 0,
            target: "docs/guides/http-api.md".to_string(),
        }],
    }
}

/// An upsert operation carrying `payload`.
fn upsert(operation_id: &str, payload: &DocumentRevisionV1) -> Operation {
    let (bytes, digest) = payload.canonical().expect("canonical");
    Operation {
        operation_id: operation_id.to_string(),
        entity_kind: DOCUMENT_ENTITY_KIND.to_string(),
        entity_key: payload.shared_id.clone(),
        op: ReplicaOp::Upsert,
        schema_version: DOCUMENT_PAYLOAD_VERSION,
        payload_digest: Some(digest),
        payload: Some(serde_json::from_str(&bytes).expect("json")),
        observed_sequence: None,
        repository: Some(REPO.to_string()),
        vector: None,
    }
}

/// A tombstone for `shared_id`.
fn tombstone(operation_id: &str, shared_id: &str) -> Operation {
    Operation {
        operation_id: operation_id.to_string(),
        entity_kind: DOCUMENT_ENTITY_KIND.to_string(),
        entity_key: shared_id.to_string(),
        op: ReplicaOp::Tombstone,
        schema_version: DOCUMENT_PAYLOAD_VERSION,
        payload_digest: None,
        payload: None,
        observed_sequence: None,
        repository: Some(REPO.to_string()),
        vector: None,
    }
}

fn fts_rows(conn: &rusqlite::Connection, shared_id: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM remote_document_fts WHERE shared_id = ?1",
        [shared_id],
        |r| r.get(0),
    )
    .expect("count fts")
}

#[test]
fn an_accepted_revision_becomes_searchable_whole() {
    let mut home = Home::new();
    let sent = payload(GUIDE, &"a".repeat(64));
    let mut ctx = home.ctx();

    let response = accept::run(
        &mut ctx,
        envelope(vec![upsert("op-20260923-doc00001", &sent)]),
    )
    .expect("accept");

    assert_eq!(response.results[0].disposition, Disposition::Accepted);
    let held = remote_document::revision(&home.conn, REPO, &sent.shared_id)
        .expect("read")
        .expect("the revision this machine now holds");
    assert_eq!(held.path, GUIDE);
    assert_eq!(held.revision_hash, "a".repeat(64));
    assert_eq!(held.chunk_count, sent.chunks.len() as i64);
    assert!(held.chunk_count > 1, "the real document has passages");
    assert_eq!(
        remote_document::chunks(&home.conn, REPO, &sent.shared_id)
            .expect("chunks")
            .len(),
        sent.chunks.len(),
        "every passage landed with the header"
    );
    assert_eq!(
        remote_document::links(&home.conn, REPO, &sent.shared_id)
            .expect("links")
            .len(),
        1,
        "and the link it carries"
    );
    assert_eq!(
        fts_rows(&home.conn, &sent.shared_id),
        sent.chunks.len() as i64,
        "searchable at the same instant, not after a later pass"
    );
    assert_eq!(
        replica_read::head(&home.conn).expect("head"),
        1,
        "and it earned exactly one feed position"
    );
}

#[test]
fn a_later_revision_replaces_the_earlier_text() {
    let mut home = Home::new();
    let first = payload(GUIDE, &"a".repeat(64));
    let mut ctx = home.ctx();
    accept::run(
        &mut ctx,
        envelope(vec![upsert("op-20260923-doc00002", &first)]),
    )
    .expect("first");

    // The sender edited the document down to its first passage.
    let mut second = first.clone();
    second.revision_hash = "b".repeat(64);
    second.chunks.truncate(1);
    second.links.clear();
    let mut ctx = home.ctx();
    let response = accept::run(
        &mut ctx,
        envelope(vec![upsert("op-20260923-doc00003", &second)]),
    )
    .expect("second");

    assert_eq!(response.results[0].disposition, Disposition::Accepted);
    assert!(
        first.chunks.len() > 1,
        "the first revision had more passages, so the count below is a real drop"
    );
    let held = remote_document::revision(&home.conn, REPO, &first.shared_id)
        .expect("read")
        .expect("row");
    assert_eq!(held.revision_hash, "b".repeat(64), "the later one wins");
    assert_eq!(held.chunk_count, 1);
    assert_eq!(
        fts_rows(&home.conn, &first.shared_id),
        1,
        "the earlier text is gone from the index"
    );
    assert_eq!(
        replica_read::head(&home.conn).expect("head"),
        2,
        "two revisions, two positions"
    );
}

#[test]
fn re_accepting_the_revision_already_held_changes_no_row() {
    let mut home = Home::new();
    let sent = payload(GUIDE, &"a".repeat(64));
    let mut ctx = home.ctx();
    accept::run(
        &mut ctx,
        envelope(vec![upsert("op-20260923-doc00004", &sent)]),
    )
    .expect("first");
    let before = remote_document::chunks(&home.conn, REPO, &sent.shared_id).expect("chunks");
    let head_before = replica_read::head(&home.conn).expect("head");

    // The same revision under a NEW operation id: the receipt cannot answer
    // this one, so the write path runs again. It must be a no-op in effect.
    let mut ctx = home.ctx();
    let response = accept::run(
        &mut ctx,
        envelope(vec![upsert("op-20260923-doc00005", &sent)]),
    )
    .expect("again");

    assert_eq!(response.results[0].disposition, Disposition::Accepted);
    assert_eq!(
        remote_document::chunks(&home.conn, REPO, &sent.shared_id).expect("chunks"),
        before,
        "the same text, neither duplicated nor reordered"
    );
    assert_eq!(
        remote_document::revision(&home.conn, REPO, &sent.shared_id)
            .expect("read")
            .expect("row")
            .revision_hash,
        "a".repeat(64)
    );
    assert_eq!(
        fts_rows(&home.conn, &sent.shared_id),
        before.len() as i64,
        "and the index is not doubled"
    );
    assert_eq!(
        replica_read::head(&home.conn).expect("head"),
        head_before + 1,
        "a distinct operation still earns its own position"
    );
}

#[test]
fn a_replayed_operation_id_reads_back_its_receipt() {
    let mut home = Home::new();
    let sent = payload(GUIDE, &"a".repeat(64));
    let operation = upsert("op-20260923-doc00006", &sent);
    let mut ctx = home.ctx();
    accept::run(&mut ctx, envelope(vec![operation.clone()])).expect("first");
    let head_before = replica_read::head(&home.conn).expect("head");

    let mut ctx = home.ctx();
    let replay = accept::run(&mut ctx, envelope(vec![operation])).expect("replay");

    assert_eq!(
        replay.results[0].disposition,
        Disposition::Duplicate,
        "the receipt answers it; the write path never runs again"
    );
    assert_eq!(
        replica_read::head(&home.conn).expect("head"),
        head_before,
        "so nothing further is journalled"
    );
}

#[test]
fn a_revision_whose_id_was_altered_in_flight_is_refused() {
    let mut home = Home::new();
    let mut sent = payload(GUIDE, &"a".repeat(64));
    // The key and the payload still agree, but neither is the id this repo and
    // path earn — the shape an attacker filing text under another document's
    // name would produce.
    sent.shared_id = share::shared_id(REPO, "docs/guides/http-api.md");
    let mut ctx = home.ctx();

    let response = accept::run(
        &mut ctx,
        envelope(vec![upsert("op-20260923-doc00007", &sent)]),
    )
    .expect("accept runs");

    assert_eq!(
        response.results[0].disposition,
        Disposition::RejectedInvalid
    );
    assert_eq!(
        remote_document::revision(&home.conn, REPO, &sent.shared_id).expect("read"),
        None,
        "nothing was written"
    );
    assert_eq!(
        replica_read::head(&home.conn).expect("head"),
        0,
        "and no position was earned"
    );
}

#[test]
fn a_revision_with_a_gap_in_its_passages_is_refused_and_the_prior_one_stands() {
    let mut home = Home::new();
    let good = payload(GUIDE, &"a".repeat(64));
    let mut ctx = home.ctx();
    accept::run(
        &mut ctx,
        envelope(vec![upsert("op-20260923-doc00008", &good)]),
    )
    .expect("first");

    // A part went missing in flight: the ordinals no longer run from zero.
    let mut broken = good.clone();
    broken.revision_hash = "b".repeat(64);
    broken.chunks.remove(0);
    broken.links.clear();
    let mut ctx = home.ctx();
    let response = accept::run(
        &mut ctx,
        envelope(vec![upsert("op-20260923-doc00009", &broken)]),
    )
    .expect("accept runs");

    assert_eq!(
        response.results[0].disposition,
        Disposition::RejectedInvalid
    );
    assert_eq!(
        remote_document::revision(&home.conn, REPO, &good.shared_id)
            .expect("read")
            .expect("row")
            .revision_hash,
        "a".repeat(64),
        "the revision this machine had is still the one it has"
    );
    assert_eq!(
        fts_rows(&home.conn, &good.shared_id),
        good.chunks.len() as i64,
        "and the new text never reached the index"
    );
}

#[test]
fn a_tombstone_forgets_that_one_document() {
    let mut home = Home::new();
    let kept = payload("docs/guides/http-api.md", &"c".repeat(64));
    let gone = payload(GUIDE, &"a".repeat(64));
    let mut ctx = home.ctx();
    accept::run(
        &mut ctx,
        envelope(vec![
            upsert("op-20260923-doc00010", &kept),
            upsert("op-20260923-doc00011", &gone),
        ]),
    )
    .expect("two revisions");
    assert!(
        fts_rows(&home.conn, &gone.shared_id) > 0,
        "it was searchable, so its absence below means something"
    );

    let mut ctx = home.ctx();
    let response = accept::run(
        &mut ctx,
        envelope(vec![tombstone("op-20260923-doc00012", &gone.shared_id)]),
    )
    .expect("tombstone");

    assert_eq!(response.results[0].disposition, Disposition::Accepted);
    assert_eq!(
        remote_document::revision(&home.conn, REPO, &gone.shared_id).expect("read"),
        None
    );
    assert_eq!(fts_rows(&home.conn, &gone.shared_id), 0);
    assert!(
        remote_document::revision(&home.conn, REPO, &kept.shared_id)
            .expect("read")
            .is_some(),
        "the other document is untouched"
    );
    assert!(fts_rows(&home.conn, &kept.shared_id) > 0);
}
