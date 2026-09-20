#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Real-binary MCP journey for the architecture-model tools.

use std::process::Command;

use assert_cmd::prelude::*;
use mcp_bin::McpHome;
use serde_json::{Value, json};
use tempfile::TempDir;

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;
#[path = "common/mcp_bin.rs"]
mod mcp_bin;

const REPO: &str = "architecture-fixture";

/// The four tools run their real domain cores over a real indexed git repo.
#[tokio::test]
async fn architecture_tools_scaffold_save_show_and_check() {
    let workspace = TempDir::new().expect("workspace");
    let repo = fixture_repo(&workspace);
    let home = McpHome::spawn(&repo, &["--repo", REPO]).await;
    index(&home, &repo);

    let mut model = home.data("architecture_scaffold", json!({})).await;
    assert_eq!(model["schema"], json!(1), "{model}");
    assert_eq!(model["repo"], json!(REPO), "{model}");
    assert!(
        model["components"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty()),
        "scaffold has no components: {model}"
    );
    model["source"] = json!("agent");
    model["components"][0]["name"] = json!("Architecture fixture");

    let saved = home
        .data("architecture_save", json!({ "model": model }))
        .await;
    assert!(saved["id"].as_str().is_some(), "{saved}");

    let shown = home.data("architecture_show", json!({})).await;
    assert_eq!(shown["repo"], json!(REPO), "{shown}");
    assert_eq!(shown["source"], json!("agent"), "{shown}");
    assert_eq!(
        shown["components"][0]["name"],
        json!("Architecture fixture"),
        "{shown}"
    );

    let rendered = home
        .data("architecture_show", json!({ "format": "mermaid" }))
        .await;
    assert!(
        rendered["mermaid"]
            .as_str()
            .is_some_and(|source| source.starts_with("flowchart")),
        "{rendered}"
    );

    let drift = home.data("architecture_check", json!({})).await;
    assert_eq!(drift["repo"], json!(REPO), "{drift}");
    assert_eq!(drift["drift_count"], json!(0), "{drift}");
}

/// Architecture tools require a repo, and a read-only save keeps its existing
/// refusal precedence before the missing-scope check.
#[tokio::test]
async fn architecture_tools_refuse_missing_scope_and_read_only_save() {
    let nowhere = TempDir::new().expect("cwd");
    let unscoped = McpHome::spawn(nowhere.path(), &[]).await;
    for (tool, args) in [
        ("architecture_scaffold", json!({})),
        ("architecture_save", json!({ "model": model() })),
        ("architecture_show", json!({})),
        ("architecture_check", json!({})),
    ] {
        let refusal = unscoped.error(tool, args).await;
        assert_eq!(refusal["code"], json!("repo_required"), "{tool}: {refusal}");
    }

    let read_only = McpHome::spawn(nowhere.path(), &["--read-only"]).await;
    let refusal = read_only
        .error("architecture_save", json!({ "model": model() }))
        .await;
    assert_eq!(refusal["code"], json!("read_only"), "{refusal}");
}

/// A small committed Rust repository with a real mined import edge.
fn fixture_repo(workspace: &TempDir) -> std::path::PathBuf {
    let repo = workspace.path().join(REPO);
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[
            ("src/a/one.rs", "use crate::b::two;\n\npub fn alpha() {}\n"),
            ("src/b/two.rs", "pub fn beta() {}\n"),
        ],
        "initial architecture fixture",
    );
    repo
}

/// Index `repo` into the same data directory the spawned MCP child serves.
fn index(home: &McpHome, repo: &std::path::Path) {
    Command::cargo_bin("comemory")
        .expect("binary")
        .env("COMEMORY_DATA_DIR", home.data_dir())
        .args(["index-code", "--repo", REPO, "--path"])
        .arg(repo)
        .assert()
        .success();
}

/// A syntactically valid model used only to prove pre-core refusals.
fn model() -> Value {
    json!({
        "schema": 1,
        "repo": REPO,
        "generated_at": "2026-09-20T00:00:00Z",
        "source": "manual",
        "direction": "LR",
        "groups": [],
        "components": [],
        "edges": []
    })
}
