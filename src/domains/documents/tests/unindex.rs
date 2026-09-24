#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/documents/unindex.rs`. Real fixtures indexed
//! through `index::run`, then unregistered through `unindex::run`.
//!
//! The case that matters for replication: `comemory unindex` is an operator
//! saying "stop tracking this path here", not "this document was deleted".
//! It must journal NOTHING, or every other machine would drop a document that
//! still exists — so the assertions below first prove the local rows and the
//! journal position really existed, then prove the journal did not grow.

use crate::test_common::docs_fixtures;

use comemory::config::{Config, Paths};
use comemory::domains::documents::{index, unindex};
use comemory::store::{connection, repository_approval};
use comemory::utilities::context::Ctx;
use tempfile::TempDir;

const REPO: &str = "Falconiere/comemory";
const AT: &str = "2026-09-23T10:00:00Z";

/// Every journalled operation's `(entity_kind, op)`, oldest first.
fn feed_rows(conn: &rusqlite::Connection) -> Vec<(String, String)> {
    let mut statement = conn
        .prepare("SELECT entity_kind, op FROM replica_feed ORDER BY sequence")
        .expect("prepare");
    statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect")
}

fn count(conn: &rusqlite::Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .expect("count")
}

#[test]
fn unindexing_a_source_removes_its_rows_and_journals_nothing() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).expect("open db");

    // Approve the repository and record the root, so the index run really does
    // share what it indexes — otherwise the empty journal below proves nothing.
    repository_approval::replace_all(&conn, &[(REPO.to_string(), REPO.to_string())], AT)
        .expect("approve");
    let root = std::fs::canonicalize(workspace.path()).expect("canonicalize workspace");
    conn.execute(
        "INSERT INTO repo_marker (repo, root_path) VALUES (?1, ?2)",
        rusqlite::params![REPO, root.to_str().expect("utf8 root")],
    )
    .expect("record the root");

    let canonical_path = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let output = index::run(
            &mut ctx,
            index::Request {
                path: vec![docs.to_str().expect("utf8 path").to_string()],
                repo: Some(REPO.to_string()),
                strict: false,
            },
        )
        .expect("index the real fixtures");
        output.sources[0].canonical_path.clone()
    };

    let shared_before = count(&conn, "document_share");
    let journalled_before = feed_rows(&conn);
    assert!(
        shared_before > 0,
        "the fixtures really were shared, so the assertions below are not vacuous"
    );
    assert_eq!(
        journalled_before.len(),
        shared_before as usize,
        "one revision per shared document: {journalled_before:?}"
    );
    assert!(
        journalled_before
            .iter()
            .all(|(kind, op)| kind == "document_revision" && op == "upsert")
    );

    let response = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        unindex::run(
            &mut ctx,
            unindex::Request {
                target: canonical_path,
            },
        )
        .expect("unindex the source")
    };

    assert!(response.documents_removed > 0, "{response:?}");
    assert_eq!(count(&conn, "documents"), 0, "the local rows are gone");
    assert_eq!(
        count(&conn, "document_share"),
        0,
        "and the portable names went with them, through the cascade"
    );
    assert_eq!(
        feed_rows(&conn),
        journalled_before,
        "unregistering a path on THIS machine is not a deletion anywhere else"
    );
}
