#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`crate::domains::code::generation`] over a REAL indexed git checkout —
//! the manifest a generation carries, the id that identifies it, and the
//! parent that decides whether anything is offered.
//!
//! Every fixture runs the production `index_code::run` against a real
//! temporary git repository; nothing here fabricates a symbol or a blob OID.

use crate::test_common::git_sample;

use comemory::config::{Config, Paths};
use comemory::domains::code::index_code::{IndexMode, Request};
use comemory::store::replica_journal::ReplicaOrigin;
use comemory::store::{code_generation, connection, indexed_files};
use comemory::utilities::context::Ctx;
use tempfile::tempdir;

/// The label the fixture repo is indexed under.
const REPO: &str = "sample";

fn ctx_over(home: &std::path::Path) -> (Paths, Config, rusqlite::Connection) {
    let paths = Paths::new(home);
    paths.ensure_dirs().expect("ensure dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (paths, Config::defaults(), conn)
}

/// One indexed fixture: the real repository, the data dir it was indexed
/// into, and the connection holding the result. Both tempdirs are kept so the
/// caller holds them alive for the length of the test.
struct Indexed {
    _home: tempfile::TempDir,
    _workspace: tempfile::TempDir,
    repo_path: std::path::PathBuf,
    paths: Paths,
    cfg: Config,
    conn: rusqlite::Connection,
}

/// Index a real repository and return everything the cases need.
fn indexed() -> Indexed {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo_path = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(home.path());
    index_run(&paths, &cfg, &mut conn, &repo_path);
    Indexed {
        _home: home,
        _workspace: workspace,
        repo_path,
        paths,
        cfg,
        conn,
    }
}

/// Run the production indexer over `repo_path`.
fn index_run(
    paths: &Paths,
    cfg: &Config,
    conn: &mut rusqlite::Connection,
    repo_path: &std::path::Path,
) {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    crate::domains::code::index_code::run(
        &mut ctx,
        Request {
            repo: REPO.into(),
            path: repo_path.to_str().expect("utf8 path").to_string(),
            mode: IndexMode::Incremental,
        },
    )
    .expect("index_code run");
}

#[test]
fn a_generations_manifest_is_exactly_what_the_index_recorded() {
    let fixture = indexed();
    let conn = &fixture.conn;

    let planned = crate::domains::code::generation::plan(conn, REPO)
        .expect("plan")
        .expect("an indexed repo has a generation");

    let mut expected: Vec<(String, String)> = indexed_files::list_for_repo(conn, REPO)
        .expect("indexed_files")
        .into_iter()
        .collect();
    expected.sort();
    let actual: Vec<(String, String)> = planned
        .projection
        .files
        .iter()
        .map(|f| (f.path.clone(), f.blob_oid.clone()))
        .collect();
    assert_eq!(
        actual, expected,
        "the manifest IS the index, not a copy of it"
    );
    assert_eq!(
        planned.generation.file_count,
        i64::try_from(expected.len()).expect("fits"),
        "the count a completeness check compares against"
    );
    assert!(
        !planned.projection.symbols.is_empty(),
        "the real repo has symbols: {:?}",
        planned.projection.symbols
    );
}

#[test]
fn a_generation_carries_the_head_and_is_built_locally() {
    let fixture = indexed();
    let conn = &fixture.conn;

    let planned = crate::domains::code::generation::plan(conn, REPO)
        .expect("plan")
        .expect("row");

    let head = comemory::store::repo_marker::last_head(conn, REPO)
        .expect("last_head")
        .expect("an indexed repo has a head");
    assert_eq!(planned.generation.head, head);
    assert_eq!(planned.generation.origin, ReplicaOrigin::Local);
    assert_eq!(
        planned.generation.parent_id, None,
        "the repo's first generation has no parent"
    );
    assert_eq!(planned.generation.generation_id.len(), 32);
}

#[test]
fn indexing_an_unchanged_tree_again_produces_the_same_generation_id() {
    let mut fixture = indexed();
    let first = crate::domains::code::generation::plan(&fixture.conn, REPO)
        .expect("plan")
        .expect("row");

    // A second real index run over the same commit. Nothing changed, so the
    // content-derived id must not move — that is what stops an idle machine
    // offering a new generation every cycle.
    let repo_path = fixture.repo_path.clone();
    index_run(&fixture.paths, &fixture.cfg, &mut fixture.conn, &repo_path);
    let second = crate::domains::code::generation::plan(&fixture.conn, REPO)
        .expect("plan")
        .expect("row");

    assert_eq!(
        second.generation.generation_id, first.generation.generation_id,
        "same tree, same head, same id"
    );
}

#[test]
fn the_next_generation_names_the_active_local_one_as_its_parent() {
    let fixture = indexed();
    let conn = &fixture.conn;
    let first = crate::domains::code::generation::plan(conn, REPO)
        .expect("plan")
        .expect("row");
    code_generation::record(conn, &first.generation, "2026-09-22T10:00:00Z").expect("record");
    code_generation::activate(
        conn,
        REPO,
        &first.generation.generation_id,
        "2026-09-22T10:01:00Z",
    )
    .expect("activate");

    let next = crate::domains::code::generation::plan(conn, REPO)
        .expect("plan")
        .expect("row");

    assert_eq!(
        next.generation.parent_id.as_deref(),
        Some(first.generation.generation_id.as_str()),
        "the next generation is planned against what the repo is at"
    );
}

#[test]
fn a_pulled_generation_is_the_next_parent_but_is_never_offered_for_upload() {
    let fixture = indexed();
    let conn = &fixture.conn;
    let mut pulled = crate::domains::code::generation::plan(conn, REPO)
        .expect("plan")
        .expect("row")
        .generation;
    pulled.generation_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
    pulled.origin = ReplicaOrigin::Sync;
    code_generation::record(conn, &pulled, "2026-09-22T10:00:00Z").expect("record");
    code_generation::activate(conn, REPO, &pulled.generation_id, "2026-09-22T10:01:00Z")
        .expect("activate");

    let next = crate::domains::code::generation::plan(conn, REPO)
        .expect("plan")
        .expect("row");

    assert_eq!(
        next.generation.parent_id.as_deref(),
        Some(pulled.generation_id.as_str()),
        "the chain is the repository's: this machine's next generation extends \
         what the repo is at, whoever built it. Claiming to be the repo's \
         first instead would mute this machine for good — no peer could \
         accept a generation whose parent is not the one they are at"
    );
    assert_eq!(
        code_generation::active(conn, REPO)
            .expect("active")
            .map(|g| g.origin),
        Some(ReplicaOrigin::Sync),
        "and the pulled projection itself is never offered back for upload"
    );
}

#[test]
fn a_repo_this_machine_never_indexed_has_no_generation() {
    let fixture = indexed();
    let conn = &fixture.conn;

    assert_eq!(
        crate::domains::code::generation::plan(conn, "Falconiere/never-indexed").expect("plan"),
        None,
        "nothing to offer for a repo with no marker"
    );
}

#[test]
fn a_symbol_in_the_projection_carries_no_source_text() {
    let fixture = indexed();
    let conn = &fixture.conn;

    let planned = crate::domains::code::generation::plan(conn, REPO)
        .expect("plan")
        .expect("row");

    // The type has no snippet field at all; this asserts the projection was
    // built from the snippet-free reader rather than from `code_symbols`
    // directly, by checking every symbol still names its real location.
    for symbol in &planned.projection.symbols {
        assert!(!symbol.symbol.is_empty());
        assert!(symbol.line_start >= 1, "1-based lines: {symbol:?}");
        assert!(symbol.line_end >= symbol.line_start, "{symbol:?}");
    }
}

/// A [`comemory::utilities::progress::ProgressSink`] that reports cancelled at
/// every file boundary — the same seam `serve::jobs::worker` cancels a real
/// job through. Everything else in this case is real: a real git checkout,
/// the production indexer, and the database it would have written.
struct CancellingSink;

impl comemory::utilities::progress::ProgressSink for CancellingSink {
    fn on_progress(&self, _done: u64, _total: u64) {}

    fn on_log(&self, _line: &str) {}

    fn is_cancelled(&self) -> bool {
        true
    }
}

#[test]
fn an_index_run_cancelled_mid_walk_leaves_no_generation_to_offer() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo_path = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(home.path());

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let cancelled = crate::domains::code::index_code::run_with_progress(
        &mut ctx,
        Request {
            repo: REPO.into(),
            path: repo_path.to_str().expect("utf8 path").to_string(),
            mode: IndexMode::Incremental,
        },
        Some(&CancellingSink),
    )
    .expect_err("a cancelled walk does not return a response");

    assert!(
        matches!(cancelled, comemory::errors::Error::Cancelled),
        "got {cancelled:?}"
    );
    assert!(
        indexed_files::list_for_repo(&conn, REPO)
            .expect("indexed_files")
            .is_empty(),
        "the cancelled run rolled its transaction back"
    );
    assert!(
        crate::domains::code::generation::plan(&conn, REPO)
            .expect("plan")
            .is_none(),
        "an index that recorded nothing has no generation to offer"
    );
    assert!(
        code_generation::all(&conn, REPO).expect("all").is_empty(),
        "and nothing was staged behind it"
    );
}
