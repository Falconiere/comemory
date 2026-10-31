#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Real folder acceptance for the explicit unified command.
use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn run(home: &Path, cwd: &Path, flags: &[&str]) -> Value {
    let output = Command::cargo_bin("comemory")
        .unwrap()
        .current_dir(cwd)
        .env("COMEMORY_SYNC_DAEMON", "0")
        .env_remove("COMEMORY_EMBED_CMD")
        .env_remove("COMEMORY_CODE_EMBED_CMD")
        .env_remove("COMEMORY_DOCUMENT_EMBED_CMD")
        .arg("--data-dir")
        .arg(home)
        .args(["--json", "run", "embedding"])
        .args(flags)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).unwrap()
}

#[test]
fn optional_flags_select_exact_folder_without_git() {
    let source = tempdir().unwrap();
    let nested = source.path().join("notes with spaces");
    fs::create_dir(&nested).unwrap();
    fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/docs/getting-started.md"),
        nested.join("guide.md"),
    )
    .unwrap();
    fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/utilities/digest.rs"),
        nested.join("digest.rs"),
    )
    .unwrap();
    fs::write(source.path().join("sibling.md"), "outside selected root").unwrap();
    let original = fs::read(nested.join("guide.md")).unwrap();
    for flags in [
        vec![],
        vec!["--path", "."],
        vec!["--repo", "chosen"],
        vec!["--path", ".", "--repo", "chosen"],
        vec!["--mode", "incremental"],
    ] {
        let home = tempdir().unwrap();
        let result = run(home.path(), &nested, &flags);
        assert_eq!(
            result["documents"]["sources"][0]["canonical_path"],
            nested.canonicalize().unwrap().to_str().unwrap()
        );
        assert_eq!(result["documents"]["sources"][0]["indexed"], 1);
        assert_eq!(result["code"]["files_indexed"], 1);
        assert_eq!(fs::read(nested.join("guide.md")).unwrap(), original);
        if flags.contains(&"chosen") {
            let output = Command::cargo_bin("comemory")
                .unwrap()
                .env("COMEMORY_SYNC_DAEMON", "0")
                .arg("--data-dir")
                .arg(home.path())
                .args(["--json", "search-code", "sha256_hex", "--repo", "chosen"])
                .assert()
                .success()
                .get_output()
                .stdout
                .clone();
            let hits: Value = serde_json::from_slice(&output).unwrap();
            assert!(
                !hits["hits"].as_array().unwrap().is_empty(),
                "public repo label must filter code: {hits}"
            );
        }
        let repeat = run(home.path(), &nested, &flags);
        assert_eq!(repeat["documents"]["sources"][0]["unchanged"], 1);
        assert_eq!(repeat["code"]["files_indexed"], 0);
    }
    let home = tempdir().unwrap();
    let relative = run(home.path(), source.path(), &["--path", "notes with spaces"]);
    assert_eq!(relative["documents"]["sources"][0]["indexed"], 1);
    let home = tempdir().unwrap();
    let absolute = run(
        home.path(),
        source.path(),
        &["--path", nested.to_str().unwrap()],
    );
    assert_eq!(absolute["documents"]["sources"][0]["indexed"], 1);
    assert!(!source.path().join(".git").exists());
}

#[test]
fn renamed_and_deleted_files_reconcile_only_the_selected_root() {
    let home = tempdir().unwrap();
    let first = tempdir().unwrap();
    let second = tempdir().unwrap();
    for root in [first.path(), second.path()] {
        fs::copy(
            concat!(env!("CARGO_MANIFEST_DIR"), "/docs/getting-started.md"),
            root.join("guide.md"),
        )
        .unwrap();
        fs::copy(
            concat!(env!("CARGO_MANIFEST_DIR"), "/src/utilities/digest.rs"),
            root.join("digest.rs"),
        )
        .unwrap();
        run(home.path(), root, &[]);
    }
    fs::rename(
        first.path().join("guide.md"),
        first.path().join("current.md"),
    )
    .unwrap();
    fs::remove_file(first.path().join("digest.rs")).unwrap();
    let result = run(home.path(), first.path(), &[]);
    assert_eq!(result["documents"]["sources"][0]["removed"], 2);
    assert_eq!(result["code"]["removed"], 1);
    assert_eq!(result["documents"]["sources"][0]["indexed"], 1);
    let other = run(home.path(), second.path(), &[]);
    assert_eq!(other["documents"]["sources"][0]["unchanged"], 1);
    assert_eq!(other["code"]["files_indexed"], 0);
}

#[test]
#[ignore = "requires COMEMORY_TEST_EMBED_CMD with a real 1024-dimensional model"]
fn real_provider_indexes_and_finds_document_semantically() {
    let cmd = std::env::var("COMEMORY_TEST_EMBED_CMD").expect("provider");
    let source = tempdir().unwrap();
    let data = tempdir().unwrap();
    let path = source.path().join("guide.md");
    fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/docs/guides/byo-vectors.md"),
        &path,
    )
    .unwrap();
    let output = Command::cargo_bin("comemory")
        .unwrap()
        .env("COMEMORY_SYNC_DAEMON", "0")
        .env("COMEMORY_DOCUMENT_EMBED_CMD", &cmd)
        .env_remove("COMEMORY_EMBED_CMD")
        .arg("--data-dir")
        .arg(data.path())
        .args(["--json", "run", "embedding", "--path"])
        .arg(source.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let indexed: Value = serde_json::from_slice(&output).unwrap();
    assert!(
        indexed["documents"]["sources"][0]["embedded"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(indexed["documents"]["sources"][0]["embedding_failed"], 0);
    let output = Command::cargo_bin("comemory")
        .unwrap()
        .env("COMEMORY_SYNC_DAEMON", "0")
        .env("COMEMORY_DOCUMENT_EMBED_CMD", &cmd)
        .env_remove("COMEMORY_EMBED_CMD")
        .arg("--data-dir")
        .arg(data.path())
        .args([
            "--json",
            "find",
            "--domain",
            "document",
            "How can I supply vectors for semantic retrieval?",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let result: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(result["hits"][0]["path"], "guide.md");
    assert!(result["hits"][0]["score_parts"]["ann"].as_f64().is_some());
}
