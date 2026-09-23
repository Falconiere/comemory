#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/domains/retrieval/doc_route.rs`, run against real fixture
//! documents indexed through the real writer (`document::writer`) into a
//! disposable temp DB — no mocks (mirrors `tests/document__writer.rs`'s
//! `open_db`/`index` setup).

use std::path::{Path, PathBuf};

use comemory::domains::documents::document::DocumentFormat;
use comemory::domains::documents::document::writer::{self, UpdateOutcome};
use comemory::domains::documents::source::classify::Classification;
use comemory::domains::documents::source::discover::Candidate;
use comemory::retrieval::doc_route::route_documents;
use comemory::retrieval::scope::{Domain, Domains, Filters};
use comemory::store::connection;
use comemory::store::document_fts;
use comemory::store::sources::{self, SourceRootUpsert};
use rusqlite::Connection;
use tempfile::TempDir;

const GUIDE_MD: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/common/fixtures/docs/guide.md"
));
const SOURCE_ID: &str = "source-under-test";
const MAX_BYTES: u64 = 16 * 1024 * 1024;
const K: usize = 50;

/// Open a fresh `comemory.db` and seed the `source_roots` row every
/// `source_files` row here is foreign-keyed to.
fn open_db(tmp: &TempDir) -> Connection {
    let conn = connection::open(tmp.path().join("comemory.db")).expect("open db");
    sources::upsert(
        &conn,
        SourceRootUpsert {
            id: SOURCE_ID,
            canonical_path: "/does/not/matter",
            kind: "dir",
            repo: None,
            created_at: "2026-01-01T00:00:00.000000000Z",
            updated_at: "2026-01-01T00:00:00.000000000Z",
        },
    )
    .expect("seed source_roots row");
    conn
}

fn write_fixture(tmp: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = tmp.path().join(name);
    std::fs::write(&path, bytes).expect("write fixture copy");
    path
}

fn index(
    conn: &mut Connection,
    path: &Path,
    relative: &str,
    format: DocumentFormat,
    repo: Option<&str>,
) -> UpdateOutcome {
    let c = Candidate {
        relative_path: PathBuf::from(relative),
        absolute_path: path.to_path_buf(),
        classification: Classification::Document(format),
    };
    // Every fixture here is written flat directly under the tempdir
    // root via `write_fixture`, so that root is the writer's TOCTOU
    // boundary.
    let source_root = std::fs::canonicalize(path.parent().expect("candidate path has a parent"))
        .expect("canonicalize source root");
    writer::update_file(conn, SOURCE_ID, repo, &c, &source_root, MAX_BYTES).expect("update_file")
}

#[test]
fn guide_hit_coalesces_multiple_chunk_matches_into_one_document() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);
    let UpdateOutcome::Indexed { document_id } =
        index(&mut conn, &path, "guide.md", DocumentFormat::Markdown, None)
    else {
        panic!("expected Indexed")
    };

    // Sanity: the raw FTS leg really does return more than one chunk hit
    // for this query, so the assertion below is not vacuous.
    let raw = document_fts::search(&conn, "comemory", K).expect("raw search");
    assert!(
        raw.len() > 1,
        "expected multiple chunk hits from guide.md, got {}",
        raw.len()
    );

    let hits = route_documents(&conn, "comemory", Filters::none(), &[], K).expect("route");
    assert_eq!(
        hits.len(),
        1,
        "one document, however many of its chunks matched"
    );
    assert_eq!(hits[0].document_id, document_id);
    assert_eq!(hits[0].title, "Comemory CLI Guide");
    assert_eq!(hits[0].bm25_rank, 1);
    assert!(!hits[0].snippet.is_empty());
}

#[test]
fn best_chunk_provenance_is_the_matching_passage() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);
    index(&mut conn, &path, "guide.md", DocumentFormat::Markdown, None);

    let hits = route_documents(
        &conn,
        "lazy reindex debounce window",
        Filters::none(),
        &[],
        K,
    )
    .expect("route");
    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert!(
        hit.heading_path.contains("Troubleshooting"),
        "heading path was {:?}",
        hit.heading_path
    );
    assert!(
        hit.snippet.contains("debounce"),
        "snippet was {:?}",
        hit.snippet
    );
    assert!(hit.line_range.0 > 0 && hit.line_range.1 >= hit.line_range.0);
}

#[test]
fn ordering_is_deterministic_score_desc_then_document_id_asc() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let content: &[u8] = b"# Widget\n\nWidget rollout notes for the widget team.\n";
    let path_a = write_fixture(&tmp, "doc-a.md", content);
    let path_b = write_fixture(&tmp, "doc-b.md", content);
    let UpdateOutcome::Indexed { document_id: id_a } = index(
        &mut conn,
        &path_a,
        "doc-a.md",
        DocumentFormat::Markdown,
        None,
    ) else {
        panic!("expected Indexed")
    };
    let UpdateOutcome::Indexed { document_id: id_b } = index(
        &mut conn,
        &path_b,
        "doc-b.md",
        DocumentFormat::Markdown,
        None,
    ) else {
        panic!("expected Indexed")
    };

    let hits = route_documents(&conn, "widget", Filters::none(), &[], K).expect("route");
    assert_eq!(hits.len(), 2, "both identical-content documents match");
    let (first, second) = if id_a < id_b {
        (id_a, id_b)
    } else {
        (id_b, id_a)
    };
    assert_eq!(
        hits[0].document_id, first,
        "identical BM25 scores break the tie toward the lower document id"
    );
    assert_eq!(hits[1].document_id, second);
    assert_eq!(hits[0].bm25_rank, 1);
    assert_eq!(hits[1].bm25_rank, 2);

    let again = route_documents(&conn, "widget", Filters::none(), &[], K).expect("route again");
    assert_eq!(hits, again, "re-running the same query is byte-identical");
}

#[test]
fn repo_filter_narrows_to_the_matching_repo() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let content: &[u8] = b"# Widget\n\nWidget rollout notes.\n";
    let path_a = write_fixture(&tmp, "repo-a.md", content);
    let path_b = write_fixture(&tmp, "repo-b.md", content);
    let UpdateOutcome::Indexed { document_id: id_a } = index(
        &mut conn,
        &path_a,
        "repo-a.md",
        DocumentFormat::Markdown,
        Some("repo-a"),
    ) else {
        panic!("expected Indexed")
    };
    index(
        &mut conn,
        &path_b,
        "repo-b.md",
        DocumentFormat::Markdown,
        Some("repo-b"),
    );

    let filters = Filters {
        repo: Some("repo-a"),
        ..Filters::none()
    };
    let hits = route_documents(&conn, "widget", filters, &[], K).expect("route");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].document_id, id_a);
}

#[test]
fn path_glob_narrows_to_the_matching_relative_path() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let content: &[u8] = b"# Widget\n\nWidget rollout notes.\n";
    let path_a = write_fixture(&tmp, "in-docs.md", content);
    let path_b = write_fixture(&tmp, "in-notes.md", content);
    let UpdateOutcome::Indexed { document_id: id_a } = index(
        &mut conn,
        &path_a,
        "docs/widget.md",
        DocumentFormat::Markdown,
        None,
    ) else {
        panic!("expected Indexed")
    };
    index(
        &mut conn,
        &path_b,
        "notes/widget.md",
        DocumentFormat::Markdown,
        None,
    );

    let globs = vec!["docs/**".to_string()];
    let hits = route_documents(&conn, "widget", Filters::none(), &globs, K).expect("route");
    assert_eq!(hits.len(), 1, "only the docs/ path must survive: {hits:?}");
    assert_eq!(hits[0].document_id, id_a);
    assert_eq!(hits[0].path, "docs/widget.md");
}

#[test]
fn path_glob_entries_are_ored_together() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let content: &[u8] = b"# Widget\n\nWidget rollout notes.\n";
    let path_a = write_fixture(&tmp, "in-docs.md", content);
    let path_b = write_fixture(&tmp, "in-notes.md", content);
    index(
        &mut conn,
        &path_a,
        "docs/widget.md",
        DocumentFormat::Markdown,
        None,
    );
    index(
        &mut conn,
        &path_b,
        "notes/widget.md",
        DocumentFormat::Markdown,
        None,
    );

    let globs = vec!["docs/**".to_string(), "notes/**".to_string()];
    let hits = route_documents(&conn, "widget", Filters::none(), &globs, K).expect("route");
    assert_eq!(
        hits.len(),
        2,
        "each glob's matches must be included (OR semantics): {hits:?}"
    );
}

#[test]
fn invalid_path_glob_is_a_usage_error() {
    let tmp = TempDir::new().expect("tempdir");
    let conn = open_db(&tmp);
    let globs = vec!["docs/[unclosed".to_string()];
    let err = route_documents(&conn, "widget", Filters::none(), &globs, K)
        .expect_err("malformed glob must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("--path") && msg.contains("docs/[unclosed"),
        "error must name the flag and the offending pattern: {msg}"
    );
}

#[test]
fn document_excluded_domain_scope_returns_empty() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);
    index(&mut conn, &path, "guide.md", DocumentFormat::Markdown, None);

    let filters = Filters {
        domains: Domains::of(&[Domain::Memory, Domain::Code]),
        ..Filters::none()
    };
    let hits = route_documents(&conn, "comemory", filters, &[], K).expect("route");
    assert!(
        hits.is_empty(),
        "Document domain excluded from scope must short-circuit to empty"
    );
}

#[test]
fn empty_query_returns_empty_without_a_match() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);
    index(&mut conn, &path, "guide.md", DocumentFormat::Markdown, None);

    assert!(
        route_documents(&conn, "", Filters::none(), &[], K)
            .expect("route")
            .is_empty()
    );
    assert!(
        route_documents(&conn, "   ", Filters::none(), &[], K)
            .expect("route")
            .is_empty()
    );
}

// ---------------------------------------------------------------------------
// AC-8, AC-9 and AC-12 at route granularity: one leg over two indexes, the
// local side winning, and a pulled hit carrying where it came from.
// ---------------------------------------------------------------------------

const SHARE_REPO: &str = "Falconiere/comemory";
const SHARE_AT: &str = "2026-09-23T10:00:00Z";

/// Put a real pulled revision of `path` in the cache and approve its repo.
fn hold_shared(conn: &Connection, path: &str, revision_hash: &str) -> String {
    use comemory::domains::documents::document::extract::extract;
    use comemory::store::remote_document::{self, Chunk, Revision};

    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    let bytes = std::fs::read(&file).unwrap_or_else(|e| panic!("read real {path}: {e}"));
    let doc = extract(DocumentFormat::Markdown, &bytes, "doc").expect("real extraction");
    let shared_id = comemory::domains::documents::share::shared_id(SHARE_REPO, path);
    let revision = Revision {
        repo: SHARE_REPO.to_string(),
        shared_id: shared_id.clone(),
        path: path.to_string(),
        title: doc.title.clone(),
        format: "markdown".to_string(),
        revision_hash: revision_hash.to_string(),
        chunk_count: doc.chunks.len() as i64,
    };
    let chunks: Vec<Chunk> = doc
        .chunks
        .iter()
        .map(|c| Chunk {
            ordinal: c.ordinal as i64,
            heading_path: c.heading_path.join(" > "),
            char_range: (c.char_range.0 as i64, c.char_range.1 as i64),
            line_range: (c.line_range.0 as i64, c.line_range.1 as i64),
            simhash: c.simhash as i64,
            text: c.text.clone(),
        })
        .collect();
    remote_document::replace_revision(conn, &revision, &chunks, &[], SHARE_AT)
        .expect("hold revision");
    comemory::store::repository_approval::replace_all(
        conn,
        &[(SHARE_REPO.to_string(), SHARE_REPO.to_string())],
        SHARE_AT,
    )
    .expect("approve");
    shared_id
}

/// A term the real cloud-sync guide uses.
const SHARED_TERM: &str = "workspace";

#[test]
fn a_pulled_revision_is_returned_with_its_provenance() {
    let tmp = TempDir::new().expect("tempdir");
    let conn = open_db(&tmp);
    let shared_id = hold_shared(&conn, "docs/guides/cloud-sync.md", &"e".repeat(64));

    let hits = route_documents(&conn, SHARED_TERM, Filters::none(), &[], K).expect("route");

    let hit = hits
        .iter()
        .find(|h| h.document_id == shared_id)
        .unwrap_or_else(|| panic!("the pulled revision must answer: {hits:?}"));
    assert_eq!(
        hit.origin,
        comemory::retrieval::doc_route::DocOrigin::Shared {
            repo: SHARE_REPO.to_string(),
            revision_hash: "e".repeat(64),
        },
        "a reader has to be able to tell there is no file behind this passage"
    );
    assert_eq!(
        hit.path, "docs/guides/cloud-sync.md",
        "the repository-relative path, not a path on this machine"
    );
    assert!(!hit.snippet.is_empty(), "with the passage that matched");
}

#[test]
fn a_document_held_on_both_sides_is_returned_once_from_the_local_row() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    // Index the fixture locally, then map it onto the shared name a peer uses.
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);
    let UpdateOutcome::Indexed { document_id } =
        index(&mut conn, &path, "guide.md", DocumentFormat::Markdown, None)
    else {
        panic!("expected Indexed")
    };
    let shared_id = hold_shared(&conn, "docs/guides/cloud-sync.md", &"e".repeat(64));
    comemory::store::document_share::record(
        &conn,
        &comemory::store::document_share::Share {
            document_id: document_id.clone(),
            repo: SHARE_REPO.to_string(),
            shared_id: shared_id.clone(),
            path: "docs/guides/cloud-sync.md".to_string(),
            blocked_reason: None,
        },
        SHARE_AT,
    )
    .expect("record the mapping");

    let hits = route_documents(&conn, SHARED_TERM, Filters::none(), &[], K).expect("route");

    assert_eq!(
        hits.iter().filter(|h| h.document_id == shared_id).count(),
        0,
        "the pulled half is dropped for a document the local index holds: {hits:?}"
    );
    assert!(
        hits.iter()
            .all(|h| h.origin == comemory::retrieval::doc_route::DocOrigin::Local),
        "so every hit has a file behind it: {hits:?}"
    );
}
