#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Contract tests for `comemory backup create|restore|merge-erasures`
//! (#256, B-4), through the real binary over real data directories: the
//! backup directory and its descriptor, the restore's confirm gate, a real
//! `SIGKILL` landing mid-swap and the rerun that finishes it, a snapshot with
//! another page size refused, and a manifest that is not established.

#[path = "common/cli_bin.rs"]
mod cli_bin;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin;
use cli_bin::CliHome;
use serde_json::{Value, json};

/// `(exit code, stdout, stderr)` of `comemory <args>` over `home`.
fn run(home: &CliHome, args: &[&str]) -> (i32, String, String) {
    let out = home.bin().args(args).output().expect("run comemory");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn save(home: &CliHome, body: &str) -> String {
    let saved = home.run_json(&["save", body, "--kind", "note"]);
    saved["id"].as_str().expect("saved id").to_string()
}

fn live_ids(home: &CliHome) -> Vec<String> {
    let listed = home.run_json(&["list"]);
    let mut ids: Vec<String> = listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|m| m["id"].as_str().expect("id").to_string())
        .collect();
    ids.sort();
    ids
}

/// The stream epoch the live database carries.
fn epoch_of(home: &CliHome) -> String {
    rusqlite::Connection::open(home.data_dir().join("comemory.db"))
        .expect("open")
        .query_row("SELECT epoch FROM replica_stream", [], |r| r.get(0))
        .expect("epoch")
}

fn utf8(path: &Path) -> &str {
    path.to_str().expect("utf8 path")
}

#[test]
fn create_writes_a_backup_directory_with_its_descriptor() {
    let home = CliHome::new();
    let id = save(&home, "The backup contract copies this memory whole.");
    let out = tempfile::tempdir().expect("out");
    let dir = out.path().join("snapshot");

    let created = home.run_json(&["backup", "create", "--out", utf8(&dir)]);

    assert_eq!(created["dir"], json!(utf8(&dir)));
    assert_eq!(created["memory_files"], json!(1));
    for key in ["created_at", "binary_version", "schema_markers", "epoch"] {
        assert!(!created[key].is_null(), "{key}: {created}");
    }
    let descriptor: Value =
        serde_json::from_slice(&std::fs::read(dir.join("backup.json")).expect("backup.json"))
            .expect("json");
    assert_eq!(descriptor["epoch"], created["epoch"]);
    assert!(dir.join("comemory.db").is_file());
    assert!(
        std::fs::read_dir(dir.join("memories"))
            .expect("memories copied")
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with(&id))
    );

    let text = home.run_ok(&["backup", "create"]);
    assert!(text.starts_with("backed up to "), "{text}");
    let backups = home.data_dir().join("backups");
    assert_eq!(
        std::fs::read_dir(&backups).expect("default dir").count(),
        1,
        "the default lands under <data_dir>/backups/<timestamp>/"
    );
}

#[test]
fn create_without_a_database_is_refused_and_creates_none() {
    let home = CliHome::new();
    let (code, _, stderr) = run(&home, &["backup", "create"]);
    assert_eq!(code, 69, "{stderr}");
    assert!(stderr.contains("no database"), "{stderr}");
    assert!(!home.data_dir().join("comemory.db").exists());
}

#[test]
fn restore_without_confirm_is_refused_and_changes_nothing() {
    let home = CliHome::new();
    let id = save(&home, "A memory the refused restore must leave alone.");
    let out = tempfile::tempdir().expect("out");
    let dir = out.path().join("snapshot");
    home.run_json(&["backup", "create", "--out", utf8(&dir)]);
    let later = save(&home, "A memory written after the backup.");

    let (code, _, stderr) = run(&home, &["backup", "restore", utf8(&dir)]);

    assert_eq!(code, 70, "{stderr}");
    assert!(stderr.contains("--confirm"), "{stderr}");
    let mut expected = vec![id, later];
    expected.sort();
    assert_eq!(live_ids(&home), expected);
}

/// Wait until the restore running over `data` has moved the markdown and
/// is copying the database in place — where the held write lock parks it.
fn wait_for_swap(data: &Path) {
    let deadline = Instant::now() + Duration::from_mins(2);
    let pending = data.join("restore.pending");
    while Instant::now() < deadline {
        let swapping = std::fs::read(&pending)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|record| record["phase"] == "swapping");
        if swapping && !data.join("memories.restore").exists() {
            // Give the process time to reach the copy's first step.
            std::thread::sleep(Duration::from_millis(300));
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("the restore never reached its swap");
}

#[test]
fn a_restore_killed_mid_swap_finishes_on_rerun() {
    let home = CliHome::new();
    let kept = save(&home, "A memory the backup holds, restored after the kill.");
    let out = tempfile::tempdir().expect("out");
    let dir = out.path().join("snapshot");
    home.run_json(&["backup", "create", "--out", utf8(&dir)]);
    save(
        &home,
        "A memory written after the backup, gone once restored.",
    );
    let data = home.data_dir();

    // A real writer holds the live file, so the restore parks inside its
    // in-place copy — after the markdown moved, before the database did.
    let blocker = rusqlite::Connection::open(data.join("comemory.db")).expect("blocker");
    blocker
        .execute_batch(
            "BEGIN IMMEDIATE; UPDATE schema_meta SET value = value WHERE key = 'version';",
        )
        .expect("hold the write lock");
    let mut child = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", &data)
        .env("COMEMORY_SYNC_PAUSE_WAIT", "300s")
        .args(["--json", "backup", "restore", utf8(&dir), "--confirm"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the restore");
    wait_for_swap(&data);
    child.kill().expect("SIGKILL mid-swap");
    let status = child.wait().expect("reap");
    assert!(!status.success(), "killed, not finished: {status:?}");
    blocker.execute_batch("ROLLBACK;").expect("release");
    drop(blocker);

    assert!(
        data.join("restore.pending").exists(),
        "the record survives the kill"
    );
    assert!(
        data.join("memories.pre-restore").exists() && !data.join("memories.restore").exists(),
        "the killed swap had moved the markdown"
    );

    let finished = home.run_json(&["backup", "restore", utf8(&dir), "--confirm"]);

    assert_eq!(finished["resumed"], json!(true), "{finished}");
    assert!(!data.join("restore.pending").exists());
    assert!(!data.join("comemory.db.restore.tmp").exists());
    assert_eq!(
        live_ids(&home),
        vec![kept],
        "the backup's content, whole: what it predates is gone"
    );
    let text = home.run_ok(&["backup", "restore", utf8(&dir), "--confirm"]);
    assert!(text.starts_with("restored "), "{text}");
}

/// A copy of the backup at `dir` whose database uses 8192-byte pages — a
/// real comemory database, re-paged through SQLite itself.
fn other_page_size(dir: &Path, into: &Path) -> PathBuf {
    std::fs::create_dir_all(into).expect("dir");
    let db = into.join("comemory.db");
    std::fs::copy(dir.join("comemory.db"), &db).expect("copy");
    let conn = comemory::store::connection::open(&db).expect("open the copy");
    conn.execute_batch("PRAGMA journal_mode = DELETE; PRAGMA page_size = 8192; VACUUM;")
        .expect("re-page");
    let size: i64 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .expect("page size");
    assert_eq!(size, 8192, "the fixture really has another page size");
    db
}

#[test]
fn a_snapshot_with_another_page_size_is_refused_naming_both() {
    let home = CliHome::new();
    save(&home, "A memory the refused restore must leave alone.");
    let out = tempfile::tempdir().expect("out");
    let dir = out.path().join("snapshot");
    home.run_json(&["backup", "create", "--out", utf8(&dir)]);
    let other = out.path().join("other-pages");
    let other_db = other_page_size(&dir, &other);
    let before = epoch_of(&home);

    let (code, _, stderr) = run(&home, &["backup", "restore", utf8(&other), "--confirm"]);

    assert_eq!(code, 64, "{stderr}");
    assert!(
        stderr.contains("8192") && stderr.contains("4096"),
        "{stderr}"
    );
    let live = home.data_dir().join("comemory.db");
    assert!(
        stderr.contains(utf8(&other_db)) && stderr.contains(utf8(&live)),
        "names both files: {stderr}"
    );
    assert!(!home.data_dir().join("restore.pending").exists());
    assert_eq!(epoch_of(&home), before, "nothing changed");
}

#[test]
fn merge_erasures_refuses_a_manifest_that_is_not_established() {
    let home = CliHome::new();
    let id = save(&home, "A memory erased so the manifest holds a line.");
    home.run_json(&["erase", "--memory", &id, "--confirm"]);
    let manifest = home.data_dir().join("replica/erasures.jsonl");
    let bytes = std::fs::read(&manifest).expect("the erase wrote the manifest");
    let torn = home.data_dir().join("torn.jsonl");
    std::fs::write(&torn, &bytes[..bytes.len() - 10]).expect("tear it");

    let (code, _, stderr) = run(&home, &["backup", "merge-erasures", utf8(&torn)]);
    assert_eq!(code, 75, "{stderr}");
    assert!(stderr.contains("not an established"), "{stderr}");
    assert_eq!(std::fs::read(&manifest).expect("bytes"), bytes, "untouched");

    let (code, _, stderr) = run(&home, &["backup", "merge-erasures", "/no/such/file.jsonl"]);
    assert_eq!(code, 64, "{stderr}");

    let merged = home.run_json(&["backup", "merge-erasures", utf8(&manifest)]);
    assert_eq!(merged["lines"], json!(1), "{merged}");
    assert_eq!(merged["cleared"], Value::Null, "nothing was unverified");
}

#[test]
fn restore_reads_the_named_erasure_manifest() {
    let home = CliHome::new();
    let id = save(
        &home,
        "A memory erased after the backup, its manifest moved.",
    );
    let out = tempfile::tempdir().expect("out");
    let dir = out.path().join("snapshot");
    home.run_json(&["backup", "create", "--out", utf8(&dir)]);
    home.run_json(&["erase", "--memory", &id, "--confirm"]);
    let moved = out.path().join("erasures.jsonl");
    std::fs::rename(home.data_dir().join("replica/erasures.jsonl"), &moved).expect("move");

    let restored = home.run_json(&[
        "backup",
        "restore",
        utf8(&dir),
        "--confirm",
        "--erasure-manifest",
        utf8(&moved),
    ]);

    assert_eq!(restored["restore_state"], Value::Null, "{restored}");
    assert_eq!(restored["erasures_merged"], json!(1), "{restored}");
    assert!(live_ids(&home).is_empty(), "the erase stays merged");
}
