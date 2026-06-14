//! Pagination behaviour for `comemory prune` (split from `cli__prune.rs`).
//!
//! Covers the windowed dry-run display of `low_value_memories` and the
//! CRITICAL data-correctness invariant that `--apply` acts on the FULL
//! candidate set even when `--limit` would window the display to fewer.

#[path = "common/cli_prune_support.rs"]
mod support;

use support::{bin, make_prune_eligible, save_memory};
use tempfile::TempDir;

/// Save a memory and immediately doctor it into a prune-eligible state.
/// Returns its id. Bodies must be unique so each save yields a distinct id.
fn seed_low_value(home: &TempDir, body: &str) -> String {
    let id = save_memory(home, body);
    make_prune_eligible(home, &id);
    id
}

/// Count markdown files in `memories/.trash` whose name starts with any of
/// `ids` — i.e. how many of the seeded candidates were soft-deleted.
fn count_trashed(home: &TempDir, ids: &[String]) -> usize {
    let trash = home.path().join(".comemory/memories/.trash");
    std::fs::read_dir(&trash)
        .map(|d| {
            d.filter_map(std::result::Result::ok)
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    ids.iter().any(|id| name.starts_with(id))
                })
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn prune_dry_run_low_value_pagination_window_is_correct() {
    // Seed enough low-value candidates to exceed a small --limit.
    let home = TempDir::new().expect("tempdir");
    let mut ids: Vec<String> = (0..5)
        .map(|i| seed_low_value(&home, &format!("paginated low-value candidate {i}")))
        .collect();
    ids.sort(); // detection returns ids sorted ascending.

    // First page of 2: items = ids[0..2], total = 5, has_more = true.
    let assertion = bin(&home)
        .args(["--json", "prune", "--limit", "2", "--offset", "0"])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    let page = &v["low_value_memories"];
    let items: Vec<&str> = page["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|x| x.as_str().expect("string"))
        .collect();
    assert_eq!(items, vec![ids[0].as_str(), ids[1].as_str()]);
    assert_eq!(page["total"].as_u64(), Some(5));
    assert_eq!(page["limit"].as_u64(), Some(2));
    assert_eq!(page["offset"].as_u64(), Some(0));
    assert_eq!(page["has_more"].as_bool(), Some(true));

    // Last page (offset 4, limit 2): a single item, has_more = false.
    let assertion = bin(&home)
        .args(["--json", "prune", "--limit", "2", "--offset", "4"])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    let page = &v["low_value_memories"];
    let items: Vec<&str> = page["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|x| x.as_str().expect("string"))
        .collect();
    assert_eq!(items, vec![ids[4].as_str()]);
    assert_eq!(page["total"].as_u64(), Some(5));
    assert_eq!(page["has_more"].as_bool(), Some(false));

    // --limit 0 returns ALL candidates in one page, has_more = false.
    let assertion = bin(&home)
        .args(["--json", "prune", "--limit", "0"])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    let page = &v["low_value_memories"];
    let items: Vec<&str> = page["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|x| x.as_str().expect("string"))
        .collect();
    let expected: Vec<&str> = ids.iter().map(String::as_str).collect();
    assert_eq!(items, expected, "--limit 0 must return every candidate");
    assert_eq!(page["total"].as_u64(), Some(5));
    assert_eq!(page["has_more"].as_bool(), Some(false));

    // Dry run must not have deleted anything.
    assert_eq!(
        count_trashed(&home, &ids),
        0,
        "dry-run must not soft-delete"
    );
}

#[test]
fn prune_apply_with_limit_one_soft_deletes_all_candidates() {
    // CRITICAL data-correctness invariant: pagination windows DISPLAY only.
    // `--apply --limit 1` must still soft-delete EVERY low-value candidate,
    // not just the single one that would appear on the page.
    let home = TempDir::new().expect("tempdir");
    let ids: Vec<String> = (0..5)
        .map(|i| seed_low_value(&home, &format!("apply-all candidate {i}")))
        .collect();

    let assertion = bin(&home)
        .args(["--json", "prune", "--apply", "--limit", "1"])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    // The DISPLAY page is windowed to 1 item, but total reflects all 5.
    let page = &v["low_value_memories"];
    assert_eq!(
        page["items"].as_array().expect("items").len(),
        1,
        "display page is windowed to --limit 1"
    );
    assert_eq!(
        page["total"].as_u64(),
        Some(5),
        "total counts all candidates"
    );

    // ...yet ALL five markdown files landed in .trash (full-set delete).
    assert_eq!(
        count_trashed(&home, &ids),
        5,
        "--apply --limit 1 must soft-delete every candidate, not just the page"
    );

    // And every id is gone from the mirror (deleted_at stamped).
    let db = home.path().join(".comemory").join("comemory.db");
    let conn = comemory::store::connection::open(db).expect("open mirror");
    for id in &ids {
        let deleted_at: Option<String> = conn
            .query_row(
                "SELECT deleted_at FROM memories WHERE id = ?1",
                [id.as_str()],
                |r| r.get(0),
            )
            .expect("row present");
        assert!(
            deleted_at.is_some(),
            "candidate {id} must be soft-deleted regardless of --limit"
        );
    }
    drop(conn);

    // Idempotent: a follow-up prune finds nothing left.
    let assertion = bin(&home).args(["--json", "prune"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(v["low_value_memories"]["total"].as_u64(), Some(0));
}

#[test]
fn prune_apply_drops_dangling_co_activated_but_keeps_live_one() {
    // `co_activated` edges point at the `file:`-PREFIXED node id
    // (`file:<repo>:<path>`), unlike `references_file`'s bare form. When a
    // referenced file's `code_symbols` rows are purged the edge dangles;
    // `--apply` must delete the dangling one while leaving the live one (whose
    // dst file still has a `code_symbols` row) untouched.
    use comemory::store::code_row::{self, CodeSymbolRow};

    let home = TempDir::new().expect("tempdir");
    // A save bootstraps the data dir + migrated mirror.
    let mem = save_memory(&home, "co_activated prune fixture body");

    let db = home.path().join(".comemory").join("comemory.db");
    let conn = comemory::store::connection::open(db).expect("open mirror");

    // One LIVE file (`live.rs`) with a real code_symbols row; `gone.rs` has
    // none, so its co_activated edge is dangling.
    code_row::insert(
        &conn,
        &CodeSymbolRow {
            repo: "demo",
            path: "live.rs",
            blob_oid: "oid",
            symbol: "live_fn",
            kind: "function",
            lang: "rust",
            line_start: 1,
            line_end: 10,
            snippet: "fn live_fn() {}",
            simhash: 0,
            parent_id: None,
        },
    )
    .expect("insert live code symbol");
    // Register `live.rs` in `indexed_files` so the stale-code cleanup (which
    // runs in the SAME `--apply` transaction, BEFORE the co_activated sweep)
    // does not purge its `code_symbols` row out from under the live edge.
    conn.execute(
        "INSERT INTO indexed_files(repo, path, blob_oid, indexed_at) \
         VALUES('demo','live.rs','oid','t')",
        [],
    )
    .expect("register live indexed_file");

    // Seed both co_activated edges directly (the reinforcement writer uses the
    // `file:`-prefixed dst grammar — match it here).
    for dst in ["file:demo:live.rs", "file:demo:gone.rs"] {
        conn.execute(
            "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,weight,created_at) \
             VALUES('memory',?1,'file',?2,'co_activated',3,'t')",
            rusqlite::params![mem, dst],
        )
        .expect("seed co_activated edge");
    }
    drop(conn);

    bin(&home)
        .args(["--json", "prune", "--apply"])
        .assert()
        .success();

    // Re-open and assert: the live edge survives, the dangling one is gone.
    let db = home.path().join(".comemory").join("comemory.db");
    let conn = comemory::store::connection::open(db).expect("reopen mirror");
    let remaining: Vec<String> = conn
        .prepare("SELECT dst_id FROM edges WHERE rel = 'co_activated' ORDER BY dst_id")
        .expect("prepare")
        .query_map([], |r| r.get::<_, String>(0))
        .expect("query")
        .filter_map(std::result::Result::ok)
        .collect();
    assert_eq!(
        remaining,
        vec!["file:demo:live.rs".to_string()],
        "dangling co_activated edge must be pruned, live one kept"
    );
}
