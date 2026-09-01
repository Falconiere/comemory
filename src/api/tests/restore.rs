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

/// `B —rel→ A` rows in `edges`.
fn relation_edges(conn: &rusqlite::Connection, src: &str, rel: &str, dst: &str) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM edges WHERE src_kind = 'memory' AND src_id = ?1 \
           AND rel = ?2 AND dst_kind = 'memory' AND dst_id = ?3",
        rusqlite::params![src, rel, dst],
        |r| r.get(0),
    )
    .expect("count relation edges")
}

#[test]
fn restore_relinks_the_incoming_supersedes_edge() {
    // save A; save B --supersedes A; delete A; restore A. Soft-delete drops
    // every edge touching A, including B's supersedes edge, which lives in
    // B's frontmatter — restore must re-derive it, or rerank's supersede
    // penalty and `show A`'s `superseded_by` stay wrong until a rebuild.
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let a = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(&mut ctx, save_request("original decision"), false, None).expect("save A")
    };
    let b = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let mut req = save_request("revised decision");
        req.supersedes = vec![a.id.clone()];
        api::save::run(&mut ctx, req, false, None).expect("save B")
    };
    assert_eq!(relation_edges(&conn, &b.id, "supersedes", &a.id), 1);

    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::delete::run(&mut ctx, &a.id).expect("delete A");
    }
    assert_eq!(
        relation_edges(&conn, &b.id, "supersedes", &a.id),
        0,
        "soft-delete drops the incoming edge (both directions)"
    );

    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::restore::run(&mut ctx, &a.id).expect("restore A");
    }
    assert_eq!(
        relation_edges(&conn, &b.id, "supersedes", &a.id),
        1,
        "restore must re-derive B —supersedes→ A from B's frontmatter"
    );

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let shown = api::show::run(&mut ctx, api::show::Request { id: a.id.clone() }).expect("show A");
    assert_eq!(
        shown.superseded_by.as_deref(),
        Some(b.id.as_str()),
        "show A must report B as its live superseder again"
    );
}

#[test]
fn restore_after_a_same_body_re_save_is_bad_request_and_keeps_the_live_row() {
    // save → delete → save the same body with new tags/quality (same
    // content-hash id) → restore. The stale trash copy must never be renamed
    // over the live file, and the mirror must keep the re-save's frontmatter.
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let body = "idempotent body";
    let first = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(&mut ctx, save_request(body), false, None).expect("save")
    };
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::delete::run(&mut ctx, &first.id).expect("delete");
    }
    let second = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let mut req = save_request(body);
        req.tags = vec!["resaved".to_string()];
        req.quality = 5;
        api::save::run(&mut ctx, req, false, None).expect("re-save")
    };
    assert_eq!(second.id, first.id, "same body ⇒ same id");
    let live_bytes = std::fs::read_to_string(&second.path).expect("read live file");

    let err = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::restore::run(&mut ctx, &first.id).expect_err("restore of a live id must fail")
    };
    assert!(matches!(err, Error::BadRequest(_)), "got {err:?}");
    assert_eq!(
        std::fs::read_to_string(&second.path).expect("read live file again"),
        live_bytes,
        "the live file's frontmatter must be unchanged"
    );

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let shown = api::show::run(
        &mut ctx,
        api::show::Request {
            id: first.id.clone(),
        },
    )
    .expect("show");
    assert_eq!(shown.tags, vec!["resaved".to_string()]);
    assert_eq!(shown.quality, 5);
}
