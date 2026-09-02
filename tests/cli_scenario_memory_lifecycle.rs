#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Memory lifecycle journey: save (arg + stdin) → list → show → supersede
//! → delete → trash. Doctor stays healthy throughout.

#[path = "common/cli_bin.rs"]
mod cli_bin;

use cli_bin::CliHome;

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
    let home = CliHome::new();
    home.run_ok(&["doctor"]);

    let mut ids = Vec::new();
    let mut note_id = None;
    for kind in KINDS {
        let body = format!("{kind} body unique token {kind}lifecycle");
        let saved = home.run_json(&["save", &body, "--kind", kind, "--repo", "alpha"]);
        let id = saved["id"].as_str().expect("id").to_string();
        if kind == "note" {
            note_id = Some(id.clone());
        }
        ids.push(id);
    }

    let piped = home
        .bin()
        .args(["--json", "save", "-", "--kind", "note", "--repo", "alpha"])
        .write_stdin("piped stdin body unique token stdinlifecycle")
        .assert()
        .success();
    let piped_json: serde_json::Value =
        serde_json::from_slice(&piped.get_output().stdout).expect("save - json");
    assert_eq!(piped_json["id"].as_str().expect("piped id").len(), 8);

    let listed = home.run_json(&["list"]);
    assert!(
        listed["total"].as_u64().expect("total") >= 7,
        "all six kinds plus stdin: {listed}"
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

    let bugs = home.run_json(&["list", "--repo", "alpha", "--kind", "bug"]);
    assert_eq!(bugs["total"].as_u64(), Some(1), "{bugs}");

    let first_id = &ids[0];
    let shown = home.run_json(&["show", first_id]);
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
    let old_id = note_id.expect("the note-kind save");
    let replacement = home.run_json(&[
        "save",
        "rewritten guidance on notelifecycle handling after the incident review",
        "--kind",
        "note",
        "--repo",
        "alpha",
        "--supersedes",
        &old_id,
    ]);
    let new_id = replacement["id"].as_str().expect("new id").to_string();

    let search = home.run_json(&["search", "notelifecycle"]);
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

    // Count the trash before the delete so the assertion below proves THIS
    // delete moved a file, not some earlier step.
    let trash = home.data_dir().join("memories").join(".trash");
    let trash_count = || std::fs::read_dir(&trash).map_or(0, |d| d.filter_map(Result::ok).count());
    let before = trash_count();

    home.run_ok(&["delete", first_id]);
    let after = home.run_json(&["list"]);
    let remaining: Vec<&str> = after["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert!(
        !remaining.contains(&first_id.as_str()),
        "deleted id in list"
    );

    assert_eq!(
        trash_count(),
        before + 1,
        "delete must add exactly one file to {trash:?}"
    );

    let doctor = home.run_json(&["doctor"]);
    assert_eq!(doctor["db_writable"].as_bool(), Some(true), "{doctor}");
}
