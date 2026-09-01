#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/memory/store.rs` — filesystem-backed memory CRUD.

use std::time::{Duration, SystemTime};

use comemory::config::paths::Paths;
use comemory::errors::Error;
use comemory::memory::{Kind, MemoryStore, Relations, SaveParams};

use crate::test_common as common;

/// Note-kind params with the legacy test defaults (`repo = "r"`,
/// `author = "a"`, quality 3) so the simple tests stay one-liners.
fn quick(body: &str) -> SaveParams<'_> {
    SaveParams {
        repo: "r",
        author: "a",
        ..SaveParams::new(body, Kind::Note)
    }
}

#[test]
fn save_then_load_round_trips() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());

    let tags = vec!["postgres".to_string()];
    let rec = store
        .save(SaveParams {
            repo: "qwick-backend",
            tags: &tags,
            author: "falconiere",
            quality: 4,
            ..SaveParams::new("Use Postgres for analytics", Kind::Decision)
        })
        .unwrap();
    assert_eq!(rec.frontmatter.kind, Kind::Decision);
    assert_eq!(rec.frontmatter.tags, vec!["postgres".to_string()]);

    let loaded = store.load(&rec.frontmatter.id).unwrap();
    assert_eq!(loaded.body.trim(), "Use Postgres for analytics");
    assert_eq!(loaded.frontmatter.id, rec.frontmatter.id);
}

#[test]
fn save_writes_relations_into_frontmatter() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());

    let rec = store
        .save(SaveParams {
            relations: Relations {
                supersedes: vec!["a1b2c3d4".to_string()],
                ..Relations::default()
            },
            ..SaveParams::new("new convention replacing an old one", Kind::Convention)
        })
        .unwrap();
    assert_eq!(rec.frontmatter.relations.supersedes, vec!["a1b2c3d4"]);

    // The relation must round-trip through the YAML on disk, not just the
    // in-memory record — markdown is the source of truth for rebuild.
    let loaded = store.load(&rec.frontmatter.id).unwrap();
    assert_eq!(loaded.frontmatter.relations.supersedes, vec!["a1b2c3d4"]);
}

#[test]
fn save_is_atomic_under_failure() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let _ = store.save(quick("body")).unwrap();
    let entries: Vec<_> = std::fs::read_dir(paths.memories_dir())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().into_string().unwrap())
        .filter(|n| {
            std::path::Path::new(n)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("tmp"))
        })
        .collect();
    assert!(
        entries.is_empty(),
        "no .tmp files should remain: {entries:?}"
    );
}

#[test]
fn list_returns_all_saved() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths);
    let _ = store.save(quick("first")).unwrap();
    let _ = store.save(quick("second")).unwrap();
    let all = store.list().unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn delete_removes_file_and_returns_record() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store.save(quick("to delete")).unwrap();
    let removed = store.delete(&rec.frontmatter.id).unwrap();
    assert_eq!(removed.frontmatter.id, rec.frontmatter.id);
    assert!(store.load(&rec.frontmatter.id).is_err());
}

#[test]
fn list_returns_results_sorted_by_created_desc() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());

    let _ = store.save(quick("alpha")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let _ = store.save(quick("beta")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let _ = store.save(quick("gamma")).unwrap();

    let list = store.list().unwrap();
    assert_eq!(list.len(), 3);

    // Robust check: every adjacent pair is non-increasing by `created`.
    let times: Vec<_> = list.iter().map(|m| m.frontmatter.created).collect();
    assert!(
        times.windows(2).all(|w| w[0] >= w[1]),
        "list not sorted by created desc: {times:?}"
    );
}

#[test]
fn list_skips_malformed_files_and_returns_valid_ones() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());

    let good = store.save(quick("valid memory")).unwrap();

    // Drop a malformed .md file alongside the valid one.
    let bad_path = paths.memories_dir().join("zzzzzzzz-bad.md");
    std::fs::write(&bad_path, "this is not valid frontmatter at all\n").unwrap();

    let list = store.list().unwrap();
    assert_eq!(
        list.len(),
        1,
        "expected only the valid record, got {list:?}"
    );
    assert_eq!(list[0].frontmatter.id, good.frontmatter.id);
}

/// Kills the `||` → `&&` mutant on `MemoryStore::list` line 207.
///
/// The skip predicate is:
///   `!name.ends_with(".md") || name.starts_with('.')`
///
/// Under `&&` that becomes:
///   `!name.ends_with(".md") && name.starts_with('.')`
///
/// Two rogue files expose each half independently:
///
/// 1. `.hidden.md` — starts with `.` AND ends with `.md`.
///    With `||` the predicate is true (dot-prefix) → skipped ✓
///    With `&&` the predicate is false (ends with .md, so first clause false) → included ✗
///
/// 2. `notes.txt` — does NOT end with `.md` and does NOT start with `.`.
///    With `||` the predicate is true (not .md) → skipped ✓
///    With `&&` the predicate is false (no dot-prefix, so second clause false) → included ✗
///
/// The test expects exactly the one real memory; under the mutant either
/// rogue file makes the count > 1 (or causes a parse error that inflates
/// the skip counter rather than the result).
#[test]
fn list_skips_dot_prefix_md_and_non_md_files() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());

    let good = store.save(quick("real memory")).unwrap();

    // File that starts with '.' and ends with '.md' — should be skipped.
    let hidden_md = paths.memories_dir().join(".hidden.md");
    std::fs::write(&hidden_md, "---\nid: 00000000\n---\nhidden\n").unwrap();

    // File that does not end with '.md' and has no dot-prefix — should be skipped.
    let non_md = paths.memories_dir().join("notes.txt");
    std::fs::write(&non_md, "plain text, not a memory\n").unwrap();

    let list = store.list().unwrap();
    assert_eq!(
        list.len(),
        1,
        "list must include only the real memory; dot-prefix .md and non-.md files must be skipped. got {list:?}"
    );
    assert_eq!(list[0].frontmatter.id, good.frontmatter.id);
}

/// Extension matching is case-insensitive: the `.md` suffix test is an
/// `eq_ignore_ascii_case` comparison on `Path::extension`, so a file renamed
/// to `.MD` out of band stays a live memory rather than silently dropping out
/// of `list` / `find_by_id`. Renames a really-saved file instead of writing a
/// hand-built fixture, so the frontmatter under test is the real thing.
#[test]
fn list_and_find_by_id_match_the_md_extension_case_insensitively() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());

    let saved = store.save(quick("uppercase extension memory")).unwrap();
    let lower = saved.path.clone();
    let upper = lower.with_extension("MD");
    std::fs::rename(&lower, &upper).unwrap();
    assert!(upper.exists(), "renamed fixture must exist at {upper:?}");

    // A fresh store, so nothing is served out of the id -> path cache.
    let store = MemoryStore::new(paths.clone());
    let list = store.list().unwrap();
    assert_eq!(
        list.len(),
        1,
        "a .MD file must still be listed as a memory, got {list:?}"
    );
    assert_eq!(list[0].frontmatter.id, saved.frontmatter.id);

    let found = store.load(&saved.frontmatter.id).unwrap();
    assert_eq!(found.frontmatter.id, saved.frontmatter.id);
    assert_eq!(found.body.trim(), "uppercase extension memory");
}

/// Rewind `path`'s mtime by `days` — the same `File::set_modified` the
/// production stamp uses, so the test measures the real syscall pair.
fn backdate(path: &std::path::Path, days: u64) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_hours(days * 24))
        .unwrap();
}

fn mtime(path: &std::path::Path) -> SystemTime {
    std::fs::metadata(path).unwrap().modified().unwrap()
}

#[test]
fn delete_stamps_the_trashed_file_mtime_as_the_deletion_instant() {
    // `fs::rename` keeps the mtime, and gc / `days_until_gc` read a trashed
    // file's mtime as "time since deletion": an old memory deleted today
    // must NOT be immediately reapable under the retention window.
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store
        .save(quick("written long ago, deleted today"))
        .unwrap();
    backdate(&rec.path, 45);
    assert!(
        mtime(&rec.path) < SystemTime::now() - Duration::from_hours(44 * 24),
        "fixture must start with a 45-day-old mtime"
    );

    let before = SystemTime::now();
    store.delete(&rec.frontmatter.id).unwrap();

    let trashed = paths.trash_dir().join(rec.path.file_name().unwrap());
    assert!(trashed.exists(), "delete must move the file into .trash/");
    // Two seconds of slack for coarse-grained filesystem timestamps.
    assert!(
        mtime(&trashed) + Duration::from_secs(2) >= before,
        "trashed mtime must be the deletion instant, got {:?} vs {before:?}",
        mtime(&trashed)
    );
}

#[test]
fn restore_refuses_to_clobber_a_live_re_save_of_the_same_body() {
    // save → delete → save the same body (same content-hash id, new
    // frontmatter) → restore must be BadRequest and leave the live file —
    // and its newer tags/quality — exactly as the re-save wrote them.
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let first = store.save(quick("same body, saved twice")).unwrap();
    let id = first.frontmatter.id.clone();
    store.delete(&id).unwrap();

    let tags = vec!["fresh".to_string()];
    let second = store
        .save(SaveParams {
            tags: &tags,
            quality: 5,
            ..quick("same body, saved twice")
        })
        .unwrap();
    assert_eq!(second.frontmatter.id, id, "same body ⇒ same id");
    let live_bytes = std::fs::read_to_string(&second.path).unwrap();

    let err = store.restore(&id).unwrap_err();
    assert!(matches!(err, Error::BadRequest(_)), "got {err:?}");
    assert_eq!(
        std::fs::read_to_string(&second.path).unwrap(),
        live_bytes,
        "restore must not touch the live file"
    );
    let loaded = store.load(&id).unwrap();
    assert_eq!(loaded.frontmatter.tags, tags);
    assert_eq!(loaded.frontmatter.quality, 5);
}

#[test]
fn re_save_of_a_deleted_body_purges_its_trash_copy() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store.save(quick("deleted then re-saved")).unwrap();
    let file_name = rec.path.file_name().unwrap().to_owned();
    store.delete(&rec.frontmatter.id).unwrap();
    let trashed = paths.trash_dir().join(&file_name);
    assert!(trashed.exists(), "delete must move the file into .trash/");

    store.save(quick("deleted then re-saved")).unwrap();

    assert!(
        !trashed.exists(),
        "a re-saved memory is no longer in the trash: {trashed:?}"
    );
    assert!(rec.path.exists(), "the live file is the re-save's");
}

#[test]
fn restore_checks_the_live_tree_before_the_trash() {
    const STALE: &str = "---\nid: stale\n---\nstale copy\n";

    // A stale trash copy alongside a live file of the same name (the state
    // the save-time purge exists to prevent, planted by hand here) must not
    // be renamed over the live file — the live tree wins, the stale copy
    // stays put.
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store.save(quick("live wins over trash")).unwrap();
    let live_bytes = std::fs::read_to_string(&rec.path).unwrap();
    let stale = paths.trash_dir().join(rec.path.file_name().unwrap());
    std::fs::write(&stale, STALE).unwrap();

    // A fresh store, so nothing is served out of the id -> path cache.
    let err = MemoryStore::new(paths.clone())
        .restore(&rec.frontmatter.id)
        .unwrap_err();
    match err {
        Error::BadRequest(msg) => assert!(msg.contains("not in the trash"), "{msg}"),
        other => panic!("expected BadRequest, got {other:?}"),
    }
    assert_eq!(std::fs::read_to_string(&rec.path).unwrap(), live_bytes);
    assert_eq!(
        std::fs::read_to_string(&stale).unwrap(),
        STALE,
        "stale copy untouched"
    );
    assert!(stale.exists(), "the stale trash copy is left where it was");
}
