#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Document revisions over the real surface: the real CLI binary, real spawned
//! `comemory serve` engines, real HTTP, and temp copies of this repository's
//! own `docs/guides` tree.
//!
//! This half covers the writer: the portable identity two machines must agree
//! on whatever their checkouts are called (#253 AC-1), what a rename journals
//! and in which order (AC-5), and the guarantee that nothing a payload carries
//! is a path on the machine that produced it.
//!
//! The reader half — an import into an indexed checkout, into a machine with
//! no registration at all, and search across a revoked approval — is
//! `replica_documents_2.rs`.

#[path = "common/replica_support.rs"]
mod replica_support;

use replica_support::{
    DOC_REPO, Engine, approve_docs, docs_tree, document_feed, index_docs_cli, journalled_revision,
    shared_names,
};

/// The guide every case keys on, as a repository-relative path.
const GUIDE: &str = "docs/guides/cloud-sync.md";

#[test]
fn two_engines_indexing_from_different_roots_agree_on_the_name() {
    let workspace = tempfile::tempdir().expect("workspace");
    // Two checkouts of the same repository, at different absolute paths and
    // under different directory names — the case the local id cannot survive.
    let first_root = docs_tree(workspace.path(), "checkout-one");
    let second_root = docs_tree(workspace.path(), "somewhere/else/checkout-two");

    let first = Engine::spawn(&[]);
    approve_docs(&first, &first_root);
    index_docs_cli(&first, &first_root.join("docs/guides"));

    let second = Engine::spawn(&[]);
    approve_docs(&second, &second_root);
    index_docs_cli(&second, &second_root.join("docs/guides"));

    let from_first = shared_names(&first.data_dir());
    let from_second = shared_names(&second.data_dir());
    assert!(
        !from_first.is_empty(),
        "the guides were indexed and shared, so the comparison means something"
    );
    assert_eq!(
        from_first, from_second,
        "the same documents earn the same portable names from either checkout"
    );
    assert!(
        from_first.iter().any(|(_, path)| path == GUIDE),
        "and the paths are repository-relative: {from_first:?}"
    );
}

#[test]
fn no_absolute_path_appears_anywhere_in_an_outbound_payload() {
    let workspace = tempfile::tempdir().expect("workspace");
    let root = docs_tree(workspace.path(), "checkout");
    let engine = Engine::spawn(&[]);
    approve_docs(&engine, &root);
    index_docs_cli(&engine, &root.join("docs/guides"));
    let shared_id = shared_names(&engine.data_dir())
        .into_iter()
        .find(|(_, path)| path == GUIDE)
        .expect("the guide was shared")
        .0;

    let payload = journalled_revision(&engine.data_dir(), &shared_id);

    let text = serde_json::to_string(&payload).expect("payload json");
    let absolute = root.to_str().expect("utf8 root");
    assert!(
        !text.contains(absolute),
        "the payload names this machine's checkout: {absolute}"
    );
    assert!(
        !text.contains(workspace.path().to_str().expect("utf8 workspace")),
        "nor the temp root above it"
    );
    assert_eq!(
        payload["path"], GUIDE,
        "only the repository-relative path crosses"
    );
    assert_eq!(payload["repo"], DOC_REPO);
}

#[test]
fn a_rename_journals_the_old_path_gone_before_the_new_one_arrives() {
    let workspace = tempfile::tempdir().expect("workspace");
    let root = docs_tree(workspace.path(), "checkout");
    let engine = Engine::spawn(&[]);
    approve_docs(&engine, &root);
    let guides = root.join("docs/guides");
    index_docs_cli(&engine, &guides);
    let before = document_feed(&engine.data_dir());
    assert!(!before.is_empty(), "the first index shared the guides");
    let old_id = shared_names(&engine.data_dir())
        .into_iter()
        .find(|(_, path)| path == GUIDE)
        .expect("the guide was shared")
        .0;

    std::fs::rename(guides.join("cloud-sync.md"), guides.join("handbook.md"))
        .expect("rename the guide");
    index_docs_cli(&engine, &guides);

    let after = document_feed(&engine.data_dir());
    let added: Vec<(String, String)> = after[before.len()..].to_vec();
    let new_id = shared_names(&engine.data_dir())
        .into_iter()
        .find(|(_, path)| path == "docs/guides/handbook.md")
        .expect("the renamed guide was shared")
        .0;
    assert_eq!(
        added,
        vec![
            (old_id, "tombstone".to_string()),
            (new_id, "upsert".to_string()),
        ],
        "a peer is told the old path is gone before it is told about the new \
         one, so it never holds both: {after:?}"
    );
}

#[test]
fn an_unapproved_repository_mints_no_name_and_journals_nothing() {
    let workspace = tempfile::tempdir().expect("workspace");
    let root = docs_tree(workspace.path(), "checkout");
    let engine = Engine::spawn(&[]);
    // Indexed with a label, but no policy has approved it here.
    let report = index_docs_cli(&engine, &root.join("docs/guides"));

    assert!(
        report["sources"][0]["indexed"]
            .as_u64()
            .expect("indexed count")
            > 0,
        "the documents really were indexed locally: {report}"
    );
    assert!(
        shared_names(&engine.data_dir()).is_empty(),
        "no portable name is minted for a repository this machine may not share"
    );
    assert!(
        document_feed(&engine.data_dir()).is_empty(),
        "and nothing is journalled for a peer to receive"
    );
    let sources = engine.cli(&["sources"]);
    assert_eq!(
        sources[0]["unshared_reason"], "no sync policy has been loaded",
        "and `comemory sources` says which of the four reasons it was: {sources}"
    );
}

#[test]
fn indexing_a_shared_document_queues_its_upload() {
    let workspace = tempfile::tempdir().expect("workspace");
    let root = docs_tree(workspace.path(), "checkout");
    let engine = Engine::spawn(&[]);
    approve_docs(&engine, &root);
    index_docs_cli(&engine, &root.join("docs/guides"));

    let feed = document_feed(&engine.data_dir());
    assert!(!feed.is_empty(), "the guides were shared");
    let queued: Vec<(String, String)> = {
        let conn = engine.db();
        let mut statement = conn
            .prepare(
                "SELECT entity_key, state FROM replica_operation \
                 WHERE entity_kind = 'document_revision' ORDER BY created_at, rowid",
            )
            .expect("prepare");
        statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect")
    };
    assert_eq!(
        queued.len(),
        feed.len(),
        "every shared revision owes exactly one upload: {queued:?}"
    );
    assert!(queued.iter().all(|(_, state)| state == "pending"));
}
