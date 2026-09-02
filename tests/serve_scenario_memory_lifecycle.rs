#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Memory lifecycle journey over `/api/v1` — the HTTP twin of
//! `tests/cli_scenario_memory_lifecycle.rs`: save (one per kind) → list →
//! show → supersede → delete (confirm-gated) → trash. Doctor stays healthy
//! throughout.

#[path = "common/serve_bin.rs"]
mod serve_bin;

use serde_json::json;
use serve_bin::ServeHome;

const KINDS: [&str; 6] = [
    "decision",
    "bug",
    "convention",
    "discovery",
    "pattern",
    "note",
];

#[test]
fn save_list_show_supersede_delete_round_trip() {
    let srv = ServeHome::new();

    let doctor = srv.get("/doctor");
    assert_eq!(doctor["db_writable"].as_bool(), Some(true), "{doctor}");

    let mut ids = Vec::new();
    for kind in KINDS {
        let body = format!("{kind} body unique token {kind}lifecycle");
        let saved = srv.post(
            "/memories",
            &json!({ "body": body, "kind": kind, "repo": "alpha" }),
        );
        ids.push(saved["id"].as_str().expect("save id").to_string());
    }

    let listed = srv.get("/memories");
    assert!(
        listed["total"].as_u64().expect("total") >= 6,
        "all six kinds: {listed}"
    );
    let kinds_seen: Vec<&str> = listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|r| r["kind"].as_str())
        .collect();
    for kind in KINDS {
        assert!(
            kinds_seen.contains(&kind),
            "list missing kind {kind}: {kinds_seen:?}"
        );
    }

    let bugs = srv.get_q("/memories", &[("repo", "alpha"), ("kind", "bug")]);
    assert_eq!(bugs["total"].as_u64(), Some(1), "{bugs}");

    let first_id = &ids[0];
    let shown = srv.get(&format!("/memories/{first_id}"));
    assert!(
        shown["body"]
            .as_str()
            .expect("body")
            .contains("decisionlifecycle"),
        "{shown}"
    );

    // The replacement shares only the search token with the original, so
    // the near-duplicate collapse cannot fold one into the other and BOTH
    // must come back — the old one annotated, the new one not.
    let old_id = ids[5].clone();
    let replacement = srv.post(
        "/memories",
        &json!({
            "body": "rewritten guidance on notelifecycle handling after the incident review",
            "kind": "note",
            "repo": "alpha",
            "supersedes": [old_id],
        }),
    );
    let new_id = replacement["id"].as_str().expect("new id").to_string();

    let search = srv.get_q("/memories/search", &[("query", "notelifecycle")]);
    let hits = search["hits"].as_array().expect("hits");
    let hit_for = |id: &str| {
        hits.iter()
            .find(|h| h["memory_id"].as_str() == Some(id))
            .unwrap_or_else(|| panic!("search must return {id}: {search}"))
    };
    assert_eq!(
        hit_for(&old_id)["superseded_by"].as_str(),
        Some(new_id.as_str()),
        "old memory must name its superseder: {search}"
    );
    assert!(
        hit_for(&new_id).get("superseded_by").is_none(),
        "replacement must not be annotated: {search}"
    );

    let (status, denied) = srv.delete_raw(&format!("/memories/{first_id}"));
    assert_eq!(status, 400, "{denied}");
    assert_eq!(
        denied["error"]["code"].as_str(),
        Some("confirmation_required"),
        "{denied}"
    );

    srv.delete(&format!("/memories/{first_id}?confirm=true"));

    let after = srv.get("/memories");
    let remaining: Vec<&str> = after["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert!(
        !remaining.contains(&first_id.as_str()),
        "deleted id in list: {after}"
    );

    let trash = srv.get("/trash");
    assert!(
        trash["items"]
            .as_array()
            .expect("trash items")
            .iter()
            .any(|r| r["id"].as_str() == Some(first_id.as_str())),
        "trash must list the deleted memory: {trash}"
    );

    let trash_dir = srv.data_dir().join("memories").join(".trash");
    let trashed = std::fs::read_dir(&trash_dir)
        .expect("read .trash")
        .filter_map(Result::ok)
        .count();
    assert!(trashed >= 1, "delete must leave a file in {trash_dir:?}");

    let doctor = srv.get("/doctor");
    assert_eq!(doctor["db_writable"].as_bool(), Some(true), "{doctor}");
}
