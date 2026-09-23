#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/documents/sources.rs` — specifically the
//! withheld-document report `comemory sources` and `GET /api/v1/sources` both
//! render. Real document fixtures are indexed through
//! `domains::documents::index::run`, and the reason a document is withheld
//! comes from the shipped rule set via `share::blocked_reason`, not a
//! hand-typed string: the listing and the gate have to agree on the rule name.

use crate::test_common::docs_fixtures;

use comemory::config::{Config, Paths};
use comemory::domains::documents::replica_payload::{ChunkWire, DocumentRevisionV1};
use comemory::domains::documents::{index, share, sources};
use comemory::store::document_share::{self, Share};
use comemory::store::{connection, documents};
use tempfile::TempDir;

const REPO: &str = "docs-corpus";
const AT: &str = "2026-09-23T10:00:00Z";

/// A value the shipped rule set really matches, assembled so the pattern
/// never appears whole in this source file — a literal would trip this
/// repository's own secret-content guardrail here.
fn marker() -> String {
    format!("{}{}{}", "AK", "IA", "QWERTYUIOPASDFGH")
}

/// Index the real document fixtures and return the home dir, the workspace,
/// the open connection and the registered source's id.
fn indexed_fixtures() -> (TempDir, TempDir, rusqlite::Connection, String) {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).expect("open db");

    let source_id = {
        let mut ctx = comemory::utilities::context::Ctx::borrowed(&paths, &cfg, &mut conn);
        let output = index::run(
            &mut ctx,
            index::Request {
                path: vec![docs.to_str().expect("utf8 path").to_string()],
                repo: Some(REPO.to_string()),
                strict: false,
            },
        )
        .expect("index the real fixtures");
        output.sources[0].source_id.clone()
    };
    (home, workspace, conn, source_id)
}

/// List through the production path, over the connection already open.
fn listing(home: &TempDir, conn: &mut rusqlite::Connection) -> Vec<sources::Row> {
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = comemory::utilities::context::Ctx::borrowed(&paths, &cfg, conn);
    sources::run(&mut ctx, sources::Request { reconcile: false }).expect("list sources")
}

/// The id and repository-relative path of the indexed `guide.md`.
fn guide_of(conn: &rusqlite::Connection, source_id: &str) -> (String, String) {
    for id in documents::document_ids_for_source(conn, source_id).expect("document ids") {
        let path = documents::get_document_path(conn, &id)
            .expect("document path")
            .expect("an indexed document has a path");
        if path.ends_with("guide.md") {
            return (id, path);
        }
    }
    panic!("guide.md was not indexed");
}

#[test]
fn a_withheld_document_is_listed_with_the_rule_that_withheld_it() {
    let (home, _workspace, mut conn, source_id) = indexed_fixtures();
    let (document_id, relative_path) = guide_of(&conn, &source_id);

    // The real first passage of the real indexed guide.md, with the marker
    // appended — the shape a capture would hand the gate.
    let chunk = documents::get_chunk(&conn, &document_id, 0)
        .expect("chunk read")
        .expect("guide.md has a first passage");
    let revision = DocumentRevisionV1 {
        shared_id: share::shared_id(REPO, &relative_path),
        repo: REPO.to_string(),
        path: relative_path.clone(),
        title: "Guide".to_string(),
        format: "markdown".to_string(),
        revision_hash: "e".repeat(64),
        chunks: vec![ChunkWire {
            ordinal: 0,
            heading_path: chunk.heading_path.clone(),
            char_start: 0,
            char_end: chunk.text.chars().count() as i64,
            line_start: chunk.line_range.0,
            line_end: chunk.line_range.1,
            simhash: 0,
            text: format!("{}{}", chunk.text, marker()),
        }],
        links: Vec::new(),
    };
    let reason = share::blocked_reason(&revision).expect("the shipped rule set must catch it");

    document_share::record(
        &conn,
        &Share {
            document_id,
            repo: REPO.to_string(),
            shared_id: revision.shared_id.clone(),
            path: relative_path.clone(),
            blocked_reason: Some(reason.clone()),
        },
        AT,
    )
    .expect("record the blocked name");

    let rows = listing(&home, &mut conn);
    assert_eq!(rows.len(), 1, "one registered source");
    assert_eq!(
        rows[0].withheld,
        vec![(relative_path, reason)],
        "the path and the rule, so an operator can see WHY it stayed local"
    );
    assert_eq!(
        rows[0].indexed,
        docs_fixtures::FIXTURE_COUNT,
        "and it is still indexed locally — withheld means unshared, not unread"
    );
}

#[test]
fn a_source_holding_nothing_back_reports_an_empty_list() {
    let (home, _workspace, mut conn, source_id) = indexed_fixtures();
    let (document_id, relative_path) = guide_of(&conn, &source_id);

    // A document with a portable name and NO blocked reason must not appear:
    // otherwise the case above would pass for a report that lists everything.
    document_share::record(
        &conn,
        &Share {
            document_id,
            repo: REPO.to_string(),
            shared_id: share::shared_id(REPO, &relative_path),
            path: relative_path,
            blocked_reason: None,
        },
        AT,
    )
    .expect("record a shareable name");

    let rows = listing(&home, &mut conn);
    assert!(
        rows[0].withheld.is_empty(),
        "nothing is withheld, got {:?}",
        rows[0].withheld
    );
}
