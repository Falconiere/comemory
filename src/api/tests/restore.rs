#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `api::restore::run` against a real store, driven through the real
//! `api::save::run` / `api::delete::run` pair — console-api spec AC-7: a
//! soft-deleted memory comes back out of `.trash/`, its row goes live again
//! in `GET /memories` (`api::list`), and search finds it once more.

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::errors::Error;
use comemory::memory::Kind;
use comemory::store::connection;

/// A fresh `Ctx::borrowed` over a temp data dir with a migrated database.
fn open_ctx(home: &std::path::Path) -> (Paths, Config, rusqlite::Connection) {
    let paths = Paths::new(home);
    paths.ensure_dirs().expect("ensure_dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (paths, Config::defaults(), conn)
}

fn save_request(body: &str) -> api::save::Request {
    api::save::Request {
        body: body.to_string(),
        title: None,
        kind: Kind::Decision,
        repo: "demo".to_string(),
        tags: vec!["restore".to_string()],
        author: "tester".to_string(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    }
}

#[test]
fn ac7_delete_then_restore_brings_the_file_and_the_row_back() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let saved = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(
            &mut ctx,
            save_request("kafka consumers must commit offsets manually"),
            false,
            None,
        )
        .expect("save")
    };
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::delete::run(&mut ctx, &saved.id).expect("delete");
    }
    assert!(
        !std::path::Path::new(&saved.path).exists(),
        "delete must move the markdown into .trash/"
    );

    let resp = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::restore::run(&mut ctx, &saved.id).expect("restore")
    };

    assert_eq!(resp.id, saved.id);
    assert_eq!(resp.path, saved.path, "restored to its original path");
    assert!(
        std::path::Path::new(&resp.path).exists(),
        "the markdown is back under memories/"
    );
    let trash: Vec<_> = std::fs::read_dir(paths.trash_dir())
        .expect("read trash dir")
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert!(trash.is_empty(), "trash must be empty again: {trash:?}");

    // Live in the mirror: listed, shown, and findable by search.
    let listed = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::list::run(
            &mut ctx,
            api::list::Request {
                repo: None,
                kind: None,
                tag: None,
                min_quality: None,
                q: None,
                limit: 50,
                offset: 0,
                sort: api::list::Sort::Created,
            },
        )
        .expect("list")
    };
    assert!(
        listed.items.iter().any(|r| r.id == saved.id),
        "restored memory must be listed again"
    );

    let shown = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::show::run(
            &mut ctx,
            api::show::Request {
                id: saved.id.clone(),
            },
        )
        .expect("show")
    };
    assert_eq!(shown.tags, vec!["restore".to_string()]);

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let found = api::search::run(
        &mut ctx,
        api::search::Request {
            query: "kafka consumers offsets".to_string(),
            k: None,
            offset: 0,
            repo: None,
            kind: None,
            vector: None,
            since: None,
            until: None,
            as_of: None,
        },
        false,
    )
    .expect("search");
    assert!(
        found.hits.iter().any(|h| h.memory_id == saved.id),
        "the FTS row must be rebuilt by the restore: {:?}",
        found.hits.iter().map(|h| &h.memory_id).collect::<Vec<_>>()
    );
}

#[test]
fn restoring_a_live_memory_is_bad_request() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let saved = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(&mut ctx, save_request("never deleted"), false, None).expect("save")
    };

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let err = api::restore::run(&mut ctx, &saved.id).expect_err("a live memory cannot be restored");
    match err {
        Error::BadRequest(msg) => assert!(
            msg.contains("not in the trash"),
            "message must say why: {msg}"
        ),
        other => panic!("expected BadRequest, got {other:?}"),
    }
}

#[test]
fn unknown_id_is_not_found() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let err = api::restore::run(&mut ctx, "deadbeef").expect_err("unknown id must be NotFound");
    assert!(matches!(err, Error::NotFound(_)), "got {err:?}");
}

#[test]
fn a_restored_memory_can_be_deleted_and_restored_again() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let saved = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(&mut ctx, save_request("round trip twice"), false, None).expect("save")
    };

    for _ in 0..2 {
        {
            let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
            api::delete::run(&mut ctx, &saved.id).expect("delete");
        }
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let resp = api::restore::run(&mut ctx, &saved.id).expect("restore");
        assert_eq!(resp.id, saved.id);
    }

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let shown = api::show::run(
        &mut ctx,
        api::show::Request {
            id: saved.id.clone(),
        },
    )
    .expect("show after two round trips");
    assert_eq!(shown.id, saved.id);
}
