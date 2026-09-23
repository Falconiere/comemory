#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    clippy::print_stderr
)]
//! Test mirror for `src/domains/documents/document/writer.rs`: the full extract →
//! one-transaction write path plus tombstone reconciliation. The
//! fingerprint-skip ladder
//! (`src/domains/documents/document/fingerprint.rs`) has its own
//! mirror at `tests/document__fingerprint.rs`; both share fixture
//! plumbing via `common/document_writer_support.rs`.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use comemory::domains::documents::document::DocumentFormat;
use comemory::domains::documents::document::deletions;
use comemory::domains::documents::document::writer::{self, UpdateOutcome};
use comemory::store::sources;
use comemory::store::{document_fts, documents};
use rusqlite::{Connection, params};
use tempfile::TempDir;

use crate::test_common::document_writer_support as support;
use support::*;

/// Real fixtures only this mirror indexes — `changelog.txt` (shared
/// with `tests/document__fingerprint.rs`) lives in `support`.
const GUIDE_MD: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/common/fixtures/docs/guide.md"
));
const PAGE_HTML: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/common/fixtures/docs/page.html"
));

fn chunk_count(conn: &Connection, document_id: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM document_chunks WHERE document_id = ?1",
        params![document_id],
        |r| r.get(0),
    )
    .expect("count chunks")
}

fn fts_count(conn: &Connection, document_id: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM document_fts WHERE document_id = ?1",
        params![document_id],
        |r| r.get(0),
    )
    .expect("count fts rows")
}

#[test]
fn first_index_creates_all_rows() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);

    let outcome = index(&mut conn, &path, "guide.md", DocumentFormat::Markdown);
    let UpdateOutcome::Indexed { document_id } = outcome else {
        panic!("expected Indexed, got {outcome:?}");
    };

    let files = sources::list_files_by_source(&conn, SOURCE_ID).expect("list files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].status, "indexed");
    assert_eq!(files[0].classification, "document");
    assert!(files[0].sha256.is_some());

    let doc = documents::get_document(&conn, &document_id)
        .expect("get document")
        .expect("document row exists");
    assert_eq!(doc.title, "Comemory CLI Guide");

    let chunks = chunk_count(&conn, &document_id);
    assert!(chunks > 1, "guide.md has multiple heading sections");
    assert_eq!(
        fts_count(&conn, &document_id),
        chunks,
        "one fts row per chunk"
    );
}

#[test]
fn content_edit_at_same_path_preserves_document_id_and_replaces_chunks() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path = write_fixture(&tmp, "notes.md", b"# First\n\noriginal content here.\n");

    let first = index(&mut conn, &path, "notes.md", DocumentFormat::Markdown);
    let UpdateOutcome::Indexed { document_id: id_1 } = first else {
        panic!("expected Indexed")
    };

    fs::write(
        &path,
        b"# Second\n\nrewritten content, totally different.\n",
    )
    .expect("rewrite");
    let second = index(&mut conn, &path, "notes.md", DocumentFormat::Markdown);
    let UpdateOutcome::Indexed { document_id: id_2 } = second else {
        panic!("expected Indexed")
    };
    assert_eq!(
        id_1, id_2,
        "document id must survive a content edit at the same path"
    );

    let doc = documents::get_document(&conn, &id_2)
        .expect("get")
        .expect("exists");
    assert_eq!(doc.title, "Second");

    let mut stmt = conn
        .prepare("SELECT text FROM document_chunks WHERE document_id = ?1 ORDER BY ordinal")
        .expect("prepare");
    let texts: Vec<String> = stmt
        .query_map(params![id_2], |r| r.get(0))
        .expect("query")
        .collect::<std::result::Result<_, _>>()
        .expect("rows");
    assert!(texts.iter().any(|t| t.contains("rewritten content")));
    assert!(!texts.iter().any(|t| t.contains("original content")));
}

#[cfg(unix)]
#[test]
fn unreadable_file_after_a_good_index_records_error_and_keeps_prior_rows() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);

    let first = index(&mut conn, &path, "guide.md", DocumentFormat::Markdown);
    let UpdateOutcome::Indexed { document_id } = first else {
        panic!("expected Indexed")
    };
    let doc_before = documents::get_document(&conn, &document_id)
        .expect("get")
        .expect("exists");
    let chunks_before = chunk_count(&conn, &document_id);

    // Resize (forces past the fingerprint shortcut) then revoke all
    // access so the writer's own `fs::read` fails.
    let mut resized = GUIDE_MD.to_vec();
    resized.extend_from_slice(b"\nmore content to change the size\n");
    fs::write(&path, &resized).expect("resize");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod 000");

    if fs::read(&path).is_ok() {
        // uid 0 bypasses mode bits, so the "unreadable" precondition cannot
        // be staged under root. Say so on stderr (visible with --nocapture
        // and in every failure report) instead of passing silently.
        eprintln!(
            "SKIPPED: chmod 000 left {} readable (running as root?)",
            path.display()
        );
        return;
    }

    let second = index(&mut conn, &path, "guide.md", DocumentFormat::Markdown);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("restore perms");

    assert!(
        matches!(second, UpdateOutcome::Error(_)),
        "unreadable file must report Error, got {second:?}"
    );

    let files = sources::list_files_by_source(&conn, SOURCE_ID).expect("list");
    assert_eq!(files[0].status, "error");
    assert!(files[0].error.is_some());

    let doc_after = documents::get_document(&conn, &document_id)
        .expect("get")
        .expect("prior revision must still exist");
    assert_eq!(
        doc_before, doc_after,
        "prior documents row must be untouched"
    );
    assert_eq!(
        chunks_before,
        chunk_count(&conn, &document_id),
        "prior chunks must be untouched"
    );
}

/// Index guide.md + changelog.txt under [`SOURCE_ID`] and return the
/// live connection plus each document's id — shared setup for the
/// `reconcile_deletions` tests below.
fn seed_two_sources(tmp: &TempDir) -> (Connection, String, String) {
    let mut conn = open_db(tmp);
    let path_a = write_fixture(tmp, "guide.md", GUIDE_MD);
    let path_b = write_fixture(tmp, "changelog.txt", CHANGELOG_TXT);
    let UpdateOutcome::Indexed { document_id: doc_a } =
        index(&mut conn, &path_a, "guide.md", DocumentFormat::Markdown)
    else {
        panic!("expected Indexed")
    };
    let UpdateOutcome::Indexed { document_id: doc_b } =
        index(&mut conn, &path_b, "changelog.txt", DocumentFormat::Txt)
    else {
        panic!("expected Indexed")
    };
    (conn, doc_a, doc_b)
}

#[test]
fn reconcile_deletions_removes_derived_rows_and_tombstones() {
    let tmp = TempDir::new().expect("tempdir");
    let (mut conn, doc_a, doc_b) = seed_two_sources(&tmp);

    // A fresh, authoritative scan only saw guide.md this time.
    let mut seen = HashSet::new();
    seen.insert("guide.md".to_string());
    let removed = deletions::reconcile_deletions(&mut conn, SOURCE_ID, &seen).expect("reconcile");
    assert_eq!(removed, 1);

    let files = sources::list_files_by_source(&conn, SOURCE_ID).expect("list");
    let changelog_row = files
        .iter()
        .find(|r| r.relative_path == "changelog.txt")
        .expect("row kept as tombstone");
    assert_eq!(changelog_row.status, "deleted");
    let guide_row = files
        .iter()
        .find(|r| r.relative_path == "guide.md")
        .expect("guide row present");
    assert_eq!(
        guide_row.status, "indexed",
        "reachable file must be untouched"
    );

    assert!(
        documents::get_document(&conn, &doc_b)
            .expect("get")
            .is_none(),
        "tombstoned document row removed"
    );
    assert!(
        documents::get_document(&conn, &doc_a)
            .expect("get")
            .is_some(),
        "kept document row survives"
    );
    assert_eq!(
        fts_count(&conn, &doc_b),
        0,
        "tombstoned document's fts rows removed"
    );
}

#[test]
fn reconcile_deletions_is_idempotent_for_already_deleted_rows() {
    let tmp = TempDir::new().expect("tempdir");
    let (mut conn, _doc_a, _doc_b) = seed_two_sources(&tmp);

    let mut seen = HashSet::new();
    seen.insert("guide.md".to_string());
    deletions::reconcile_deletions(&mut conn, SOURCE_ID, &seen).expect("first reconcile");

    let removed_again =
        deletions::reconcile_deletions(&mut conn, SOURCE_ID, &seen).expect("second reconcile");
    assert_eq!(removed_again, 0, "already-deleted rows are left alone");
}

#[test]
fn fts_match_finds_a_guide_phrase_and_ranks_it_first() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = open_db(&tmp);
    let path_a = write_fixture(&tmp, "guide.md", GUIDE_MD);
    let path_b = write_fixture(&tmp, "changelog.txt", CHANGELOG_TXT);
    let path_c = write_fixture(&tmp, "page.html", PAGE_HTML);

    let UpdateOutcome::Indexed {
        document_id: guide_id,
    } = index(&mut conn, &path_a, "guide.md", DocumentFormat::Markdown)
    else {
        panic!("expected Indexed")
    };
    index(&mut conn, &path_b, "changelog.txt", DocumentFormat::Txt);
    index(&mut conn, &path_c, "page.html", DocumentFormat::Html);

    let hits = document_fts::search(&conn, "lazy reindex debounce window", 5).expect("search");
    assert!(
        !hits.is_empty(),
        "must find the guide's troubleshooting passage"
    );
    assert_eq!(
        local_id(&hits[0].source),
        guide_id,
        "on-topic chunk must rank first"
    );
}

/// Create `tmp/root` (a real file plus a symlink pointing at it) and
/// `tmp/outside`, returning the canonicalized root and the symlink's
/// own path — the fixture the TOCTOU regression test below repoints.
#[cfg(unix)]
fn seed_symlinked_source(tmp: &TempDir) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::symlink;

    let root = tmp.path().join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::create_dir_all(tmp.path().join("outside")).expect("create outside");
    let inside_target = root.join("real.txt");
    fs::write(&inside_target, "legit content\n").expect("write inside target");
    let link = root.join("link.txt");
    symlink(&inside_target, &link).expect("create symlink");
    (fs::canonicalize(&root).expect("canonicalize root"), link)
}

/// Every `document_chunks.text` value currently in the DB.
fn all_chunk_texts(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT text FROM document_chunks")
        .expect("prepare");
    stmt.query_map([], |r| r.get(0))
        .expect("query")
        .collect::<std::result::Result<_, _>>()
        .expect("rows")
}

/// TOCTOU regression: `source::discover` validates a symlink's target
/// once, at walk time. If the link is repointed OUTSIDE the source root
/// before the writer later reads it (a second `comemory index` run, or
/// even just a slow first one), the writer must re-validate at read
/// time and reject it — never index the foreign content.
#[cfg(unix)]
#[test]
fn symlink_repointed_outside_the_root_after_validation_is_rejected_not_indexed() {
    use std::os::unix::fs::symlink;

    let tmp = TempDir::new().expect("tempdir");
    let (source_root, link) = seed_symlinked_source(&tmp);
    let mut conn = open_db(&tmp);
    let c = candidate("link.txt", &link, DocumentFormat::Txt);
    let first = writer::update_file(&mut conn, SOURCE_ID, None, &c, &source_root, MAX_BYTES)
        .expect("update_file");
    assert!(
        matches!(first, UpdateOutcome::Indexed { .. }),
        "first run must index the in-boundary target, got {first:?}"
    );

    // Repoint the symlink to a file OUTSIDE the source root (differing
    // size forces past the fingerprint's exact-match skip).
    let secret = tmp.path().join("outside").join("secret.txt");
    fs::write(&secret, "top secret out-of-boundary content\n").expect("write secret");
    fs::remove_file(&link).expect("remove old link");
    symlink(&secret, &link).expect("repoint symlink outside root");

    let second = writer::update_file(&mut conn, SOURCE_ID, None, &c, &source_root, MAX_BYTES)
        .expect("update_file");
    assert!(
        matches!(second, UpdateOutcome::Error(_)),
        "a symlink repointed outside the root must be rejected, got {second:?}"
    );
    let files = sources::list_files_by_source(&conn, SOURCE_ID).expect("list");
    assert_eq!(files[0].status, "error");
    assert!(
        files[0].error.as_deref().unwrap_or("").contains("escaped"),
        "error diagnostic must name the boundary violation, got {:?}",
        files[0].error
    );

    let texts = all_chunk_texts(&conn);
    assert!(
        texts.iter().any(|t| t.contains("legit content")),
        "prior legitimate revision must remain indexed"
    );
    assert!(
        !texts.iter().any(|t| t.contains("top secret")),
        "foreign out-of-boundary content must never be indexed"
    );
}

/// The boundary check canonicalizes `source_root` itself rather than
/// trusting the caller to have already resolved it: a symlinked
/// (non-canonical) ALIAS of the real root must still index its in-
/// boundary content successfully, not get rejected as "escaped" merely
/// because the alias path string doesn't literally prefix-match the
/// resolved target.
#[cfg(unix)]
#[test]
fn non_canonical_symlink_alias_of_the_source_root_still_indexes() {
    use std::os::unix::fs::symlink;

    let tmp = TempDir::new().expect("tempdir");
    let real_root = tmp.path().join("real_root");
    fs::create_dir_all(&real_root).expect("create real root");
    let file_path = real_root.join("notes.txt");
    fs::write(&file_path, "hello from the real root\n").expect("write fixture");
    let alias_root = tmp.path().join("alias_root");
    symlink(&real_root, &alias_root).expect("symlink alias root");

    let mut conn = open_db(&tmp);
    let c = candidate("notes.txt", &file_path, DocumentFormat::Txt);
    // `alias_root`, not canonicalized here — the writer must resolve it.
    let outcome = writer::update_file(&mut conn, SOURCE_ID, None, &c, &alias_root, MAX_BYTES)
        .expect("update_file");
    assert!(
        matches!(outcome, UpdateOutcome::Indexed { .. }),
        "a non-canonical source_root alias must still resolve and index, got {outcome:?}"
    );
}

// ---------------------------------------------------------------------------
// AC-4: the capture rides the transactions that already exist. One operation
// per successful index, one per tombstone, and nothing at all for a document
// this machine may not share.
// ---------------------------------------------------------------------------

const REPO: &str = "Falconiere/comemory";

/// Make `REPO` shareable: approve the label the way a policy load would, and
/// give it the indexed working-tree root a path is made relative TO. Both are
/// real rows in the real tables — the writer reads no other source of truth.
fn approve(conn: &Connection, root: &std::path::Path) {
    comemory::store::repository_approval::replace_all(
        conn,
        &[(REPO.to_string(), REPO.to_string())],
        "2026-09-23T10:00:00Z",
    )
    .expect("approve the repository");
    conn.execute(
        "INSERT INTO repo_marker (repo, root_path) VALUES (?1, ?2) \
         ON CONFLICT(repo) DO UPDATE SET root_path = excluded.root_path",
        params![REPO, root.to_str().expect("utf8 root")],
    )
    .expect("record the indexed root");
}

/// Index `name` under `REPO` — the labelled variant of `support::index`.
fn index_shared(conn: &mut Connection, path: &std::path::Path, name: &str) -> UpdateOutcome {
    let c = candidate(name, path, DocumentFormat::Markdown);
    let root = fs::canonicalize(path.parent().expect("parent")).expect("canonicalize");
    writer::update_file(conn, SOURCE_ID, Some(REPO), &c, &root, MAX_BYTES).expect("update_file")
}

/// Every journalled operation, oldest first.
fn feed_rows(conn: &Connection) -> Vec<(String, String, String)> {
    let mut statement = conn
        .prepare("SELECT entity_kind, entity_key, op FROM replica_feed ORDER BY sequence")
        .expect("prepare");
    statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect")
}

/// The one `documents` row in a single-fixture database.
fn document_id_of(conn: &Connection) -> String {
    conn.query_row("SELECT id FROM documents", [], |r| r.get(0))
        .expect("one document row")
}

#[test]
fn an_approved_document_journals_exactly_one_operation() {
    let tmp = TempDir::new().expect("tempdir");
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);
    let mut conn = open_db(&tmp);
    approve(
        &conn,
        &fs::canonicalize(tmp.path()).expect("canonicalize tmp"),
    );

    let outcome = index_shared(&mut conn, &path, "guide.md");

    assert!(
        matches!(outcome, UpdateOutcome::Indexed { .. }),
        "{outcome:?}"
    );
    let share = comemory::store::document_share::by_shared_id(
        &conn,
        REPO,
        &comemory::domains::documents::share::shared_id(REPO, "guide.md"),
    )
    .expect("share read")
    .expect("the document earned a portable name");
    assert_eq!(share.path, "guide.md");
    assert_eq!(share.blocked_reason, None);
    assert_eq!(
        feed_rows(&conn),
        vec![(
            "document_revision".to_string(),
            share.shared_id.clone(),
            "upsert".to_string()
        )],
        "one operation, keyed by the portable name"
    );
}

#[test]
fn a_re_index_of_unchanged_content_journals_nothing_further() {
    let tmp = TempDir::new().expect("tempdir");
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);
    let mut conn = open_db(&tmp);
    approve(
        &conn,
        &fs::canonicalize(tmp.path()).expect("canonicalize tmp"),
    );

    index_shared(&mut conn, &path, "guide.md");
    let after_first = feed_rows(&conn).len();
    let second = index_shared(&mut conn, &path, "guide.md");

    assert!(matches!(second, UpdateOutcome::Unchanged), "{second:?}");
    assert_eq!(after_first, 1, "the first index really journalled one");
    assert_eq!(
        feed_rows(&conn).len(),
        1,
        "an unchanged file is not a new revision"
    );
}

#[test]
fn the_journal_shares_the_writer_transaction() {
    let tmp = TempDir::new().expect("tempdir");
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);
    let mut conn = open_db(&tmp);
    approve(
        &conn,
        &fs::canonicalize(tmp.path()).expect("canonicalize tmp"),
    );
    // Make the journal append fail, in SQLite itself, after the document rows
    // of the same transaction have been written. If the two were separate
    // transactions the document would survive the journal's failure.
    conn.execute_batch(
        "CREATE TRIGGER refuse_feed BEFORE INSERT ON replica_feed \
         BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .expect("arm the trigger");

    let c = candidate("guide.md", &path, DocumentFormat::Markdown);
    let root = fs::canonicalize(tmp.path()).expect("canonicalize");
    let failed = writer::update_file(&mut conn, SOURCE_ID, Some(REPO), &c, &root, MAX_BYTES);

    assert!(failed.is_err(), "the refused append must surface");
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM documents", [], |r| r.get::<_, i64>(0))
            .expect("count documents"),
        0,
        "the document write rolled back WITH the journal, so they are one \
         transaction"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM document_share", [], |r| r
            .get::<_, i64>(0))
            .expect("count shares"),
        0
    );
}

#[test]
fn an_unlabelled_document_is_indexed_and_journals_nothing() {
    let tmp = TempDir::new().expect("tempdir");
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);
    let mut conn = open_db(&tmp);
    approve(
        &conn,
        &fs::canonicalize(tmp.path()).expect("canonicalize tmp"),
    );

    // `None` label — the common case for a source registered without `--repo`.
    let outcome = index(&mut conn, &path, "guide.md", DocumentFormat::Markdown);

    assert!(
        matches!(outcome, UpdateOutcome::Indexed { .. }),
        "{outcome:?}"
    );
    assert!(
        chunk_count(&conn, &document_id_of(&conn)) > 0,
        "it is indexed locally, which is what makes the empty feed meaningful"
    );
    assert!(feed_rows(&conn).is_empty(), "nothing to share");
}

#[test]
fn a_document_under_an_unapproved_label_journals_nothing() {
    let tmp = TempDir::new().expect("tempdir");
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);
    let mut conn = open_db(&tmp);
    // A root is recorded, but the policy approves a DIFFERENT repository.
    comemory::store::repository_approval::replace_all(
        &conn,
        &[(
            "Falconiere/other".to_string(),
            "Falconiere/other".to_string(),
        )],
        "2026-09-23T10:00:00Z",
    )
    .expect("approve another repository");
    conn.execute(
        "INSERT INTO repo_marker (repo, root_path) VALUES (?1, ?2)",
        params![REPO, tmp.path().to_str().expect("utf8")],
    )
    .expect("record the root");

    let outcome = index_shared(&mut conn, &path, "guide.md");

    assert!(
        matches!(outcome, UpdateOutcome::Indexed { .. }),
        "{outcome:?}"
    );
    assert!(
        feed_rows(&conn).is_empty(),
        "unapproved repositories share nothing"
    );
    assert_eq!(
        comemory::store::document_share::by_document(&conn, &document_id_of(&conn))
            .expect("share read"),
        None,
        "and no portable name was minted for it"
    );
}

#[test]
fn a_document_whose_repository_has_no_indexed_root_journals_nothing() {
    let tmp = TempDir::new().expect("tempdir");
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);
    let mut conn = open_db(&tmp);
    // Approved, but never indexed here: there is no root to make the path
    // relative to, so any id minted now would be one no peer computes.
    comemory::store::repository_approval::replace_all(
        &conn,
        &[(REPO.to_string(), REPO.to_string())],
        "2026-09-23T10:00:00Z",
    )
    .expect("approve");

    let outcome = index_shared(&mut conn, &path, "guide.md");

    assert!(
        matches!(outcome, UpdateOutcome::Indexed { .. }),
        "{outcome:?}"
    );
    assert!(feed_rows(&conn).is_empty());
}

#[test]
fn a_file_that_cannot_be_read_journals_nothing() {
    let tmp = TempDir::new().expect("tempdir");
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);
    let mut conn = open_db(&tmp);
    approve(
        &conn,
        &fs::canonicalize(tmp.path()).expect("canonicalize tmp"),
    );
    // The read fails before extraction — the failure the writer records as
    // `error` on the `source_files` row rather than propagating.
    fs::remove_file(&path).expect("remove the file");

    let outcome = index_shared(&mut conn, &path, "guide.md");

    assert!(matches!(outcome, UpdateOutcome::Error(_)), "{outcome:?}");
    assert!(
        feed_rows(&conn).is_empty(),
        "a document that never extracted has no revision to share"
    );
}

#[test]
fn a_tombstone_journals_exactly_one_operation_for_a_shared_document() {
    let tmp = TempDir::new().expect("tempdir");
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);
    let mut conn = open_db(&tmp);
    approve(
        &conn,
        &fs::canonicalize(tmp.path()).expect("canonicalize tmp"),
    );
    index_shared(&mut conn, &path, "guide.md");
    let shared_id = comemory::domains::documents::share::shared_id(REPO, "guide.md");

    // A complete walk that no longer sees the file.
    let removed = deletions::reconcile_deletions(&mut conn, SOURCE_ID, &HashSet::<String>::new())
        .expect("reconcile");

    assert_eq!(removed, 1);
    assert_eq!(
        feed_rows(&conn),
        vec![
            (
                "document_revision".to_string(),
                shared_id.clone(),
                "upsert".to_string()
            ),
            (
                "document_revision".to_string(),
                shared_id,
                "tombstone".to_string()
            ),
        ],
        "the revision, then its removal, under one portable name"
    );
}

#[test]
fn a_tombstone_for_a_document_that_was_never_shared_journals_nothing() {
    let tmp = TempDir::new().expect("tempdir");
    let path = write_fixture(&tmp, "guide.md", GUIDE_MD);
    let mut conn = open_db(&tmp);
    // Indexed with no label, so it was never shared.
    index(&mut conn, &path, "guide.md", DocumentFormat::Markdown);
    assert!(
        documents::get_document(&conn, &document_id_of(&conn))
            .expect("read")
            .is_some(),
        "the local rows existed, so the empty feed below is not vacuous"
    );

    let removed = deletions::reconcile_deletions(&mut conn, SOURCE_ID, &HashSet::<String>::new())
        .expect("reconcile");

    assert_eq!(removed, 1, "it really was tombstoned locally");
    assert!(
        feed_rows(&conn).is_empty(),
        "a peer that never received the revision must not be told to delete it"
    );
}

/// The `documents.id` a local hit carries, for assertions that expect one.
fn local_id(source: &comemory::store::document_fts::HitSource) -> &str {
    match source {
        comemory::store::document_fts::HitSource::Local(id) => id.as_str(),
        shared @ comemory::store::document_fts::HitSource::Shared { .. } => {
            panic!("expected a local hit, got {shared:?}")
        }
    }
}
