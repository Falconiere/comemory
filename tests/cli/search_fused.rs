use assert_cmd::Command;
use comemory::config::paths::Paths;
use serde_json::Value;

use super::common;

#[test]
fn search_surfaces_lexical_only_match() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());

    let mut save = Command::cargo_bin("comemory").unwrap();
    save.env("COMEMORY_DATA_DIR", paths.data_dir())
        .arg("save")
        .arg("zzzyx_unique_token: a rare lexical marker")
        .arg("--kind")
        .arg("note")
        .arg("--repo")
        .arg("r");
    save.assert().success();

    let out = Command::cargo_bin("comemory")
        .unwrap()
        .env("COMEMORY_DATA_DIR", paths.data_dir())
        .arg("--json")
        .arg("search")
        .arg("zzzyx_unique_token")
        .arg("--limit")
        .arg("5")
        .output()
        .unwrap();
    assert!(out.status.success(), "search exited non-zero");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let hits = v["hits"].as_array().unwrap();
    assert!(!hits.is_empty(), "fused search returned no hits");
    let first = hits[0]["snippet"].as_str().unwrap();
    assert!(first.contains("zzzyx_unique_token"));
}
