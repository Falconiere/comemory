#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `api::refresh_refs::run` against a real git repo indexed by the real
//! `api::index_code::run` — console-api spec AC-8: a reference whose file
//! has moved on since it was pinned reports `stale`, and one refresh re-pins
//! it to the current HEAD blob so it reports `fresh` again.
//!
//! The save-time anchor is deliberately NOT captured here: `api::save`
//! anchors against the *calling process's* cwd repo (its documented cwd
//! semantics), which in a test is the comemory checkout, not the fixture
//! repo — so the ref lands `unpinned` and the first refresh is what pins it
//! against `repo_marker.root_path`. That is the same path a memory saved
//! over HTTP takes, which makes it the honest one to test.

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::errors::Error;
use comemory::memory::Kind;
use comemory::serve::RootOverrides;
use comemory::store::connection;

use crate::test_common::{git_commit, git_repo};

/// The fixture repo's one indexed file and the symbol the memory cites.
const TARGET_PATH: &str = "src/refresh_target.rs";
const SYMBOL: &str = "refreshed_symbol";

/// A fresh `Ctx::borrowed` over a temp data dir with a migrated database.
fn open_ctx(home: &std::path::Path) -> (Paths, Config, rusqlite::Connection) {
    let paths = Paths::new(home);
    paths.ensure_dirs().expect("ensure_dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (paths, Config::defaults(), conn)
}

/// Build a one-file git repo whose single rust file defines [`SYMBOL`].
fn build_repo(workspace: &std::path::Path, body: &str) -> std::path::PathBuf {
    let repo = workspace.join("fixture-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[(TARGET_PATH, body)], "v1");
    repo
}

/// Run the real code indexer over `repo` under the label `sample`.
fn index(ctx: &mut Ctx<'_>, repo: &std::path::Path) {
    api::index_code::run(
        ctx,
        api::index_code::Request {
            repo: "sample".into(),
            path: repo.to_str().expect("utf8 repo path").to_string(),
            mode: api::index_code::IndexMode::Incremental,
        },
    )
    .expect("index_code");
}

fn save_request(body: &str, ref_symbol: Vec<String>) -> api::save::Request {
    api::save::Request {
        body: body.to_string(),
        title: None,
        kind: Kind::Note,
        repo: "sample".to_string(),
        tags: Vec::new(),
        author: "tester".to_string(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol,
    }
}

#[test]
fn ac8_a_stale_reference_is_re_pinned_fresh_at_the_current_head() {
    let home = tempfile::tempdir().expect("tempdir");
    let workspace = tempfile::tempdir().expect("workspace");
    let repo = build_repo(
        workspace.path(),
        "pub fn refreshed_symbol() -> u32 {\n    1\n}\n",
    );
    let (paths, cfg, mut conn) = open_ctx(home.path());
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        index(&mut ctx, &repo);
    }

    let anchor = format!("sample:{TARGET_PATH}:{SYMBOL}");
    let saved = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(
            &mut ctx,
            save_request(
                "the retry budget lives in refreshed_symbol",
                vec![anchor.clone()],
            ),
            false,
            None,
        )
        .expect("save")
    };

    // Pin #1: the first refresh anchors the (unpinned) ref at HEAD.
    let first = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::refresh_refs::run(&mut ctx, &saved.id, &RootOverrides::new()).expect("first refresh")
    };
    assert_eq!(first.refreshed, 1, "one anchorable reference");
    assert!(first.skipped.is_empty(), "got {:?}", first.skipped);
    assert_eq!(first.code_refs.len(), 1, "got {:?}", first.code_refs);
    assert_eq!(first.code_refs[0].anchor, anchor);
    assert_eq!(
        first.code_refs[0].status, "fresh",
        "a ref pinned at the current HEAD is fresh: {:?}",
        first.code_refs[0]
    );
    let first_blob = first.code_refs[0]
        .blob
        .clone()
        .expect("the refresh must capture a blob");

    // The commit moves the referenced file; re-indexing keeps the symbol
    // index current, so the blob compare (not the index) decides staleness.
    git_commit::commit_files(
        &repo,
        &[(
            TARGET_PATH,
            "pub fn refreshed_symbol() -> u32 {\n    2\n}\n",
        )],
        "v2",
    );
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        index(&mut ctx, &repo);
    }

    let stale = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::show::run(
            &mut ctx,
            api::show::Request {
                id: saved.id.clone(),
            },
        )
        .expect("show after the commit")
    };
    assert_eq!(
        stale.code_refs[0].status, "stale",
        "committed code changed under the pin: {:?}",
        stale.code_refs[0]
    );

    // Pin #2: the refresh moves the anchor forward, and the ref is fresh.
    let second = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::refresh_refs::run(&mut ctx, &saved.id, &RootOverrides::new()).expect("second refresh")
    };
    assert_eq!(second.refreshed, 1);
    assert_eq!(second.code_refs[0].status, "fresh");
    let second_blob = second.code_refs[0]
        .blob
        .clone()
        .expect("re-pinned blob")
        .clone();
    assert_ne!(second_blob, first_blob, "the anchor moved to the new blob");

    // The frontmatter on disk carries the new anchor, not just the mirror.
    let raw = std::fs::read_to_string(&saved.path).expect("read markdown");
    assert!(
        raw.contains(&second_blob),
        "the re-pinned blob must be written back to the markdown: {raw}"
    );
}

#[test]
fn a_reference_whose_repo_root_is_unknown_is_skipped_not_an_error() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let anchor = "never-indexed:src/gone.rs:missing_symbol".to_string();
    let saved = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(
            &mut ctx,
            save_request("cites a repo nobody indexed", vec![anchor.clone()]),
            false,
            None,
        )
        .expect("save")
    };

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let resp = api::refresh_refs::run(&mut ctx, &saved.id, &RootOverrides::new())
        .expect("refresh must not error");
    assert_eq!(resp.refreshed, 0);
    assert_eq!(resp.skipped, vec![anchor]);
}

#[test]
fn a_memory_with_no_references_refreshes_to_zero() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let saved = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(
            &mut ctx,
            save_request("no references at all", Vec::new()),
            false,
            None,
        )
        .expect("save")
    };

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let resp = api::refresh_refs::run(&mut ctx, &saved.id, &RootOverrides::new()).expect("refresh");
    assert_eq!(resp.id, saved.id);
    assert_eq!(resp.refreshed, 0);
    assert!(resp.skipped.is_empty());
    assert!(resp.code_refs.is_empty());
}

#[test]
fn unknown_id_is_not_found() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let err = api::refresh_refs::run(&mut ctx, "deadbeef", &RootOverrides::new())
        .expect_err("unknown id is NotFound");
    assert!(matches!(err, Error::NotFound(_)), "got {err:?}");
}
