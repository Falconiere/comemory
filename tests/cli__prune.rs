#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
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

#[path = "common/cli_prune_support.rs"]
mod support;

use support::{bin, make_prune_eligible, save_memory};
use tempfile::TempDir;

#[test]
fn prune_dry_run_on_clean_db_emits_zero_counts() {
    let home = TempDir::new().expect("tempdir");
    let assertion = bin(&home).args(["--json", "prune"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(v["orphan_edges"].as_i64(), Some(0));
    // Each list is now a `Page` envelope: { items, limit, offset, total, has_more }.
    let stale: Vec<&str> = v["stale_code_files"]["items"]
        .as_array()
        .expect("stale_code_files.items is array")
        .iter()
        .map(|x| x.as_str().expect("string entry"))
        .collect();
    assert!(stale.is_empty(), "expected no stale code files: {stale:?}");
    assert_eq!(v["stale_code_files"]["total"].as_u64(), Some(0));
    assert_eq!(v["stale_code_files"]["has_more"].as_bool(), Some(false));
    let low_value = v["low_value_memories"]["items"]
        .as_array()
        .expect("low_value_memories.items is array");
    assert!(
        low_value.is_empty(),
        "expected no low-value memories: {low_value:?}"
    );
    assert_eq!(v["low_value_memories"]["total"].as_u64(), Some(0));
    assert_eq!(v["low_value_memories"]["has_more"].as_bool(), Some(false));
    assert_eq!(v["trash_count"].as_u64(), Some(0));
    assert_eq!(v["reclaimable_bytes"].as_u64(), Some(0));
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
    let items = v["low_value_memories"]["items"]
        .as_array()
        .expect("low_value_memories.items is array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"].as_str(), Some(id.as_str()));
    assert_eq!(items[0]["reason"].as_str(), Some("low value"));
    assert!(items[0]["activation"].is_number(), "row: {:?}", items[0]);
    assert!(items[0]["age_days"].is_u64(), "row: {:?}", items[0]);
    assert_eq!(
        items[0]["title"].as_str(),
        Some("stale prune candidate body")
    );
    assert_eq!(v["low_value_memories"]["total"].as_u64(), Some(1));

    // Dry run must not touch the markdown source of truth.
    let trash = home.path().join(".comemory/memories/.trash");
    let trashed = std::fs::read_dir(&trash)
        .map(std::iter::Iterator::count)
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
        v["low_value_memories"]["items"][0]["id"].as_str(),
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
        v["low_value_memories"]["items"]
            .as_array()
            .expect("array")
            .len(),
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
    let mut flagged: Vec<&str> = v["low_value_memories"]["items"]
        .as_array()
        .expect("low_value_memories.items is array")
        .iter()
        .map(|x| x["id"].as_str().expect("id field"))
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
        v["low_value_memories"]["items"]
            .as_array()
            .expect("array")
            .len(),
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
fn prune_reports_trash_count_and_reclaimable_bytes_from_real_deletes() {
    // AC-31: trash_count / reclaimable_bytes are corpus-level totals over
    // memories/.trash/, driven here by a REAL `comemory delete` soft-delete
    // (not a hand-written trash file) — stat the trash directory ourselves
    // and compare against the report.
    let home = TempDir::new().expect("tempdir");
    let deleted = save_memory(&home, "will be soft-deleted via comemory delete");
    let kept = save_memory(&home, "stays live prune candidate body");
    make_prune_eligible(&home, &kept);

    bin(&home).args(["delete", &deleted]).assert().success();

    let trash = home.path().join(".comemory/memories/.trash");
    let real_files: Vec<std::path::PathBuf> = std::fs::read_dir(&trash)
        .expect("read .trash")
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .collect();
    let real_count = real_files.len() as u64;
    let real_bytes: u64 = real_files
        .iter()
        .map(|p| std::fs::metadata(p).expect("stat trash file").len())
        .sum();
    assert!(
        real_count >= 1,
        "expected the soft-deleted file in .trash/: {real_files:?}"
    );

    let assertion = bin(&home).args(["--json", "prune"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");

    assert_eq!(v["trash_count"].as_u64(), Some(real_count));
    assert_eq!(v["reclaimable_bytes"].as_u64(), Some(real_bytes));

    // The still-live prune candidate is unaffected by the unrelated delete.
    let items = v["low_value_memories"]["items"]
        .as_array()
        .expect("items array");
    assert!(
        items
            .iter()
            .any(|row| row["id"].as_str() == Some(kept.as_str())),
        "expected {kept} among low_value_memories: {items:?}"
    );
}
