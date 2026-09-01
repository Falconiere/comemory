#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `api::update::run` against a real store: memories written through the
//! real `api::save::run`, a real migrated SQLite database, and the markdown
//! on disk re-read with `Frontmatter::split` to prove the file (the source
//! of truth) actually changed. Covers console-api spec AC-6 — a tags patch
//! keeps the id and rewrites the frontmatter, a body patch mints a new id
//! that supersedes the old — plus the 400/404 paths and the no-op patch.

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::errors::Error;
use comemory::memory::{Frontmatter, Kind};
use comemory::store::connection;

/// A fresh `Ctx::borrowed` over a temp data dir with a migrated database.
fn open_ctx(home: &std::path::Path) -> (Paths, Config, rusqlite::Connection) {
    let paths = Paths::new(home);
    paths.ensure_dirs().expect("ensure_dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (paths, Config::defaults(), conn)
}

/// A minimal `api::save::Request` — the same shape `api::tests::show` uses.
fn save_request(body: &str) -> api::save::Request {
    api::save::Request {
        body: body.to_string(),
        title: None,
        kind: Kind::Note,
        repo: "demo".to_string(),
        tags: Vec::new(),
        author: "tester".to_string(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    }
}

/// Read a memory file back off disk and return its parsed frontmatter + body.
fn read_back(path: &str) -> (Frontmatter, String) {
    let raw = std::fs::read_to_string(path).expect("read memory markdown");
    Frontmatter::split(&raw).expect("split frontmatter")
}

#[test]
fn ac6_tags_patch_keeps_the_id_and_rewrites_the_markdown() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let saved = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(
            &mut ctx,
            save_request("connection pooling belongs in pgbouncer"),
            false,
            None,
        )
        .expect("save")
    };

    let resp = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::update::run(
            &mut ctx,
            &saved.id,
            api::update::Request {
                // The repeated tag proves the de-dup the mirror also applies.
                tags: Some(vec!["db".into(), "db".into(), "pool".into()]),
                ..api::update::Request::default()
            },
        )
        .expect("update")
    };

    assert_eq!(resp.id, saved.id, "a frontmatter patch keeps the id");
    assert_eq!(resp.path, saved.path, "and the file it lives in");
    assert!(resp.superseded.is_none());
    assert_eq!(resp.changed, vec!["tags"]);

    let (fm, body) = read_back(&resp.path);
    assert_eq!(fm.id, saved.id);
    assert_eq!(fm.tags, vec!["db".to_string(), "pool".to_string()]);
    assert!(
        body.contains("pgbouncer"),
        "body must survive a frontmatter patch verbatim: {body:?}"
    );

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let shown = api::show::run(&mut ctx, api::show::Request { id: saved.id }).expect("show");
    let mut tags = shown.tags;
    tags.sort();
    assert_eq!(tags, vec!["db".to_string(), "pool".to_string()]);
}

#[test]
fn ac6_body_patch_mints_a_new_id_that_supersedes_the_old() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let old = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let req = api::save::Request {
            quality: 4,
            tags: vec!["pooling".into()],
            kind: Kind::Convention,
            ..save_request("pgbouncer runs in session mode")
        };
        api::save::run(&mut ctx, req, false, None).expect("save")
    };

    let resp = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::update::run(
            &mut ctx,
            &old.id,
            api::update::Request {
                body: Some("pgbouncer runs in transaction mode".into()),
                ..api::update::Request::default()
            },
        )
        .expect("update")
    };

    assert_ne!(resp.id, old.id, "a body change is a new content hash");
    assert_eq!(resp.superseded, Some(old.id.clone()));
    assert_eq!(resp.changed, vec!["body"]);

    // The old memory is still there, now annotated by the supersede edge
    // `retrieval::rerank` reads (`api::show` reports it on the OLD id).
    let old_shown = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::show::run(&mut ctx, api::show::Request { id: old.id.clone() }).expect("show old")
    };
    assert_eq!(old_shown.superseded_by, Some(resp.id.clone()));

    // The new memory carries the old frontmatter forward.
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let new_shown = api::show::run(&mut ctx, api::show::Request { id: resp.id }).expect("show new");
    assert_eq!(new_shown.body, "pgbouncer runs in transaction mode");
    assert_eq!(new_shown.quality, 4);
    assert_eq!(new_shown.kind, "convention");
    assert_eq!(new_shown.repo, Some("demo".to_string()));
    assert_eq!(new_shown.tags, vec!["pooling".to_string()]);
}

#[test]
fn a_title_patch_folds_into_the_body_and_mints_a_new_id() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let old = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(
            &mut ctx,
            save_request("the body with no title"),
            false,
            None,
        )
        .expect("save")
    };

    let resp = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::update::run(
            &mut ctx,
            &old.id,
            api::update::Request {
                title: Some("Pooling policy".into()),
                ..api::update::Request::default()
            },
        )
        .expect("update")
    };

    assert_ne!(resp.id, old.id);
    assert_eq!(resp.superseded, Some(old.id));
    let (_, body) = read_back(&resp.path);
    assert_eq!(body, "Pooling policy\n\nthe body with no title");
}

#[test]
fn a_body_patch_that_changes_nothing_stays_in_place() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let body = "an unchanged body";
    let old = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(&mut ctx, save_request(body), false, None).expect("save")
    };

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let resp = api::update::run(
        &mut ctx,
        &old.id,
        api::update::Request {
            body: Some(body.to_string()),
            ..api::update::Request::default()
        },
    )
    .expect("update");

    assert_eq!(resp.id, old.id, "same content hash, same memory");
    assert!(
        resp.superseded.is_none(),
        "a memory must never supersede itself"
    );
    assert!(resp.changed.is_empty(), "got {:?}", resp.changed);
}

#[test]
fn an_empty_patch_is_a_no_op_that_still_succeeds() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let old = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(&mut ctx, save_request("nothing to patch here"), false, None).expect("save")
    };
    let before = std::fs::read_to_string(&old.path).expect("read before");

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    // Both an absent field and a field set to its current value are no-ops.
    let resp = api::update::run(
        &mut ctx,
        &old.id,
        api::update::Request {
            quality: Some(3),
            repo: Some("demo".into()),
            ..api::update::Request::default()
        },
    )
    .expect("update");

    assert_eq!(resp.id, old.id);
    assert!(resp.changed.is_empty(), "got {:?}", resp.changed);
    assert_eq!(
        std::fs::read_to_string(&old.path).expect("read after"),
        before,
        "a no-op patch must not rewrite the file"
    );
}

#[test]
fn kind_and_repo_patches_land_in_the_frontmatter() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let old = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(
            &mut ctx,
            save_request("a note that is really a bug"),
            false,
            None,
        )
        .expect("save")
    };

    let resp = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::update::run(
            &mut ctx,
            &old.id,
            api::update::Request {
                kind: Some(Kind::Bug),
                repo: Some("other-repo".into()),
                quality: Some(5),
                ..api::update::Request::default()
            },
        )
        .expect("update")
    };
    assert_eq!(resp.changed, vec!["kind", "repo", "quality"]);

    let (fm, _) = read_back(&resp.path);
    assert_eq!(fm.kind, Kind::Bug);
    assert_eq!(fm.repo, "other-repo");
    assert_eq!(fm.quality, 5);
    assert_eq!(fm.author, "tester", "untouched fields survive");

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let shown = api::show::run(&mut ctx, api::show::Request { id: old.id }).expect("show");
    assert_eq!(shown.kind, "bug");
    assert_eq!(shown.repo, Some("other-repo".to_string()));
    assert_eq!(shown.quality, 5);
}

#[test]
fn quality_out_of_range_is_bad_request() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let old = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(&mut ctx, save_request("quality range guard"), false, None).expect("save")
    };

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let err = api::update::run(
        &mut ctx,
        &old.id,
        api::update::Request {
            quality: Some(9),
            ..api::update::Request::default()
        },
    )
    .expect_err("quality 9 must be rejected");
    assert!(matches!(err, Error::BadRequest(_)), "got {err:?}");
}

#[test]
fn unknown_id_is_not_found() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let err = api::update::run(&mut ctx, "deadbeef", api::update::Request::default())
        .expect_err("unknown id must be NotFound");
    assert!(matches!(err, Error::NotFound(_)), "got {err:?}");
}

#[test]
fn soft_deleted_id_is_not_found() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let old = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(&mut ctx, save_request("about to be deleted"), false, None).expect("save")
    };
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::delete::run(&mut ctx, &old.id).expect("delete");
    }

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let err = api::update::run(
        &mut ctx,
        &old.id,
        api::update::Request {
            quality: Some(5),
            ..api::update::Request::default()
        },
    )
    .expect_err("a trashed memory cannot be patched");
    assert!(matches!(err, Error::NotFound(_)), "got {err:?}");
}
