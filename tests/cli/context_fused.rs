use assert_cmd::Command;
use comemory::config::paths::Paths;
use serde_json::Value;

use super::common;

/// Regression for C4: `comemory context` must route memory retrieval through
/// RRF-fused dense+BM25 search, not the old vector-only path. A rare-token
/// memory should surface in the `memories` array of the bundle.
#[test]
fn context_surfaces_lexical_only_memory() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());

    let mut save = Command::cargo_bin("comemory").unwrap();
    save.env("COMEMORY_DATA_DIR", paths.data_dir())
        .arg("save")
        .arg("zzzyx_unique_token: a rare lexical marker for context")
        .arg("--kind")
        .arg("note")
        .arg("--repo")
        .arg("r");
    save.assert().success();

    let out = Command::cargo_bin("comemory")
        .unwrap()
        .env("COMEMORY_DATA_DIR", paths.data_dir())
        .arg("--json")
        .arg("context")
        .arg("zzzyx_unique_token")
        .arg("--limit")
        .arg("5")
        .output()
        .unwrap();
    assert!(out.status.success(), "context exited non-zero");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let memories = v["memories"].as_array().unwrap();
    assert!(
        !memories.is_empty(),
        "fused context returned no memory hits"
    );
    let first = memories[0]["snippet"].as_str().unwrap();
    assert!(first.contains("zzzyx_unique_token"));
}

/// Regression for C4: `--limit 0` must be rejected at parse time by the
/// context subcommand's clap value parser.
#[test]
fn context_rejects_limit_zero() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());

    let out = Command::cargo_bin("comemory")
        .unwrap()
        .env("COMEMORY_DATA_DIR", paths.data_dir())
        .arg("context")
        .arg("anything")
        .arg("--limit")
        .arg("0")
        .output()
        .unwrap();
    assert!(!out.status.success(), "expected limit=0 to be rejected");
}
