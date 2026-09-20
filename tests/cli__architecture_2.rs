#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Refusals: every model a `comemory architecture save` must reject, and the
//! two commands that have nothing to read.

use std::path::Path;

use serde_json::{Value, json};
use tempfile::TempDir;

#[path = "common/architecture_repo.rs"]
mod architecture_repo;

use architecture_repo::{architecture_json, bin, index_repo};

/// Save `model` for repo `r` and return the failed assertion's stderr.
fn save_failure(home: &TempDir, dir: &Path, model: &Value, code: i32) -> String {
    let path = dir.join("model.json");
    std::fs::write(&path, serde_json::to_string_pretty(model).expect("json")).expect("write");
    let out = bin(home)
        .args(["architecture", "save"])
        .arg(&path)
        .args(["--repo", "r"])
        .assert()
        .failure()
        .code(code);
    String::from_utf8(out.get_output().stderr.clone()).expect("utf8")
}

/// The scaffolded model of the indexed fixture repo.
fn scaffold(home: &TempDir) -> Value {
    architecture_json(home, &["scaffold", "--repo", "r", "--json"])
}

/// Whether any architecture memory exists for repo `r`.
fn tagged_rows(home: &TempDir) -> usize {
    let out = bin(home)
        .args(["list", "--tag", "architecture", "--repo", "r", "--json"])
        .assert()
        .success();
    let listed: Value = serde_json::from_slice(&out.get_output().stdout).expect("json");
    listed["items"].as_array().expect("items").len()
}

#[test]
fn a_member_that_matches_no_indexed_file_is_refused_and_nothing_is_written() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");

    let mut model = scaffold(&home);
    model["components"][0]["members"] = json!(["src/does-not-exist"]);
    let stderr = save_failure(&home, ws.path(), &model, 64);

    assert!(stderr.contains("src/does-not-exist"), "{stderr}");
    assert!(stderr.contains("matches no indexed file"), "{stderr}");
    assert_eq!(tagged_rows(&home), 0);
}

#[test]
fn every_structural_violation_is_refused_by_name() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");
    let base = scaffold(&home);

    let mut duplicate = base.clone();
    let first = duplicate["components"][0].clone();
    duplicate["components"]
        .as_array_mut()
        .expect("components")
        .push(first);
    assert!(
        save_failure(&home, ws.path(), &duplicate, 64).contains("duplicate component id"),
        "duplicate id must be named"
    );

    let mut group = base.clone();
    group["components"][0]["group"] = json!("nope");
    assert!(
        save_failure(&home, ws.path(), &group, 64).contains("undeclared group nope"),
        "unknown group must be named"
    );

    let mut edge = base.clone();
    edge["edges"] = json!([{ "from": "src_a", "to": "ghost", "kind": "calls", "weight": 1 }]);
    assert!(
        save_failure(&home, ws.path(), &edge, 64).contains("\"ghost\" is not a declared component"),
        "dangling endpoint must be named"
    );

    let mut bad_id = base.clone();
    bad_id["components"][0]["id"] = json!("1bad");
    assert!(
        save_failure(&home, ws.path(), &bad_id, 64).contains("\"1bad\""),
        "malformed id must be named"
    );

    let mut wrong_repo = base.clone();
    wrong_repo["repo"] = json!("elsewhere");
    assert!(
        save_failure(&home, ws.path(), &wrong_repo, 64).contains("\"elsewhere\""),
        "foreign repo must be named"
    );

    assert_eq!(tagged_rows(&home), 0, "no refusal may write a memory");
}

#[test]
fn an_unknown_field_is_a_data_error_and_an_oversized_model_is_refused() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");
    let base = scaffold(&home);

    let mut unknown = base.clone();
    unknown["surprise"] = json!(true);
    let stderr = save_failure(&home, ws.path(), &unknown, 65);
    assert!(stderr.contains("surprise"), "{stderr}");

    let mut big = base.clone();
    let template = big["components"][0].clone();
    let mut n = 0;
    while serde_json::to_vec(&big).expect("json").len() <= 32 * 1024 {
        let mut c = template.clone();
        c["id"] = json!(format!("pad_{n}"));
        c["summary"] = json!("x".repeat(200));
        big["components"]
            .as_array_mut()
            .expect("components")
            .push(c);
        n += 1;
    }
    let stderr = save_failure(&home, ws.path(), &big, 64);
    assert!(stderr.contains("at most 32768 bytes allowed"), "{stderr}");
}

#[test]
fn show_and_check_name_the_repo_when_no_model_was_ever_saved() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");

    for command in ["show", "check"] {
        let out = bin(&home)
            .args(["architecture", command, "--repo", "r"])
            .assert()
            .failure()
            .code(64);
        let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf8");
        assert!(stderr.contains("no architecture model saved"), "{stderr}");
        assert!(stderr.contains("\"r\""), "{stderr}");
    }
}

#[test]
fn scaffolding_an_unindexed_repo_points_at_the_indexer() {
    let home = TempDir::new().expect("tempdir");
    let out = bin(&home)
        .args(["architecture", "scaffold", "--repo", "never-indexed"])
        .assert()
        .failure()
        .code(64);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf8");
    assert!(stderr.contains("no indexed files"), "{stderr}");
    assert!(stderr.contains("comemory index-code"), "{stderr}");
}
