#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Automatic embedding failure and explicit-vector precedence at the CLI boundary.

use assert_cmd::Command;
use tempfile::tempdir;

#[test]
fn failed_provider_keeps_save_and_explicit_vector_bypasses_it() {
    let home = tempdir().expect("data dir");
    let marker = home.path().join("called");
    let command = format!("touch '{}'; exit 1", marker.display());
    let saved = Command::cargo_bin("comemory")
        .expect("binary")
        .env("COMEMORY_DATA_DIR", home.path())
        .env("COMEMORY_EMBED_CMD", &command)
        .args([
            "--json",
            "save",
            "Embedding failure preserves this real memory body",
        ])
        .assert()
        .success();
    let result: serde_json::Value =
        serde_json::from_slice(&saved.get_output().stdout).expect("json");
    assert!(result["warnings"].to_string().contains("embedding failed"));
    assert!(std::path::Path::new(result["path"].as_str().expect("path")).exists());
    assert!(marker.exists());
    std::fs::remove_file(&marker).expect("remove marker");
    let vector = vec!["0.1"; 1024].join(",");
    Command::cargo_bin("comemory")
        .expect("binary")
        .env("COMEMORY_DATA_DIR", home.path())
        .env("COMEMORY_EMBED_CMD", &command)
        .args([
            "--json",
            "save",
            "A distinct explicit vector memory",
            "--vector",
            &vector,
        ])
        .assert()
        .success();
    assert!(!marker.exists(), "explicit vector must bypass the provider");
    Command::cargo_bin("comemory")
        .expect("binary")
        .env("COMEMORY_DATA_DIR", home.path())
        .env("COMEMORY_EMBED_CMD", &command)
        .args([
            "save",
            "Invalid explicit vectors fail without inference",
            "--vector",
            "0.1",
        ])
        .assert()
        .failure();
    assert!(
        !marker.exists(),
        "invalid explicit vector must also bypass provider"
    );
    Command::cargo_bin("comemory")
        .expect("binary")
        .env("COMEMORY_DATA_DIR", home.path())
        .env("COMEMORY_EMBED_CMD", &command)
        .args(["search", "some query"])
        .assert()
        .failure();
}

#[test]
#[ignore = "requires COMEMORY_TEST_EMBED_CMD with a real 1024-dimensional model"]
fn real_provider_embeds_a_guide_and_retrieves_it_semantically() {
    let command = std::env::var("COMEMORY_TEST_EMBED_CMD").expect("set COMEMORY_TEST_EMBED_CMD");
    let home = tempdir().expect("data dir");
    let body = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/docs/guides/byo-vectors.md"
    ))
    .expect("guide");
    Command::cargo_bin("comemory")
        .expect("binary")
        .env("COMEMORY_DATA_DIR", home.path())
        .env("COMEMORY_EMBED_CMD", &command)
        .args(["--json", "save", &body])
        .assert()
        .success();
    let output = Command::cargo_bin("comemory")
        .expect("binary")
        .env("COMEMORY_DATA_DIR", home.path())
        .env("COMEMORY_EMBED_CMD", &command)
        .args([
            "--json",
            "search",
            "How can I add semantic vectors from an external model?",
        ])
        .assert()
        .success();
    let result: serde_json::Value =
        serde_json::from_slice(&output.get_output().stdout).expect("json");
    assert!(
        result["hits"][0]["score_parts"]["legs"]["ann"]
            .as_f64()
            .is_some(),
        "expected ANN contribution: {result}"
    );
}

#[test]
fn malformed_or_wrong_width_provider_preserves_lexical_save() {
    for command in ["printf 'not-json'", "printf '{\"embedding\":[1]}'"] {
        let home = tempdir().expect("data dir");
        let output = Command::cargo_bin("comemory")
            .expect("binary")
            .env("COMEMORY_DATA_DIR", home.path())
            .env("COMEMORY_EMBED_CMD", command)
            .args(["--json", "save", include_str!("../docs/getting-started.md")])
            .assert()
            .success();
        let result: serde_json::Value =
            serde_json::from_slice(&output.get_output().stdout).expect("JSON");
        assert!(
            result["warnings"]
                .as_array()
                .is_some_and(|warnings| !warnings.is_empty())
        );
        assert!(std::path::Path::new(result["path"].as_str().expect("path")).exists());
    }
}
