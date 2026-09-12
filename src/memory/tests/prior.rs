#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/memory/prior.rs` — `MemoryStore::prior` against real
//! files on disk: an empty store, a store with no `memories/` directory yet,
//! a live file, its trash copy, a revived file, and an unparsable file.

use comemory::config::paths::Paths;
use comemory::memory::{Kind, MemoryStore, Prior, SaveParams};

use crate::test_common as common;

const BODY: &str = "prior probe body";

#[test]
fn prior_is_none_on_an_empty_store() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths);
    assert_eq!(store.prior("00000000").unwrap(), None);
}

#[test]
fn prior_is_none_when_the_memories_dir_does_not_exist_yet() {
    // The first save ever: nothing under the data dir has been created.
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir().join("never-created"));
    let store = MemoryStore::new(paths);
    assert_eq!(store.prior("00000000").unwrap(), None);
}

#[test]
fn prior_reports_a_live_file_then_its_trash_copy_then_the_revived_file() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths);

    let rec = store.save(SaveParams::new(BODY, Kind::Note)).unwrap();
    let id = rec.frontmatter.id.clone();
    let expected = Prior {
        created: rec.frontmatter.created,
        content_hash: rec.frontmatter.content_hash.clone(),
        trashed: false,
    };
    assert_eq!(store.prior(&id).unwrap(), Some(expected.clone()));

    store.delete(&id).unwrap();
    assert_eq!(
        store.prior(&id).unwrap(),
        Some(Prior {
            trashed: true,
            ..expected.clone()
        }),
        "a deleted id is still known, via its .trash/ copy"
    );

    // A fresh store instance (no id→path cache) sees the same answers.
    let fresh = MemoryStore::new(Paths::new(sb.data_dir()));
    assert_eq!(fresh.prior(&id).unwrap().map(|p| p.trashed), Some(true));

    let revived = store
        .save(SaveParams {
            created: Some(rec.frontmatter.created),
            ..SaveParams::new(BODY, Kind::Note)
        })
        .unwrap();
    assert_eq!(revived.frontmatter.id, id);
    assert_eq!(store.prior(&id).unwrap(), Some(expected));
}

#[test]
fn collides_with_is_true_only_for_a_different_hash() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths);
    let rec = store.save(SaveParams::new(BODY, Kind::Note)).unwrap();
    let prior = store.prior(&rec.frontmatter.id).unwrap().unwrap();

    assert!(!prior.collides_with(&rec.frontmatter.content_hash));
    assert!(prior.collides_with(&"ff".repeat(32)));
}

#[test]
fn prior_propagates_an_unparsable_live_file() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    std::fs::write(
        paths.memories_dir().join("deadbeef-broken.md"),
        "---\nid: [\n---\nbody\n",
    )
    .unwrap();
    let store = MemoryStore::new(paths);

    let err = store.prior("deadbeef").unwrap_err();
    assert!(
        err.to_string().starts_with("yaml:"),
        "a file that exists but cannot be parsed must surface, not read as None: {err}"
    );
}
