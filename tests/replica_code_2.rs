#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The reader half of issue 252's real-process surface: what a machine that
//! has no checkout can answer about a repository a peer shared, what changes
//! when a real checkout is connected, and the two ways a path can stop being
//! there.
//!
//! Covers AC-4 (an import never touches a local row), AC-6 (a checkout-less
//! machine answers `repos` and the graph but not `search-code`, then one repo
//! carries both revisions), AC-8 (a tracked deletion) and AC-11 (an unmounted
//! checkout is not a delete). The upload half is `replica_code.rs`.

#[path = "common/replica_support.rs"]
mod replica_support;

use replica_support::{
    CODE_REPO, Engine, cli_raw, code_envelope, git, pinned_repo, planned_generation,
    publish_locally, shared_paths,
};

/// A small pinned tree — these cases are about readers, not size.
const SMALL: usize = 12;

/// Index `root` under [`CODE_REPO`] through the real CLI.
fn index_cli(data_dir: &std::path::Path, root: &std::path::Path, extra: &[&str]) {
    let mut args = vec![
        "index-code",
        "--repo",
        CODE_REPO,
        "--path",
        root.to_str().expect("utf8 path"),
    ];
    args.extend_from_slice(extra);
    let (code, _out, err) = cli_raw(data_dir, &args);
    assert_eq!(code, 0, "index-code failed: {err}");
}

/// The `repos` row for [`CODE_REPO`], through the real CLI.
fn repo_row(engine: &Engine) -> serde_json::Value {
    let listed = engine.cli(&["repos", "--repo", CODE_REPO]);
    listed["repos"]
        .as_array()
        .expect("repos")
        .first()
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

#[test]
fn a_machine_with_no_checkout_lists_the_repo_and_answers_the_graph() {
    let workspace = tempfile::tempdir().expect("workspace");
    let root = pinned_repo(workspace.path(), SMALL);
    let author = Engine::spawn(&[]);
    index_cli(&author.data_dir(), &root, &[]);
    let payload = planned_generation(&author.data_dir(), CODE_REPO).expect("generation");

    let peer = Engine::spawn(&[]);
    let (status, body) = peer.post(
        "/api/v1/sync/replica/import",
        &code_envelope("op-20260922-view0001", CODE_REPO, &payload),
    );
    assert_eq!(status, 200, "import: {body}");

    let row = repo_row(&peer);
    assert_eq!(row["repo"], CODE_REPO, "the inventory lists it: {row}");
    assert_eq!(row["status"], "shared");
    assert_eq!(row["shared_head"], payload["head"]);
    assert!(row["last_head"].is_null(), "nothing was indexed here");
    assert_eq!(row["symbols"], 0);

    let (status, graph) = peer.get(&format!(
        "/api/v1/graph?repo={}",
        urlencoding_lite(CODE_REPO)
    ));
    assert_eq!(status, 200, "{graph}");
    let edges = graph["data"]["edges"].as_array().expect("edges");
    assert!(
        !edges.is_empty(),
        "the graph answers from the shared projection: {graph}"
    );

    let found = peer.cli(&["search-code", "fn", "--repo", CODE_REPO]);
    assert_eq!(
        found["hits"].as_array().map(Vec::len),
        Some(0),
        "but source search has no snippet to return: {found}"
    );
}

#[test]
fn connecting_a_real_checkout_leaves_one_repo_carrying_both_revisions() {
    let workspace = tempfile::tempdir().expect("workspace");
    let root = pinned_repo(workspace.path(), SMALL);
    let author = Engine::spawn(&[]);
    index_cli(&author.data_dir(), &root, &[]);
    let payload = planned_generation(&author.data_dir(), CODE_REPO).expect("generation");
    let peer = Engine::spawn(&[]);
    let (status, _) = peer.post(
        "/api/v1/sync/replica/import",
        &code_envelope("op-20260922-view0002", CODE_REPO, &payload),
    );
    assert_eq!(status, 200);

    index_cli(&peer.data_dir(), &root, &[]);

    let listed = peer.cli(&["repos"]);
    let rows = listed["repos"].as_array().expect("repos");
    assert_eq!(rows.len(), 1, "one repository, not one per side: {listed}");
    assert!(
        !rows[0]["last_head"].is_null(),
        "the local revision is here: {listed}"
    );
    assert_eq!(
        rows[0]["shared_head"], payload["head"],
        "and so is the shared one"
    );
    let found = peer.cli(&["search-code", "fn", "--repo", CODE_REPO]);
    assert!(
        found["hits"].as_array().is_some_and(|h| !h.is_empty()),
        "local snippets now answer source search: total={}",
        found["total"]
    );
}

#[test]
fn an_import_leaves_a_real_local_index_byte_identical() {
    let workspace = tempfile::tempdir().expect("workspace");
    let root = pinned_repo(workspace.path(), SMALL);
    let peer = Engine::spawn(&[]);
    index_cli(&peer.data_dir(), &root, &[]);
    let before = local_symbols(&peer);
    assert!(!before.is_empty(), "the fixture has local rows to protect");

    // A peer's generation for the SAME label, built at a head this machine
    // has never seen.
    let author = Engine::spawn(&[]);
    let other = pinned_repo(workspace.path().join("second").as_path(), SMALL);
    index_cli(&author.data_dir(), &other, &[]);
    let payload = planned_generation(&author.data_dir(), CODE_REPO).expect("generation");
    let (status, body) = peer.post(
        "/api/v1/sync/replica/import",
        &code_envelope("op-20260922-view0003", CODE_REPO, &payload),
    );
    assert_eq!(status, 200, "import: {body}");

    assert_eq!(
        local_symbols(&peer),
        before,
        "an import writes only the pulled projection"
    );
}

/// `(path, symbol, snippet)` for every locally indexed symbol, in id order.
fn local_symbols(engine: &Engine) -> Vec<(String, String, String)> {
    let conn = engine.db();
    let mut statement = conn
        .prepare("SELECT path, symbol, snippet FROM code_symbols WHERE repo = ?1 ORDER BY id")
        .expect("prepare");
    statement
        .query_map([CODE_REPO], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("rows")
}

#[test]
fn a_tracked_deletion_leaves_the_path_out_of_the_next_generation() {
    let workspace = tempfile::tempdir().expect("workspace");
    let root = pinned_repo(workspace.path(), SMALL);
    let author = Engine::spawn(&[]);
    index_cli(&author.data_dir(), &root, &[]);
    let first = planned_generation(&author.data_dir(), CODE_REPO).expect("first");
    // The author publishes its own generation, so the next one it plans
    // extends this one rather than claiming to be the repo's first.
    publish_locally(&author.data_dir(), CODE_REPO);
    let peer = Engine::spawn(&[]);
    let (status, _) = peer.post(
        "/api/v1/sync/replica/import",
        &code_envelope("op-20260922-del00001", CODE_REPO, &first),
    );
    assert_eq!(status, 200);
    assert!(
        shared_paths(&peer.data_dir(), CODE_REPO).contains(&"file_0000.rs".to_string()),
        "the peer holds the path the author is about to delete"
    );

    std::fs::remove_file(root.join("file_0000.rs")).expect("remove");
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "drop a tracked file"]);
    // A full re-walk is what forgets a deleted file's cursor: the incremental
    // walk only ever visits files that still exist.
    index_cli(&author.data_dir(), &root, &["--mode", "full"]);
    let second = planned_generation(&author.data_dir(), CODE_REPO).expect("second");
    let (status, body) = peer.post(
        "/api/v1/sync/replica/import",
        &code_envelope("op-20260922-del00002", CODE_REPO, &second),
    );
    assert_eq!(status, 200, "second import: {body}");

    let held = shared_paths(&peer.data_dir(), CODE_REPO);
    assert!(
        !held.contains(&"file_0000.rs".to_string()),
        "the deleted path is gone from the peer: {held:?}"
    );
    assert_eq!(held.len(), SMALL - 1, "and nothing else went with it");
}

#[test]
fn an_unmounted_checkout_is_not_a_delete() {
    let workspace = tempfile::tempdir().expect("workspace");
    let root = pinned_repo(workspace.path(), SMALL);
    let engine = Engine::spawn(&[]);
    index_cli(&engine.data_dir(), &root, &[]);
    let before = repo_row(&engine);
    assert!(!before["last_head"].is_null(), "indexed: {before}");

    // The volume goes away. Nothing about the index changed.
    std::fs::remove_dir_all(&root).expect("unmount");

    let after = repo_row(&engine);
    assert_eq!(after["repo"], CODE_REPO, "the row is still listed: {after}");
    assert_eq!(
        after["symbols"], before["symbols"],
        "and its index was not deleted"
    );
    assert_eq!(
        after["status"], "unknown",
        "git state is unreadable rather than absent: {after}"
    );
    assert!(
        planned_generation(&engine.data_dir(), CODE_REPO).is_some(),
        "the generation it would offer is still derivable; a disconnect \
         withholds the push, it does not erase what was indexed"
    );
}

/// Percent-encode the `/` in a repo label for a query string. The labels in
/// this suite contain nothing else that needs escaping.
fn urlencoding_lite(label: &str) -> String {
    label.replace('/', "%2F")
}
