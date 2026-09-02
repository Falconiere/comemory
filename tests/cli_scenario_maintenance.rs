#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Maintenance journey: consolidate near-dups → prune a low-value memory
//! → rebuild → search still finds the keeper → gc reports.

#[path = "common/cli_bin.rs"]
mod cli_bin;

use cli_bin::CliHome;

/// Body A for the near-duplicate pair. Measured in
/// `tests/common/cli_save_support.rs`: Hamming(A, B) = 5.
const DUP_BODY_A: &str = "postgres connection pool exhausts under load spikes raise \
     max_connections to fifty and add pgbouncer in transaction mode for the api workers \
     during peak traffic hours";
/// Body B: A with one word changed (`fifty` → `eighty`).
const DUP_BODY_B: &str = "postgres connection pool exhausts under load spikes raise \
     max_connections to eighty and add pgbouncer in transaction mode for the api workers \
     during peak traffic hours";

fn save_id(home: &CliHome, body: &str) -> String {
    home.run_json(&["save", body, "--kind", "note"])["id"]
        .as_str()
        .expect("id")
        .to_string()
}

fn make_prune_eligible(home: &CliHome, id: &str) {
    let db = home.data_dir().join("comemory.db");
    let conn = comemory::store::connection::open(db).expect("open mirror");
    conn.execute(
        "UPDATE memories SET quality = 2, last_accessed = '2025-01-01T00:00:00Z' WHERE id = ?1",
        [id],
    )
    .expect("doctor row");
}

#[test]
fn consolidate_prune_rebuild_gc() {
    let home = CliHome::new();
    let keeper = save_id(&home, DUP_BODY_A);
    let _dup = save_id(&home, DUP_BODY_B);
    let low = save_id(
        &home,
        "unrelated prune candidate about lighthouse inspection logs",
    );
    make_prune_eligible(&home, &low);

    let clusters = home.run_json(&["consolidate"]);
    assert!(
        clusters["clusters"]["total"].as_u64().expect("total") >= 1,
        "near-dup pair must cluster: {clusters}"
    );

    let dry = home.run_json(&["prune"]);
    let low_total = dry["low_value_memories"]["total"].as_u64().expect("total");
    assert!(
        low_total >= 1,
        "prune dry-run must flag the low-value row: {dry}"
    );

    home.run_ok(&["prune", "--apply"]);
    let listed = home.run_json(&["list"]);
    let ids: Vec<&str> = listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert!(
        !ids.contains(&low.as_str()),
        "applied prune must drop {low}"
    );
    assert!(ids.contains(&keeper.as_str()), "keeper must survive prune");

    home.run_ok(&["rebuild"]);
    // "fifty" is unique to DUP_BODY_A; "pgbouncer" is shared and the
    // near-dup collapse may keep only B in a mixed query.
    let search = home.run_json(&["search", "fifty"]);
    let hits = search["hits"].as_array().expect("hits");
    assert!(
        hits.iter()
            .any(|h| h["memory_id"].as_str() == Some(keeper.as_str())),
        "rebuild must still search the keeper: {search}"
    );

    let gc = home.run_json(&["gc"]);
    assert!(gc.get("removed").is_some(), "gc reports removed: {gc}");
    assert!(gc.get("log_rows").is_some(), "gc reports log_rows: {gc}");
    assert!(
        gc.get("event_rows").is_some(),
        "gc reports event_rows: {gc}"
    );
}
