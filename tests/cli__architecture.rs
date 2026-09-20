#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory architecture scaffold` / `save` / `show` against a really
//! indexed two-directory repository.

use tempfile::TempDir;

#[path = "common/architecture_repo.rs"]
mod architecture_repo;

use architecture_repo::{architecture_json, bin, index_repo, scaffold_and_save, stdout_of};

#[test]
fn scaffold_clusters_indexed_directories_and_projects_mined_edges() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");

    let v = architecture_json(&home, &["scaffold", "--repo", "r", "--json"]);
    assert_eq!(v["schema"], 1);
    assert_eq!(v["repo"], "r");
    assert_eq!(v["source"], "scaffold");

    let ids: Vec<&str> = v["components"]
        .as_array()
        .expect("components")
        .iter()
        .map(|c| c["id"].as_str().expect("id"))
        .collect();
    assert!(ids.contains(&"src_a"), "{ids:?}");
    assert!(ids.contains(&"src_b"), "{ids:?}");

    let members: Vec<&str> = v["components"]
        .as_array()
        .expect("components")
        .iter()
        .flat_map(|c| c["members"].as_array().expect("members"))
        .map(|m| m.as_str().expect("member"))
        .collect();
    assert!(members.contains(&"src/a"), "{members:?}");

    let edges = v["edges"].as_array().expect("edges");
    assert!(
        edges
            .iter()
            .any(|e| e["from"] == "src_a" && e["to"] == "src_b" && e["kind"] == "imports"),
        "{edges:?}"
    );
}

#[test]
fn scaffold_is_byte_stable_across_runs() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");

    let first = architecture_json(&home, &["scaffold", "--repo", "r", "--json"]);
    let second = architecture_json(&home, &["scaffold", "--repo", "r", "--json"]);
    // `generated_at` is a wall-clock stamp by design; everything else must match.
    for key in [
        "schema",
        "repo",
        "source",
        "direction",
        "components",
        "edges",
    ] {
        assert_eq!(first[key], second[key], "{key} drifted");
    }
}

#[test]
fn depth_one_collapses_the_directories_into_one_component() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");

    let v = architecture_json(
        &home,
        &["scaffold", "--repo", "r", "--depth", "1", "--json"],
    );
    let components = v["components"].as_array().expect("components");
    assert_eq!(components.len(), 1, "{components:?}");
    assert_eq!(components[0]["id"], "src");
    assert_eq!(components[0]["files"], 2);
    // Both files live in the one component, so no edge survives.
    assert!(v["edges"].as_array().expect("edges").is_empty(), "{v}");
}

#[test]
fn a_saved_model_becomes_the_repos_single_tagged_memory() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");
    scaffold_and_save(&home, "r", ws.path());

    let out = bin(&home)
        .args(["list", "--tag", "architecture", "--repo", "r", "--json"])
        .assert()
        .success();
    let listed: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).expect("json");
    let items = listed["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{listed}");
    assert_eq!(items[0]["kind"], "note");
    assert_eq!(items[0]["repo"], "r");
}

#[test]
fn a_second_save_supersedes_the_first() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");
    let mut model = scaffold_and_save(&home, "r", ws.path());

    let first = architecture_json(&home, &["show", "--repo", "r", "--json"]);
    assert_eq!(first["components"], model["components"]);

    model["components"][0]["name"] = serde_json::json!("Alpha");
    model["source"] = serde_json::json!("agent");
    let path = ws.path().join("enriched.json");
    std::fs::write(&path, serde_json::to_string_pretty(&model).expect("json")).expect("write");
    let out = bin(&home)
        .args(["architecture", "save"])
        .arg(&path)
        .args(["--repo", "r", "--json"])
        .assert()
        .success();
    let saved: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).expect("json");
    let superseded = saved["superseded"].as_str().expect("superseded id");

    // A superseded memory stays live and annotated — comemory never deletes
    // history on a supersede — so the tag now lists both, newest first.
    let out = bin(&home)
        .args(["list", "--tag", "architecture", "--repo", "r", "--json"])
        .assert()
        .success();
    let listed: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).expect("json");
    let items = listed["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "{listed}");
    assert_eq!(items[0]["id"], saved["id"], "newest first");
    assert_eq!(items[1]["id"], superseded);

    let out = bin(&home)
        .args(["show", superseded, "--json"])
        .assert()
        .success();
    let old: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).expect("json");
    assert_eq!(old["superseded_by"], saved["id"], "{old}");

    let shown = architecture_json(&home, &["show", "--repo", "r", "--json"]);
    assert_eq!(shown["components"][0]["name"], "Alpha");
    assert_eq!(shown["source"], "agent");
}

#[test]
fn show_renders_mermaid_and_the_global_json_flag_overrides_it() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");
    scaffold_and_save(&home, "r", ws.path());

    let mermaid = stdout_of(&home, &["show", "--repo", "r", "--format", "mermaid"]);
    assert!(mermaid.starts_with("flowchart LR\n"), "{mermaid}");
    assert!(mermaid.contains("src_a[\""), "{mermaid}");
    assert!(mermaid.contains("-->|imports| src_b"), "{mermaid}");
    assert_eq!(
        mermaid,
        stdout_of(&home, &["show", "--repo", "r", "--format", "mermaid"])
    );

    let json = stdout_of(
        &home,
        &["show", "--repo", "r", "--format", "mermaid", "--json"],
    );
    let parsed: serde_json::Value = serde_json::from_str(json.trim()).expect("json wins");
    assert_eq!(parsed["schema"], 1);
}
