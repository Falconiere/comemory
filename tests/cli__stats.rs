#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory stats` driven as a real subprocess (spec AC-1, AC-2).
//!
//! The corpus is built by the real `comemory save` — not by hand-inserted
//! rows — so this exercises the same write path a user's memories take, and
//! the counters are checked against the same facts counted independently
//! (the markdown directory listing, a direct SQL count).

use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;
use tempfile::TempDir;

/// Run `comemory` with the temp data dir, returning stdout on success.
fn comemory(data_dir: &Path, args: &[&str]) -> String {
    let out = Command::cargo_bin("comemory")
        .unwrap()
        .args(args)
        .env("COMEMORY_DATA_DIR", data_dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "comemory {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn stats_json(data_dir: &Path, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["stats", "--json"];
    args.extend_from_slice(extra);
    serde_json::from_str(&comemory(data_dir, &args)).expect("stats --json parses")
}

/// Save three real memories, two in `comemory` and one in `toolu`, and
/// return the id of the first.
fn seed(data_dir: &Path) -> String {
    let out = comemory(
        data_dir,
        &[
            "save",
            "the ranker reads frontmatter, never the body",
            "--kind",
            "decision",
            "--repo",
            "comemory",
            "--json",
        ],
    );
    let first: serde_json::Value = serde_json::from_str(&out).expect("save --json parses");
    let id = first["id"]
        .as_str()
        .expect("save reports an id")
        .to_string();

    comemory(
        data_dir,
        &[
            "save",
            "an empty tag list round-tripped as null and failed to parse",
            "--kind",
            "bug",
            "--repo",
            "comemory",
        ],
    );
    comemory(
        data_dir,
        &[
            "save",
            "co-change edges decay faster than import edges",
            "--kind",
            "discovery",
            "--repo",
            "toolu",
        ],
    );
    id
}

#[test]
fn stats_counts_a_real_corpus_and_agrees_with_the_filesystem() {
    let dir = TempDir::new().unwrap();
    seed(dir.path());

    let s = stats_json(dir.path(), &[]);

    assert_eq!(s["memories"], 3);
    assert_eq!(s["trashed"], 0);
    assert_eq!(s["markdown_files"], 3);

    let on_disk = std::fs::read_dir(dir.path().join("memories"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .count();
    assert_eq!(
        s["markdown_files"].as_u64().unwrap() as usize,
        on_disk,
        "markdown_files must equal what is actually on disk"
    );

    assert!(
        s["db_bytes"].as_u64().unwrap() > 0,
        "a migrated database has a non-zero page product"
    );
    assert_eq!(
        s["schema_version"],
        comemory::store::migrate::CURRENT_VERSION
    );
}

#[test]
fn soft_delete_moves_a_memory_from_live_to_trashed_without_changing_markdown_count() {
    let dir = TempDir::new().unwrap();
    let id = seed(dir.path());

    let before = stats_json(dir.path(), &[]);
    assert_eq!(before["memories"], 3);

    comemory(dir.path(), &["delete", &id]);
    let after = stats_json(dir.path(), &[]);

    assert_eq!(after["memories"], 2, "the live count drops");
    assert_eq!(after["trashed"], 1, "the row is soft-deleted, not gone");
    assert_eq!(
        after["markdown_files"], 2,
        "the file moved to .trash/, which markdown_files does not count"
    );
}

#[test]
fn repo_scopes_the_memory_counter() {
    let dir = TempDir::new().unwrap();
    seed(dir.path());

    let scoped = stats_json(dir.path(), &["--repo", "comemory"]);
    assert_eq!(scoped["memories"], 2);

    let other = stats_json(dir.path(), &["--repo", "toolu"]);
    assert_eq!(other["memories"], 1);
}

#[test]
fn stats_on_an_empty_data_dir_reports_unknown_and_creates_no_database() {
    let dir = TempDir::new().unwrap();

    let s = stats_json(dir.path(), &[]);

    assert_eq!(s["memories"], 0);
    assert_eq!(s["db_bytes"], 0);
    assert_eq!(s["schema_version"], "unknown");
    assert!(
        !dir.path().join("comemory.db").exists(),
        "a read command must not materialize a database"
    );
}

#[test]
fn the_tty_view_renders_every_counter() {
    let dir = TempDir::new().unwrap();
    seed(dir.path());

    let text = comemory(dir.path(), &["stats"]);

    for label in [
        "memories",
        "trashed",
        "markdown",
        "code symbols",
        "documents",
        "graph edges",
        "repos",
        "comemory.db",
        "schema",
    ] {
        assert!(
            text.contains(label),
            "TTY output is missing {label:?}:\n{text}"
        );
    }
}
