#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `api::show::run` against a real store: memories written through the real
//! `api::save::run` (never hand-inserted rows), a real migrated SQLite
//! database. Covers spec AC-6, AC-7, and the `superseded_by`/`activation`
//! wiring; the full git-repo fresh/stale/ghost walk (AC-8) and the
//! access-tracking activation sequence (AC-10) live in the real-binary
//! suite `tests/cli__show.rs`, where a subprocess is the more faithful
//! consumer of both `comemory search` and `comemory index-code`.

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::errors::Error;
use comemory::memory::Kind;
use comemory::store::connection;

/// `api::save::run` with no CLI raw-vector input, mirroring `api::tests::save`.
fn save(ctx: &mut Ctx<'_>, req: api::save::Request) -> api::save::Response {
    api::save::run(ctx, req, false, None).expect("save run")
}

fn save_request(body: &str) -> api::save::Request {
    api::save::Request {
        body: body.to_string(),
        title: None,
        kind: Kind::Note,
        repo: "demo".to_string(),
        tags: Vec::new(),
        author: String::new(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    }
}

/// A fresh `Ctx::borrowed` over a temp data dir with a migrated database.
fn open_ctx(home: &std::path::Path) -> (Paths, Config, rusqlite::Connection) {
    let paths = Paths::new(home);
    paths.ensure_dirs().expect("ensure_dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (paths, Config::defaults(), conn)
}

#[test]
fn unknown_id_is_not_found() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let err = api::show::run(
        &mut ctx,
        api::show::Request {
            id: "deadbeef".into(),
        },
    )
    .expect_err("unknown id must be NotFound");
    assert!(matches!(err, Error::NotFound(_)), "got {err:?}");
}

#[test]
fn soft_deleted_id_is_not_found() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let id = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        save(&mut ctx, save_request("a memory that will be soft-deleted")).id
    };
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::delete::run(&mut ctx, &id).expect("delete run");
    }

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let err = api::show::run(&mut ctx, api::show::Request { id })
        .expect_err("soft-deleted id must be NotFound");
    assert!(matches!(err, Error::NotFound(_)), "got {err:?}");
}

/// AC-6: body verbatim, quality, tags, and exactly one `code_refs` entry for
/// a single `repo:path:symbol` body mention (the implied file ref is
/// collapsed — see `api::show::code_refs_for`'s doc). The symbol never
/// resolves (no `comemory index-code` ran), so its status is `unpinned`.
#[test]
fn full_shape_body_quality_tags_and_one_code_ref() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let body = "the ranker reads frontmatter, never the body\n\nsee `demo:src/lib.rs:foo_fn`";
    let id = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let req = api::save::Request {
            quality: 4,
            tags: vec!["ranking".to_string(), "frontmatter".to_string()],
            ..save_request(body)
        };
        save(&mut ctx, req).id
    };

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let resp = api::show::run(&mut ctx, api::show::Request { id: id.clone() }).expect("show run");

    assert_eq!(resp.id, id);
    assert_eq!(resp.body, body, "body must round-trip verbatim");
    assert_eq!(resp.quality, 4);
    // `memory_tags` carries no ordering guarantee (no ORDER BY on the batched
    // fetch), so compare as a set rather than assuming insertion order.
    let mut tags = resp.tags.clone();
    tags.sort();
    assert_eq!(tags, vec!["frontmatter".to_string(), "ranking".to_string()]);
    assert_eq!(resp.repo, Some("demo".to_string()));
    assert!(!resp.created.is_empty());
    assert!(!resp.updated.is_empty());
    assert_eq!(resp.access_count, 0, "never accessed yet");
    assert!(resp.last_accessed.is_none());
    assert!(
        resp.activation.abs() < 0.01,
        "a never-accessed, just-created memory activates near zero: {}",
        resp.activation
    );
    assert!(resp.superseded_by.is_none());
    assert_eq!(
        resp.code_refs.len(),
        1,
        "one repo:path:symbol mention must surface exactly one code_refs row: {:?}",
        resp.code_refs
    );
    let r = &resp.code_refs[0];
    assert_eq!(r.anchor, "demo:src/lib.rs:foo_fn");
    assert_eq!(r.path, "src/lib.rs");
    assert_eq!(r.status, "unpinned", "no --ref-symbol anchor was captured");
}

/// `superseded_by` reuses `retrieval::rerank::live_superseder`'s join: the
/// same edge `comemory search`'s rerank stage reads.
#[test]
fn superseded_by_reports_the_live_superseder() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let old_id = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        save(
            &mut ctx,
            save_request("old convention: pgbouncer session mode"),
        )
        .id
    };
    let new_id = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let req = api::save::Request {
            supersedes: vec![old_id.clone()],
            ..save_request("new convention: pgbouncer transaction mode")
        };
        save(&mut ctx, req).id
    };

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let resp = api::show::run(&mut ctx, api::show::Request { id: old_id }).expect("show run");
    assert_eq!(resp.superseded_by, Some(new_id));
}

#[test]
fn a_reference_with_no_symbol_suffix_surfaces_as_a_file_ref() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let body = "file-only mention of `demo:README.md` with no symbol suffix";
    let id = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        save(&mut ctx, save_request(body)).id
    };

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let resp = api::show::run(&mut ctx, api::show::Request { id }).expect("show run");
    assert_eq!(resp.code_refs.len(), 1);
    assert_eq!(resp.code_refs[0].anchor, "demo:README.md");
    assert_eq!(resp.code_refs[0].path, "README.md");
}
