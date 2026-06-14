//! `comemory prune` operates against the v0.2 SQLite mirror
//! (`comemory.db`). It reports orphan edges (memory→… edges whose
//! source memory is missing or soft-deleted), stale code files (paths
//! referenced from `code_symbols` that no longer appear in
//! `indexed_files`), and low-value memories (signal-based detection
//! from `prune::low_value`).
//!
//! On a freshly-initialised DB all lists are empty and the default
//! (dry-run) mode must not mutate anything. `--apply` soft-deletes
//! flagged low-value memories through the same path as `comemory
//! delete`.

use assert_cmd::Command;
use tempfile::TempDir;

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

#[test]
fn prune_dry_run_on_clean_db_emits_zero_counts() {
    let home = TempDir::new().expect("tempdir");
    let assertion = bin(&home).args(["--json", "prune"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(v["orphan_edges"].as_i64(), Some(0));
    let stale: Vec<&str> = v["stale_code_files"]
        .as_array()
        .expect("stale_code_files is array")
        .iter()
        .map(|x| x.as_str().expect("string entry"))
        .collect();
    assert!(stale.is_empty(), "expected no stale code files: {stale:?}");
    let low_value = v["low_value_memories"]
        .as_array()
        .expect("low_value_memories is array");
    assert!(
        low_value.is_empty(),
        "expected no low-value memories: {low_value:?}"
    );
}

/// Save a memory via the real binary and return its id from the JSON
/// output.
fn save_memory(home: &TempDir, body: &str) -> String {
    let assertion = bin(home)
        .args(["--json", "save", body, "--kind", "note"])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse save JSON");
    v["id"].as_str().expect("save emits id").to_string()
}

/// Make the memory prune-eligible by doctoring the mirror row: drop the
/// quality to 2 and back-date `last_accessed` so the activation falls
/// below the default −2.0 floor. Saves carry no feedback row, so the
/// Beta posterior sits exactly at the 0.25 ceiling (inclusive).
fn make_prune_eligible(home: &TempDir, id: &str) {
    let db = home.path().join(".comemory").join("comemory.db");
    let conn = comemory::store::connection::open(db).expect("open mirror");
    conn.execute(
        "UPDATE memories SET quality = 2, last_accessed = '2025-01-01T00:00:00Z' WHERE id = ?1",
        [id],
    )
    .expect("doctor row");
}

#[test]
fn prune_dry_run_reports_low_value_memory_without_deleting() {
    let home = TempDir::new().expect("tempdir");
    let id = save_memory(&home, "stale prune candidate body");
    make_prune_eligible(&home, &id);

    // No --apply: the default mode must scan + report only.
    let assertion = bin(&home).args(["--json", "prune"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    let flagged: Vec<&str> = v["low_value_memories"]
        .as_array()
        .expect("low_value_memories is array")
        .iter()
        .map(|x| x.as_str().expect("string entry"))
        .collect();
    assert_eq!(flagged, vec![id.as_str()]);

    // Dry run must not touch the markdown source of truth.
    let trash = home.path().join(".comemory/memories/.trash");
    let trashed = std::fs::read_dir(&trash)
        .map(|d| d.count())
        .unwrap_or_default();
    assert_eq!(trashed, 0, "default dry-run must not move files to .trash");
}

#[test]
fn prune_apply_soft_deletes_low_value_memory() {
    let home = TempDir::new().expect("tempdir");
    let id = save_memory(&home, "doomed prune candidate body");
    make_prune_eligible(&home, &id);

    let assertion = bin(&home)
        .args(["--json", "prune", "--apply"])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(
        v["low_value_memories"][0].as_str(),
        Some(id.as_str()),
        "apply-mode report must still list the flagged id"
    );

    // Markdown moved into .trash/ (soft delete, same path as `delete`).
    let memories = home.path().join(".comemory/memories");
    let live = std::fs::read_dir(&memories)
        .expect("read memories dir")
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(&id))
        .count();
    assert_eq!(live, 0, "markdown must leave memories/");
    let trashed = std::fs::read_dir(memories.join(".trash"))
        .expect("read .trash")
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(&id))
        .count();
    assert_eq!(trashed, 1, "markdown must land in .trash/");

    // Gone from `comemory list`.
    let assertion = bin(&home).args(["--json", "list"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        !stdout.contains(&id),
        "soft-deleted memory must not appear in list output: {stdout}"
    );

    // Idempotent: a second apply-mode prune finds nothing left to flag.
    let assertion = bin(&home)
        .args(["--json", "prune", "--apply"])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(
        v["low_value_memories"].as_array().expect("array").len(),
        0,
        "second prune must report no low-value memories"
    );
}

#[test]
fn prune_apply_heals_half_deleted_memory_instead_of_wedging() {
    // Wedge state: live `memories` row, markdown file already gone —
    // producible by a crash inside `comemory delete` between its file
    // move and its DB transaction. Prune --apply must not abort on the
    // NotFound: it heals the DB mirror, still processes every other
    // flagged id, and emits the full report.
    let home = TempDir::new().expect("tempdir");
    let wedged = save_memory(&home, "wedged half-deleted body");
    let normal = save_memory(&home, "normal prune candidate body");
    make_prune_eligible(&home, &wedged);
    make_prune_eligible(&home, &normal);

    // Doctor the wedge: remove the markdown file but keep the DB row live.
    let memories = home.path().join(".comemory/memories");
    let md = std::fs::read_dir(&memories)
        .expect("read memories dir")
        .filter_map(std::result::Result::ok)
        .find(|e| e.file_name().to_string_lossy().starts_with(&wedged))
        .expect("wedged markdown file exists");
    std::fs::remove_file(md.path()).expect("remove wedged markdown");

    let assertion = bin(&home)
        .args(["--json", "prune", "--apply"])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    let mut flagged: Vec<&str> = v["low_value_memories"]
        .as_array()
        .expect("low_value_memories is array")
        .iter()
        .map(|x| x.as_str().expect("string entry"))
        .collect();
    flagged.sort_unstable();
    let mut expected = vec![wedged.as_str(), normal.as_str()];
    expected.sort_unstable();
    assert_eq!(flagged, expected, "report must list both flagged ids");

    // The normal candidate went through the full soft-delete path.
    let trashed = std::fs::read_dir(memories.join(".trash"))
        .expect("read .trash")
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(&normal))
        .count();
    assert_eq!(trashed, 1, "normal candidate must land in .trash/");

    // The wedged row was healed: deleted_at stamped despite the missing
    // markdown.
    let db = home.path().join(".comemory").join("comemory.db");
    let conn = comemory::store::connection::open(db).expect("open mirror");
    let deleted_at: Option<String> = conn
        .query_row(
            "SELECT deleted_at FROM memories WHERE id = ?1",
            [wedged.as_str()],
            |r| r.get(0),
        )
        .expect("wedged row still present");
    assert!(
        deleted_at.is_some(),
        "wedged row must be stamped deleted_at"
    );
    drop(conn);

    // And the wedge is gone for good: a follow-up prune flags nothing.
    let assertion = bin(&home)
        .args(["--json", "prune", "--apply"])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(
        v["low_value_memories"].as_array().expect("array").len(),
        0,
        "healed wedge must not be re-flagged"
    );
}

#[test]
fn prune_dry_run_after_save_is_idempotent() {
    // Saving a memory creates `memory→{repo,author}` edges via the v0.2
    // mirror. Those edges are live (the source memory exists with
    // deleted_at IS NULL) so default-mode prune must report 0 orphans and
    // a follow-up doctor invocation must still succeed.
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["save", "prune dry-run body", "--kind", "note"])
        .assert()
        .success();
    let assertion = bin(&home).args(["--json", "prune"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(v["orphan_edges"].as_i64(), Some(0));
}

#[test]
fn prune_examples_block_present() {
    // The shared `cli_help_examples` test expects every subcommand --help
    // to ship an `Examples:` block. Asserted directly here as a guard
    // against future EXAMPLES drift inside this command specifically.
    let help = Command::cargo_bin("comemory")
        .expect("bin")
        .args(["prune", "--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(help).expect("utf8");
    assert!(
        text.contains("Examples:"),
        "prune --help must contain Examples block: {text:?}"
    );
}
