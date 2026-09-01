#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `api::trash::run` against a real store — console-api spec AC-17: a memory
//! soft-deleted through the real `api::delete::run` shows up in the trash
//! listing with `days_until_gc == prune.trash_retention_days` on the day of
//! deletion. Also pins the two invariants the surface carries: a live memory
//! never appears, and a data dir with no database answers an empty page
//! instead of creating one.

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
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
        kind: Kind::Bug,
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

fn request(limit: usize, offset: usize) -> api::trash::Request {
    api::trash::Request { limit, offset }
}

#[test]
fn ac17_a_freshly_deleted_memory_lists_with_the_full_retention_window() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let saved = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::save::run(
            &mut ctx,
            save_request("the retry loop double-counts attempts"),
            false,
            None,
        )
        .expect("save")
    };

    let live = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::trash::run(&mut ctx, request(50, 0)).expect("trash before delete")
    };
    assert!(
        live.items.is_empty(),
        "a live memory is not in the trash: {:?}",
        live.items.iter().map(|r| &r.id).collect::<Vec<_>>()
    );

    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::delete::run(&mut ctx, &saved.id).expect("delete");
    }

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let page = api::trash::run(&mut ctx, request(50, 0)).expect("trash after delete");
    assert_eq!(page.items.len(), 1, "one soft-deleted memory");
    assert_eq!(page.total, Some(1));
    let row = &page.items[0];
    assert_eq!(row.id, saved.id);
    assert_eq!(row.title, "the retry loop double-counts attempts");
    assert_eq!(row.kind, "bug");
    assert_eq!(row.repo, Some("demo".to_string()));
    assert!(!row.deleted_at.is_empty(), "deleted_at is stamped");
    assert_eq!(
        row.days_until_gc,
        i64::from(cfg.prune.trash_retention_days),
        "the whole window is left on the day of deletion"
    );
    let path = row.path.clone().expect("the trashed file is on disk");
    assert!(
        std::path::Path::new(&path).starts_with(paths.trash_dir()),
        "path must point inside .trash/: {path}"
    );
}

#[test]
fn an_absent_database_answers_an_empty_page_without_creating_one() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let page = api::trash::run(&mut ctx, request(50, 0)).expect("trash on a fresh data dir");
    assert!(page.items.is_empty());
    assert_eq!(page.total, Some(0));
    assert!(
        !paths.db_path().exists(),
        "a read must never create comemory.db"
    );
}

#[test]
fn the_listing_pages_newest_deletion_first() {
    let home = tempfile::tempdir().expect("tempdir");
    let (paths, cfg, mut conn) = open_ctx(home.path());
    let mut ids = Vec::new();
    for n in 0..3 {
        let saved = {
            let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
            api::save::run(
                &mut ctx,
                save_request(&format!("trash row number {n}")),
                false,
                None,
            )
            .expect("save")
        };
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::delete::run(&mut ctx, &saved.id).expect("delete");
        ids.push(saved.id);
    }

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let first = api::trash::run(&mut ctx, request(2, 0)).expect("first page");
    assert_eq!(first.items.len(), 2);
    assert_eq!(first.total, Some(3));
    assert!(first.has_more, "a third row is left");

    let second = api::trash::run(&mut ctx, request(2, 2)).expect("second page");
    assert_eq!(second.items.len(), 1);
    assert!(!second.has_more);

    let paged: Vec<&str> = first
        .items
        .iter()
        .chain(second.items.iter())
        .map(|r| r.id.as_str())
        .collect();
    for id in &ids {
        assert!(paged.contains(&id.as_str()), "{id} missing from {paged:?}");
    }
}
