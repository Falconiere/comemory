#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory save` — part 3: the content-addressed replay contract
//! (Falconiere/comemory#131). A byte-identical body re-saved lands on the
//! same id with `created: false`, overwrites metadata last-writer-wins,
//! keeps the markdown `created`, revives a trashed id, survives `rebuild`,
//! and a same-id different-body save is refused before any write.

// Only `save_json` / `count_md_files` are needed here; the near-dup fixture
// bodies the shared module also carries stay with `cli__save{,_2}.rs`.
#[path = "common/cli_save_support.rs"]
#[allow(dead_code)]
mod support;

use assert_cmd::Command;
use comemory::memory::Frontmatter;
use comemory::memory::id::memory_id;
use comemory::store::connection;
use std::fs;
use support::{count_md_files, save_json};
use tempfile::tempdir;

const BODY: &str = "advisory locks serialize migrations in postgres";

/// Two real bodies whose SHA-256 digests share their first 4 bytes.
const COLLIDE_A: &str = "collision probe 14565";
const COLLIDE_B: &str = "collision probe 24048";
const COLLIDE_ID: &str = "0adf80f7";

/// The `comemory` binary bound to `home`'s data dir.
fn bin(home: &tempfile::TempDir) -> Command {
    let mut cmd = Command::cargo_bin("comemory").expect("bin");
    cmd.env("COMEMORY_DATA_DIR", home.path());
    cmd
}

/// Parse the frontmatter of the memory file at `path`.
fn frontmatter_at(path: &str) -> Frontmatter {
    let raw = fs::read_to_string(path).expect("read memory file");
    Frontmatter::split(&raw).expect("split frontmatter").0
}

/// `(count, created_at, deleted_at, kind, repo)` of the `memories` row for
/// `id`; `repo` is `NULL` in the mirror when the save passed none.
fn memory_row(
    home: &tempfile::TempDir,
    id: &str,
) -> (i64, String, Option<String>, String, Option<String>) {
    let conn = connection::open(home.path().join("comemory.db")).expect("open db");
    conn.query_row(
        "SELECT count(*), min(created_at), min(deleted_at), min(kind), min(repo) \
         FROM memories WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )
    .expect("memories row")
}

/// The `memory_tags` rows for `id`, sorted.
fn tags_of(home: &tempfile::TempDir, id: &str) -> Vec<String> {
    let conn = connection::open(home.path().join("comemory.db")).expect("open db");
    let mut stmt = conn
        .prepare("SELECT tag FROM memory_tags WHERE memory_id = ?1 ORDER BY tag")
        .expect("prepare");
    stmt.query_map([id], |r| r.get(0))
        .expect("query")
        .collect::<Result<Vec<String>, _>>()
        .expect("tags")
}

#[test]
fn identical_resave_reports_created_false_and_keeps_one_memory() {
    let home = tempdir().expect("tempdir");

    let first = save_json(&home, BODY);
    assert_eq!(first["created"], serde_json::json!(true), "{first}");
    let id = first["id"].as_str().expect("id").to_string();

    // Replay with a trailing newline: `trim_end` makes it the same body.
    let replay = save_json(&home, &format!("{BODY}\n"));
    assert_eq!(replay["id"].as_str(), Some(id.as_str()));
    assert_eq!(replay["created"], serde_json::json!(false), "{replay}");
    assert_eq!(replay["path"], first["path"]);
    assert_eq!(count_md_files(home.path()), 1);
    assert_eq!(memory_row(&home, &id).0, 1, "one row, not two");

    // A third save with different metadata: still one memory, metadata
    // overwritten last-writer-wins in both the file and the mirror.
    // `save_json_args` pins `--kind note`, so drive the binary directly.
    let out = bin(&home)
        .args([
            "--json", "save", "--kind", "decision", "--repo", "other", "--tags", "x,y", BODY,
        ])
        .assert()
        .success();
    let third: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.get_output().stdout).trim())
            .expect("save json");
    assert_eq!(third["id"].as_str(), Some(id.as_str()));
    assert_eq!(third["created"], serde_json::json!(false), "{third}");
    assert_eq!(count_md_files(home.path()), 1);
    let fm = frontmatter_at(third["path"].as_str().expect("path"));
    assert_eq!(fm.kind.as_str(), "decision");
    assert_eq!(fm.repo, "other");
    assert_eq!(fm.tags, vec!["x".to_string(), "y".to_string()]);
    let (count, _, _, kind, repo) = memory_row(&home, &id);
    assert_eq!(
        (count, kind.as_str(), repo.as_deref()),
        (1, "decision", Some("other"))
    );
    assert_eq!(tags_of(&home, &id), vec!["x".to_string(), "y".to_string()]);
}

#[test]
fn replay_then_rebuild_keeps_the_original_created() {
    let home = tempdir().expect("tempdir");
    let first = save_json(&home, BODY);
    let id = first["id"].as_str().expect("id").to_string();
    let path = first["path"].as_str().expect("path").to_string();
    let created = frontmatter_at(&path).created;
    let (_, created_at, _, _, _) = memory_row(&home, &id);

    save_json(&home, BODY);
    assert_eq!(
        frontmatter_at(&path).created,
        created,
        "a replay must not re-stamp the markdown created"
    );
    assert_eq!(
        memory_row(&home, &id).1,
        created_at,
        "mirror created_at untouched"
    );

    // A rebuild re-mirrors from markdown: it must find — and keep — the
    // original creation instant, not a re-stamped one.
    bin(&home).arg("rebuild").assert().success();
    assert_eq!(frontmatter_at(&path).created, created);
    assert_eq!(
        memory_row(&home, &id).1,
        created_at,
        "rebuild adopted the original"
    );
}

#[test]
fn replay_of_a_trashed_body_revives_it_and_reports_created_false() {
    let home = tempdir().expect("tempdir");
    let first = save_json(&home, BODY);
    let id = first["id"].as_str().expect("id").to_string();
    let path = first["path"].as_str().expect("path").to_string();
    let created = frontmatter_at(&path).created;

    bin(&home).args(["delete", &id]).assert().success();
    let trash_dir = home.path().join("memories").join(".trash");
    assert_eq!(fs::read_dir(&trash_dir).expect("trash").count(), 1);
    assert!(
        memory_row(&home, &id).2.is_some(),
        "soft-deleted in the mirror"
    );

    let replay = save_json(&home, BODY);
    assert_eq!(replay["id"].as_str(), Some(id.as_str()));
    assert_eq!(
        replay["created"],
        serde_json::json!(false),
        "a revived memory existed before: {replay}"
    );
    assert!(path_exists(&path), "live file is back");
    assert_eq!(
        fs::read_dir(&trash_dir).expect("trash").count(),
        0,
        "trash copy purged"
    );
    let (count, _, deleted_at, _, _) = memory_row(&home, &id);
    assert_eq!((count, deleted_at), (1, None), "row is live again");
    assert_eq!(
        frontmatter_at(&path).created,
        created,
        "created carried over from the trash copy"
    );
}

fn path_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

#[test]
fn colliding_body_is_refused_before_any_write() {
    assert_eq!(memory_id(COLLIDE_A), COLLIDE_ID);
    assert_eq!(memory_id(COLLIDE_B), COLLIDE_ID);
    let home = tempdir().expect("tempdir");
    let first = save_json(&home, COLLIDE_A);
    let path = first["path"].as_str().expect("path").to_string();
    let bytes_before = fs::read(&path).expect("read first");

    let assertion = bin(&home)
        .args(["--json", "save", "--kind", "note", COLLIDE_B])
        .assert()
        .code(65);
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains(COLLIDE_ID),
        "stderr names the id: {stderr:?}"
    );
    assert!(
        stderr.contains("different body"),
        "stderr names the cause: {stderr:?}"
    );

    assert_eq!(count_md_files(home.path()), 1, "no second file");
    assert_eq!(
        fs::read(&path).expect("re-read"),
        bytes_before,
        "first file untouched"
    );
    let conn = connection::open(home.path().join("comemory.db")).expect("open db");
    let stored: String = conn
        .query_row(
            "SELECT body FROM memories WHERE id = ?1",
            [COLLIDE_ID],
            |r| r.get(0),
        )
        .expect("row");
    assert_eq!(stored, COLLIDE_A, "mirror row still holds the first body");
}

#[test]
fn resave_tty_says_updated() {
    let home = tempdir().expect("tempdir");
    let first = bin(&home).args(["save", BODY]).assert().success();
    let stdout = String::from_utf8_lossy(&first.get_output().stdout).to_string();
    assert!(stdout.starts_with("saved "), "insert: {stdout:?}");

    let replay = bin(&home).args(["save", BODY]).assert().success();
    let stdout = String::from_utf8_lossy(&replay.get_output().stdout).to_string();
    assert!(stdout.starts_with("updated "), "replay: {stdout:?}");
    assert!(
        stdout.contains("path:"),
        "path line still printed: {stdout:?}"
    );
}
