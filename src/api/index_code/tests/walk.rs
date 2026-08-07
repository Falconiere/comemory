#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/index_code/walk.rs`. Every function in that
//! file is `pub(crate)` (shared with `cli::index_code`'s `--extract`
//! path), so it is exercised indirectly through the public
//! `api::index_code::run` entry point rather than called directly —
//! chunk-child naming, the parent-snippet headline, and the repo-root
//! stamp are all walk-internal behavior.

use crate::test_common::git_commit;
use crate::test_common::git_repo;

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;
use tempfile::tempdir;

/// Real oversized function fixture, shared with `tests/cli__index_code_2.rs`
/// and `src/ast/tests/chunk.rs` so every consumer chunks the same corpus.
const OVERSIZED_SRC: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/ast/fixtures/oversized_fn.rs"
));

fn build_oversized_repo(root: &std::path::Path) -> std::path::PathBuf {
    let repo = root.join("oversized-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("big.rs", OVERSIZED_SRC)], "init");
    repo
}

/// Index `repo` as `"big"` against a fresh temp data-dir under `home`,
/// returning the opened connection for the caller's own assertions.
fn index_big_repo(home: &std::path::Path, repo: &std::path::Path) -> rusqlite::Connection {
    let paths = Paths::new(home);
    paths.ensure_dirs().expect("ensure dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    api::index_code::run(
        &mut ctx,
        api::index_code::Request {
            repo: "big".into(),
            path: repo.to_str().expect("utf8 path").to_string(),
        },
    )
    .expect("index_code run");
    conn
}

#[test]
fn walk_writes_chunk_children_named_name_hash_n_and_a_headline_parent_snippet() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = build_oversized_repo(workspace.path());
    let conn = index_big_repo(home.path(), &repo);

    let parent_id: i64 = conn
        .query_row(
            "SELECT id FROM code_symbols WHERE repo='big' AND symbol='with_env' \
             AND parent_id IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("parent row for with_env");
    let children: Vec<String> = conn
        .prepare(
            "SELECT symbol FROM code_symbols WHERE repo='big' AND parent_id = ?1 \
             ORDER BY line_start",
        )
        .expect("prepare children query")
        .query_map([parent_id], |r| r.get(0))
        .expect("query children")
        .collect::<Result<_, _>>()
        .expect("collect children");
    assert!(
        children.len() >= 2,
        "expected >= 2 chunk children: {children:?}"
    );
    for (i, symbol) in children.iter().enumerate() {
        assert_eq!(symbol, &format!("with_env#{}", i + 1));
    }

    let parent_snippet: String = conn
        .query_row(
            "SELECT snippet FROM code_symbols WHERE id = ?1",
            [parent_id],
            |r| r.get(0),
        )
        .expect("parent snippet");
    assert!(
        parent_snippet.starts_with("fn with_env"),
        "headline keeps the signature line: {parent_snippet:?}"
    );
}

#[test]
fn walk_stamps_the_canonical_repo_root_on_repo_marker() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = build_oversized_repo(workspace.path());
    let canonical = repo.canonicalize().expect("canonicalize repo path");
    let conn = index_big_repo(home.path(), &repo);

    let root_path: String = conn
        .query_row(
            "SELECT root_path FROM repo_marker WHERE repo = 'big'",
            [],
            |r| r.get(0),
        )
        .expect("repo_marker.root_path written");
    assert_eq!(root_path, canonical.to_string_lossy());
}
