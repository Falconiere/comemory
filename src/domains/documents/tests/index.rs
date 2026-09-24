#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/documents/index.rs`. Real document fixtures (`common
//! /docs_fixtures.rs`, matching `tests/api__sources.rs`);
//! `domains::documents::index::run`
//! is called directly against a `Ctx::borrowed` connection. `cli::index::run`
//! stays byte-compat tested against CLI stdout in `tests/cli__index.rs`;
//! the HTTP job route (`POST /api/v1/sources`) lives in
//! `tests/serve__routes__sources.rs`.

use crate::test_common::docs_fixtures;
use crate::test_common::git_sample;
use crate::test_common::git_worktree::add_worktree;

use comemory::config::{Config, Paths};
use comemory::domains::documents::index;
use comemory::domains::documents::share;
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use tempfile::TempDir;

fn ctx_over(home: &TempDir) -> (Paths, Config, rusqlite::Connection) {
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (paths, Config::defaults(), conn)
}

#[test]
fn run_registers_and_indexes_real_fixtures() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(&home);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let output = index::run(
        &mut ctx,
        index::Request {
            path: vec![docs.to_str().expect("utf8 path").to_string()],
            repo: Some("docs-corpus".into()),
            strict: false,
        },
    )
    .expect("index run");
    assert_eq!(output.sources.len(), 1);
    let s = &output.sources[0];
    assert_eq!(s.indexed, docs_fixtures::FIXTURE_COUNT);
    assert_eq!(s.too_large, 0);
    assert!(s.errors.is_empty());
    assert_eq!(s.repo.as_deref(), Some("docs-corpus"));
}

#[test]
fn run_never_fails_on_strict_leaving_the_error_check_to_the_caller() {
    // Regression for the module doc's "strict decision": a too-large file
    // must still surface in `Output.sources[].errors`, but `run` itself
    // must return `Ok` even though `req.strict` is set — the CLI (not this
    // module) is responsible for turning that into an `Error::Document`.
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(&home);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let output = index::run(
        &mut ctx,
        index::Request {
            path: vec![docs.to_str().expect("utf8 path").to_string()],
            repo: None,
            strict: true,
        },
    )
    .expect("run must succeed even though strict is set and files are unremarkable");
    let s = &output.sources[0];
    assert_eq!(s.indexed, docs_fixtures::FIXTURE_COUNT);
    assert!(s.errors.is_empty());
}

#[test]
fn run_labels_a_source_inside_a_linked_worktree_with_the_main_repo_name() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let repo = git_sample::build_sample_repo(workspace.path());
    let worktree = workspace.path().join("sample-repo-docs-wt");
    add_worktree(&repo, &worktree, "docs-wt");
    let docs = docs_fixtures::seed(&worktree);
    let (paths, cfg, mut conn) = ctx_over(&home);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let output = index::run(
        &mut ctx,
        index::Request {
            path: vec![docs.to_str().expect("utf8 path").to_string()],
            repo: None,
            strict: false,
        },
    )
    .expect("index from a worktree");
    assert_eq!(
        output.sources[0].repo.as_deref(),
        Some("sample-repo"),
        "the inferred label is the main worktree's, not the linked worktree's"
    );
}

#[test]
fn run_on_a_nonexistent_path_is_a_usage_error() {
    let home = TempDir::new().expect("home");
    let (paths, cfg, mut conn) = ctx_over(&home);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let err = index::run(
        &mut ctx,
        index::Request {
            path: vec!["/nonexistent/does/not/exist".to_string()],
            repo: None,
            strict: false,
        },
    )
    .expect_err("nonexistent path must fail");
    assert!(err.to_string().contains("cannot resolve"), "{err}");
}

// ---------------------------------------------------------------------------
// AC-15 and AC-5: what a walk's absences are allowed to mean, and the order a
// rename is journalled in.
// ---------------------------------------------------------------------------

/// The checked-in fixtures, read from their real location rather than
/// through `docs_fixtures::seed` — these cases need a subdirectory and a
/// rename, which the flat seeder does not produce.
const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/common/fixtures/docs");

const SHARE_REPO: &str = "Falconiere/comemory";
const SHARE_AT: &str = "2026-09-23T10:00:00Z";

/// A home + workspace where `SHARE_REPO` is approved and rooted, so indexing
/// really shares what it finds.
fn shared_workspace() -> (TempDir, TempDir, rusqlite::Connection) {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    comemory::store::repository_approval::replace_all(
        &conn,
        &[(SHARE_REPO.to_string(), SHARE_REPO.to_string())],
        SHARE_AT,
    )
    .expect("approve");
    let root = std::fs::canonicalize(workspace.path()).expect("canonicalize workspace");
    conn.execute(
        "INSERT INTO repo_marker (repo, root_path) VALUES (?1, ?2)",
        rusqlite::params![SHARE_REPO, root.to_str().expect("utf8 root")],
    )
    .expect("record the root");
    (home, workspace, conn)
}

/// Index `dir` under `SHARE_REPO` and return its one source report.
fn index_shared(
    home: &TempDir,
    conn: &mut rusqlite::Connection,
    dir: &std::path::Path,
) -> index::SourceReport {
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, conn);
    let mut output = index::run(
        &mut ctx,
        index::Request {
            path: vec![dir.to_str().expect("utf8 path").to_string()],
            repo: Some(SHARE_REPO.to_string()),
            strict: false,
        },
    )
    .expect("index run");
    output.sources.remove(0)
}

/// Every journalled `(entity_key, op)`, oldest first.
fn feed_rows(conn: &rusqlite::Connection) -> Vec<(String, String)> {
    let mut statement = conn
        .prepare("SELECT entity_key, op FROM replica_feed ORDER BY sequence")
        .expect("prepare");
    statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect")
}

#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("set permissions");
}

#[cfg(unix)]
#[test]
fn an_incomplete_walk_journals_no_deletion_and_a_complete_one_journals_the_real_ones() {
    let (home, workspace, mut conn) = shared_workspace();
    let docs = workspace.path().join("docs");
    let deep = docs.join("deep");
    std::fs::create_dir_all(&deep).expect("mkdir");
    let guide = docs.join("guide.md");
    std::fs::copy(std::path::Path::new(FIXTURE_DIR).join("guide.md"), &guide).expect("copy guide");
    std::fs::copy(
        std::path::Path::new(FIXTURE_DIR).join("changelog.txt"),
        deep.join("changelog.txt"),
    )
    .expect("copy changelog");

    let first = index_shared(&home, &mut conn, &docs);
    assert!(first.walk_complete);
    assert_eq!(first.indexed, 2, "{first:?}");
    let after_first = feed_rows(&conn);
    assert_eq!(after_first.len(), 2, "both were shared: {after_first:?}");

    // guide.md really is gone; the subdirectory merely cannot be read.
    std::fs::remove_file(&guide).expect("remove guide");
    set_mode(&deep, 0o000);
    let partial = index_shared(&home, &mut conn, &docs);
    set_mode(&deep, 0o755);

    assert!(!partial.walk_complete, "the walk skipped an entry");
    assert_eq!(
        partial.removed, 0,
        "an incomplete walk deletes nothing, not even the file that is \
         genuinely gone — the next complete walk will catch it"
    );
    assert_eq!(
        feed_rows(&conn),
        after_first,
        "and it journals no tombstone"
    );

    // The same walk, readable: now the absence is evidence.
    let complete = index_shared(&home, &mut conn, &docs);

    assert!(complete.walk_complete);
    assert_eq!(complete.removed, 1, "{complete:?}");
    let rows = feed_rows(&conn);
    assert_eq!(rows.len(), 3, "exactly one tombstone was added: {rows:?}");
    assert_eq!(rows[2].1, "tombstone");
    assert_eq!(
        rows[2].0,
        share::shared_id(SHARE_REPO, "docs/guide.md"),
        "keyed by the portable name the revision used"
    );
}

#[test]
fn a_rename_journals_the_old_paths_tombstone_then_the_new_paths_revision() {
    let (home, workspace, mut conn) = shared_workspace();
    let docs = workspace.path().join("docs");
    std::fs::create_dir_all(&docs).expect("mkdir");
    let before = docs.join("guide.md");
    std::fs::copy(std::path::Path::new(FIXTURE_DIR).join("guide.md"), &before).expect("copy guide");

    let first = index_shared(&home, &mut conn, &docs);
    assert_eq!(first.indexed, 1, "{first:?}");
    assert_eq!(feed_rows(&conn).len(), 1);

    std::fs::rename(&before, docs.join("handbook.md")).expect("rename");
    let renamed = index_shared(&home, &mut conn, &docs);

    assert_eq!(renamed.removed, 1);
    assert_eq!(renamed.indexed, 1);
    let rows = feed_rows(&conn);
    assert_eq!(
        rows[1..],
        [
            (
                share::shared_id(SHARE_REPO, "docs/guide.md"),
                "tombstone".to_string()
            ),
            (
                share::shared_id(SHARE_REPO, "docs/handbook.md"),
                "upsert".to_string()
            ),
        ],
        "the old path goes before the new one arrives, so a peer never holds \
         both: {rows:?}"
    );
}
