//! Behavioral tests for [`comemory::prune::stale_code::detect`] — the ghost
//! code-reference prune rule.
//!
//! A memory owning a pinned `references_symbol` anchor is a ghost candidate
//! only when its target no longer resolves against a CURRENT index: the file is
//! present in HEAD but the symbol is gone from `code_symbols`, and the index is
//! current for that repo. A symbol that still resolves is NOT a candidate, and a
//! merely STALE index degrades to `unknown` (never a false ghost). Real migrated
//! DB + real git repo, no mocks — the same fixture shape the fetch path uses.

#[path = "common/code_seed.rs"]
mod code_seed;
#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

use comemory::cli::pagination::PaginationArgs;
use comemory::cli::prune;
use comemory::git_utils;
use comemory::graph::edges::{self, EdgeKey};
use comemory::prune::stale_code;
use comemory::store::connection;

/// Seed a minimal live `memories` row (mirrors `retrieval__code_ref_collect`).
fn seed_memory(conn: &rusqlite::Connection, id: &str) {
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES(?1,'s','note','h','body','t','t','p.md')",
        [id],
    )
    .expect("seed memory");
}

/// Seed a `code_ref` symbol anchor carrying a pinned blob.
fn seed_anchor(conn: &rusqlite::Connection, memory_id: &str, dst: &str, blob: &str) {
    conn.execute(
        "INSERT INTO code_ref(memory_id, rel, dst_id, pinned_blob, created_at) \
         VALUES(?1, 'references_symbol', ?2, ?3, 't')",
        rusqlite::params![memory_id, dst, blob],
    )
    .expect("seed anchor");
}

/// Stamp `repo` as resolved to `root` and indexed at `marked_head`. When
/// `marked_head` equals the repo's live HEAD the index is "current" and the
/// symbol-ghost verdict is trusted; otherwise it degrades to `unknown`.
fn mark_repo(conn: &rusqlite::Connection, repo: &str, root: &std::path::Path, marked_head: &str) {
    conn.execute(
        "INSERT INTO repo_marker(repo, root_path, last_mined_commit) VALUES(?1, ?2, ?3)",
        rusqlite::params![repo, root.to_string_lossy(), marked_head],
    )
    .expect("mark repo");
}

/// Build a one-file repo committed to HEAD; return (workspace, repo, head).
fn repo_with_file() -> (tempfile::TempDir, std::path::PathBuf, String) {
    let ws = tempfile::tempdir().expect("ws");
    let repo = ws.path().join("repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("a.rs", "fn run() {}\n")], "init");
    let head = git_utils::current_head(&repo).expect("head");
    (ws, repo, head)
}

#[test]
fn present_symbol_is_not_a_ghost_candidate() {
    let (_d, conn) = code_seed::open_db();
    let (_ws, repo, head) = repo_with_file();
    let pinned = git_utils::blob_oid_at_head(&repo, "a.rs")
        .expect("blob")
        .expect("tracked");

    seed_memory(&conn, "m1");
    // Symbol present in a CURRENT index -> fresh, not ghost.
    code_seed::seed_symbol(&conn, "r", "a.rs", "run");
    seed_anchor(&conn, "m1", "r:a.rs:run", &pinned);
    mark_repo(&conn, "r", &repo, &head);

    let flagged = stale_code::detect(&conn).expect("detect");
    assert!(
        !flagged.contains(&"m1".to_string()),
        "a symbol that still resolves must not be a ghost candidate, got {flagged:?}"
    );
}

#[test]
fn missing_symbol_with_current_index_is_a_ghost_candidate() {
    let (_d, conn) = code_seed::open_db();
    let (_ws, repo, head) = repo_with_file();
    let pinned = git_utils::blob_oid_at_head(&repo, "a.rs")
        .expect("blob")
        .expect("tracked");

    seed_memory(&conn, "m1");
    // File present in HEAD, but the symbol is absent from code_symbols, and the
    // index is current -> Ghost.
    seed_anchor(&conn, "m1", "r:a.rs:gone", &pinned);
    mark_repo(&conn, "r", &repo, &head);

    let flagged = stale_code::detect(&conn).expect("detect");
    assert_eq!(
        flagged,
        vec!["m1".to_string()],
        "a pinned symbol gone from a current index must be a ghost candidate"
    );
}

#[test]
fn stale_index_does_not_produce_a_false_ghost() {
    let (_d, conn) = code_seed::open_db();
    let (_ws, repo, _head) = repo_with_file();
    let pinned = git_utils::blob_oid_at_head(&repo, "a.rs")
        .expect("blob")
        .expect("tracked");

    seed_memory(&conn, "m1");
    seed_anchor(&conn, "m1", "r:a.rs:gone", &pinned);
    // Mark the index at a DIFFERENT head than the live HEAD: index is stale, so
    // the symbol-ghost verdict degrades to `unknown`, never `ghost`.
    mark_repo(
        &conn,
        "r",
        &repo,
        "0000000000000000000000000000000000000000",
    );

    let flagged = stale_code::detect(&conn).expect("detect");
    assert!(
        flagged.is_empty(),
        "a stale index must not yield a false ghost candidate, got {flagged:?}"
    );
}

/// Seed the memory->symbol `references_symbol` edge that a real save writes
/// alongside the `code_ref` anchor, so prune's dangling-edge sweep can find
/// and purge it.
fn seed_edge(conn: &rusqlite::Connection, memory_id: &str, dst_id: &str) {
    edges::insert(
        conn,
        EdgeKey {
            src_kind: "memory",
            src_id: memory_id,
            dst_kind: "symbol",
            dst_id,
            rel: "references_symbol",
        },
    )
    .expect("seed edge");
}

/// `prune --apply` GC's the dangling `references_symbol` edge whose symbol is
/// gone from a current index AND removes the orphaned `code_ref` row, so a
/// second `stale_code::detect` no longer re-flags the memory.
#[tokio::test]
async fn prune_apply_removes_ghost_code_ref_and_stops_reflagging() {
    let (dir, conn) = code_seed::open_db();
    let (_ws, repo, head) = repo_with_file();
    let pinned = git_utils::blob_oid_at_head(&repo, "a.rs")
        .expect("blob")
        .expect("tracked");

    seed_memory(&conn, "m1");
    // Symbol absent from code_symbols (ghost) but file present in a current
    // index. Both the code_ref anchor and the backing edge are written, as a
    // real `save --ref-symbol` does.
    seed_anchor(&conn, "m1", "r:a.rs:gone", &pinned);
    seed_edge(&conn, "m1", "r:a.rs:gone");
    mark_repo(&conn, "r", &repo, &head);

    assert_eq!(
        stale_code::detect(&conn).expect("detect before"),
        vec!["m1".to_string()],
        "the ghost ref must be flagged before prune --apply"
    );
    drop(conn);

    prune::run(
        prune::Args {
            apply: true,
            page: PaginationArgs {
                limit: 50,
                offset: 0,
            },
        },
        true,
        Some(dir.path().to_path_buf()),
    )
    .await
    .expect("prune --apply");

    let conn = connection::open(dir.path().join("comemory.db")).expect("reopen");
    let code_ref_rows: i64 = conn
        .query_row("SELECT count(*) FROM code_ref", [], |r| r.get(0))
        .expect("count code_ref");
    assert_eq!(
        code_ref_rows, 0,
        "the orphaned code_ref row must be deleted by prune --apply"
    );
    assert!(
        stale_code::detect(&conn).expect("detect after").is_empty(),
        "after prune --apply removes the edge + code_ref, detect must not re-flag"
    );
}
