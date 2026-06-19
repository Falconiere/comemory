//! Integration tests for [`comemory::retrieval::code_ref_fetch::RefStatusCache`].
//!
//! Drives the per-repo current-state resolution against a REAL temp git repo
//! and a migrated `comemory.db` (no mocks): an unknown repo is unverifiable, a
//! tracked file whose HEAD blob matches the pin is fresh, and a committed edit
//! makes the same symbol ref stale once the index is current.

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

use comemory::git_utils;
use comemory::retrieval::code_ref_fetch::RefStatusCache;
use comemory::store::connection;

/// Open a freshly migrated `comemory.db` in a tempdir.
fn open_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

/// Insert a `repo_marker` row pinning `repo` to `root` at `head` (its last
/// indexed commit), the state `RefStatusCache` reads for root + currency.
fn mark_repo(conn: &rusqlite::Connection, repo: &str, root: &std::path::Path, head: &str) {
    conn.execute(
        "INSERT OR REPLACE INTO repo_marker(repo, root_path, last_mined_commit) \
         VALUES(?1, ?2, ?3)",
        rusqlite::params![repo, root.to_string_lossy(), head],
    )
    .expect("insert repo_marker");
}

#[test]
fn unknown_repo_is_unknown_even_for_pinned_ref() {
    let (_d, conn) = open_db();
    let mut cache = RefStatusCache::default();
    // No repo_marker row -> resolve_root errors -> repo_on_disk = false.
    let status = cache.status(&conn, "nope", "a.rs", true, Some("blob"), true);
    assert_eq!(status.as_str(), "unknown");
}

#[test]
fn tracked_file_blob_match_is_fresh() {
    let (_d, conn) = open_db();
    let ws = tempfile::tempdir().expect("ws");
    let repo = ws.path().join("repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("a.rs", "fn run() {}\n")], "init");
    let head = git_utils::current_head(&repo).expect("head");
    let pinned = git_utils::blob_oid_at_head(&repo, "a.rs")
        .expect("blob lookup")
        .expect("tracked blob");
    mark_repo(&conn, "r", &repo, &head);

    let mut cache = RefStatusCache::default();
    // Symbol ref, index current (marker head == HEAD), symbol resolved -> fresh.
    let status = cache.status(&conn, "r", "a.rs", true, Some(&pinned), true);
    assert_eq!(status.as_str(), "fresh");
}

#[test]
fn committed_edit_makes_pinned_symbol_stale_when_index_current() {
    let (_d, conn) = open_db();
    let ws = tempfile::tempdir().expect("ws");
    let repo = ws.path().join("repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("a.rs", "fn run() {}\n")], "init");
    let old_blob = git_utils::blob_oid_at_head(&repo, "a.rs")
        .expect("blob")
        .expect("tracked");

    // Change the committed blob; mark the index current at the NEW head.
    git_commit::commit_files(&repo, &[("a.rs", "fn run() { let _ = 1; }\n")], "edit");
    let new_head = git_utils::current_head(&repo).expect("head");
    mark_repo(&conn, "r", &repo, &new_head);

    let mut cache = RefStatusCache::default();
    // Pinned to the OLD blob; HEAD blob differs and the symbol still resolves.
    let status = cache.status(&conn, "r", "a.rs", true, Some(&old_blob), true);
    assert_eq!(status.as_str(), "stale");
}

#[test]
fn unpinned_anchor_is_unpinned_regardless_of_repo_state() {
    let (_d, conn) = open_db();
    let mut cache = RefStatusCache::default();
    let status = cache.status(&conn, "r", "a.rs", false, None, false);
    assert_eq!(status.as_str(), "unpinned");
}
