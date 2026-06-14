//! Integration tests for `comemory` memory-only subcommands.
//!
//! Each test isolates state by pointing `COMEMORY_DATA_DIR` at a fresh
//! `tempfile::TempDir`. We cover the side-effecting flow (save -> list ->
//! delete) and the diagnostic command (doctor).

use assert_cmd::Command;
use tempfile::TempDir;

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

/// Extract the id from `saved <id>` in the save command's stdout.
fn extract_saved_id(stdout: &str) -> String {
    let line = stdout
        .lines()
        .find(|l| l.starts_with("saved "))
        .expect("save stdout has 'saved <id>' line");
    line.strip_prefix("saved ")
        .expect("strip 'saved ' prefix")
        .split_whitespace()
        .next()
        .expect("id token after 'saved '")
        .to_string()
}

#[test]
fn save_then_list_shows_id() {
    let home = TempDir::new().expect("tempdir");
    let save = bin(&home)
        .args(["save", "hello world from save_then_list", "--kind", "note"])
        .assert()
        .success();
    let saved_stdout = String::from_utf8(save.get_output().stdout.clone()).expect("utf8 stdout");
    let id = extract_saved_id(&saved_stdout);
    assert_eq!(id.len(), 8, "memory id should be 8 hex chars: {id:?}");

    let list = bin(&home).args(["list"]).assert().success();
    let out = String::from_utf8(list.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        out.contains(&id),
        "list output should mention saved id {id}: {out:?}"
    );
    // The list row format is `<id>  <kind>  <repo>  <slug>`; confirm the kind
    // we passed lands in the row for the saved id.
    let row = out
        .lines()
        .find(|l| l.contains(&id))
        .expect("row for saved id");
    assert!(
        row.contains("note"),
        "row should include kind 'note': {row:?}"
    );
}

#[test]
fn save_then_delete_removes_from_list() {
    let home = TempDir::new().expect("tempdir");
    let save = bin(&home)
        .args(["save", "delete-me memory body", "--kind", "note"])
        .assert()
        .success();
    let saved_stdout = String::from_utf8(save.get_output().stdout.clone()).expect("utf8 stdout");
    let id = extract_saved_id(&saved_stdout);

    bin(&home)
        .args(["delete", &id])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!("deleted {id}")));

    let list = bin(&home).args(["list"]).assert().success();
    let out = String::from_utf8(list.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        !out.contains(&id),
        "deleted id should not appear in list: {out:?}"
    );
}

#[test]
fn save_json_emits_id_and_path() {
    let home = TempDir::new().expect("tempdir");
    let save = bin(&home)
        .args([
            "--json",
            "save",
            "json mode save body",
            "--kind",
            "discovery",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(save.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    let id = v["id"].as_str().expect("id field is a string").to_string();
    let path = v["path"].as_str().expect("path field is a string");
    assert_eq!(id.len(), 8, "id should be 8 hex chars");
    assert!(
        path.ends_with(".md"),
        "path should point at a .md file: {path}"
    );
}

#[test]
fn list_json_emits_array() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["save", "first list-json body", "--kind", "note"])
        .assert()
        .success();
    bin(&home)
        .args([
            "save",
            "second list-json body",
            "--kind",
            "bug",
            "--repo",
            "alpha",
        ])
        .assert()
        .success();
    let list = bin(&home).args(["--json", "list"]).assert().success();
    let stdout = String::from_utf8(list.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    let arr = v.as_array().expect("top-level JSON array");
    assert_eq!(arr.len(), 2, "two rows expected, got {arr:?}");
    for row in arr {
        assert!(row["id"].is_string());
        assert!(row["kind"].is_string());
        assert!(row["repo"].is_string());
        assert!(row["slug"].is_string());
    }
}

#[test]
fn list_filters_by_repo_and_kind() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args([
            "save",
            "alpha decision body",
            "--kind",
            "decision",
            "--repo",
            "alpha",
        ])
        .assert()
        .success();
    bin(&home)
        .args(["save", "beta bug body", "--kind", "bug", "--repo", "beta"])
        .assert()
        .success();
    bin(&home)
        .args(["save", "alpha bug body", "--kind", "bug", "--repo", "alpha"])
        .assert()
        .success();

    let filtered = bin(&home)
        .args(["list", "--repo", "alpha", "--kind", "bug"])
        .assert()
        .success();
    let out = String::from_utf8(filtered.get_output().stdout.clone()).expect("utf8 stdout");
    let line_count = out.lines().filter(|l| !l.is_empty()).count();
    assert_eq!(line_count, 1, "exactly one alpha+bug row expected: {out:?}");
    assert!(out.contains("alpha"));
    assert!(out.contains("bug"));
}

#[test]
fn delete_missing_id_fails() {
    // Fresh data dir: `delete` must call `ensure_dirs` before opening the
    // store so the missing-id case surfaces "memory not found" instead of an
    // ENOENT on `memories/`.
    let home = TempDir::new().expect("tempdir");
    let assertion = bin(&home)
        .args(["delete", "deadbeef0000"])
        .assert()
        .failure();
    let stderr = String::from_utf8(assertion.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(
        stderr.contains("memory not found"),
        "stderr should mention 'memory not found', got: {stderr:?}"
    );
    assert!(
        !stderr.contains("No such file or directory"),
        "stderr should not surface raw ENOENT, got: {stderr:?}"
    );
}

#[test]
fn save_rejects_out_of_range_quality() {
    // clap's value_parser range guard rejects --quality 99 with exit 2 before
    // the save handler runs. The error message must reference the upper bound
    // so users know the accepted range is 1..=5.
    let home = TempDir::new().expect("tempdir");
    let assertion = bin(&home)
        .args(["save", "body", "--kind", "note", "--quality", "99"])
        .assert()
        .failure();
    let stderr = String::from_utf8(assertion.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(
        stderr.contains("5"),
        "stderr should mention upper bound 5, got: {stderr:?}"
    );
    let code = assertion.get_output().status.code().expect("exit code");
    assert_ne!(code, 0, "out-of-range quality should fail");
}

#[test]
fn save_rejects_unknown_kind() {
    // --kind is now a clap ValueEnum; unknown values like `banana` are
    // rejected at parse time with exit 2 instead of being silently coerced
    // to `Note` by the old `parse_kind` fallback.
    let home = TempDir::new().expect("tempdir");
    let assertion = bin(&home)
        .args(["save", "body", "--kind", "banana"])
        .assert()
        .failure();
    let code = assertion.get_output().status.code().expect("exit code");
    assert_ne!(code, 0, "unknown --kind should fail");
}

#[test]
#[ignore = "downloads ~130 MB nomic-text model on first run"]
fn search_json_emits_route_field() {
    // After wiring `retrieval::classify` into the CLI, `comemory search --json`
    // MUST surface the chosen route so callers (and the corrective-fallback
    // pipeline in later tasks) can observe which branch fired without
    // re-running the classifier. Ignored by default because it loads the
    // nomic-text embedder.
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args([
            "save",
            "postgres analytics decision body",
            "--kind",
            "decision",
        ])
        .assert()
        .success();

    let search = bin(&home)
        .args([
            "--json",
            "search",
            "what is the postgres analytics decision",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(search.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    let route = v["route"].as_str().expect("envelope has a 'route' string");
    assert!(
        matches!(route, "Hybrid" | "Symbol" | "FtsFirst"),
        "route must be one of the known variants, got {route:?}",
    );
    assert!(v.get("hits").is_some(), "envelope must include 'hits' key");
    assert!(v["hits"].is_array(), "hits must be a JSON array");
}

#[test]
fn ast_finds_rust_function_pattern() {
    // `ast` runs purely against a source file + ast-grep — no embedders,
    // no LanceDB. Cheap, hermetic, validates the CLI shape end-to-end.
    let home = TempDir::new().expect("tempdir");
    let src_dir = home.path().join("ast-fixture");
    std::fs::create_dir_all(&src_dir).expect("mkdir fixture");
    let file = src_dir.join("lib.rs");
    std::fs::write(&file, "fn run_migration() {}\nfn other() {}\n").expect("write fixture");

    let cmd = bin(&home)
        .args(["--json", "ast", "fn $NAME() {}", "--lang", "rs", "--file"])
        .arg(&file)
        .assert()
        .success();
    let stdout = String::from_utf8(cmd.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    // `ast --json` now emits the shared `Page` envelope, not a bare array:
    // `{ items, limit, offset, total, has_more }`.
    let arr = v["items"].as_array().expect("Page.items array");
    assert_eq!(arr.len(), 2, "expected 2 matches, got {arr:?}");
    assert_eq!(v["total"], serde_json::json!(2), "Page.total in {v}");
    assert_eq!(
        v["has_more"],
        serde_json::json!(false),
        "Page.has_more in {v}"
    );
    let lines: Vec<u64> = arr
        .iter()
        .map(|r| r["line"].as_u64().expect("line"))
        .collect();
    assert!(
        lines.contains(&1) && lines.contains(&2),
        "lines were {lines:?}"
    );
}

/// Runs the headline `context` flow end-to-end: `index-code` against a tiny
/// fixture repo, then `context <symbol> --json` and parse the bundle. The
/// first run downloads the jina-code (~300 MB) and nomic-text (~130 MB)
/// embedder models, so the test is `#[ignore]`d by default. Run explicitly
/// with `cargo test -- --ignored index_code_and_context_run`.
#[test]
#[ignore]
fn index_code_and_context_run() {
    let home = TempDir::new().expect("tempdir");
    let repo_dir = home.path().join("myrepo");
    std::fs::create_dir_all(repo_dir.join("src")).expect("mkdir src");
    std::fs::write(repo_dir.join("src/lib.rs"), "fn run_migration() {}\n").expect("write lib.rs");

    bin(&home)
        .args(["index-code", "--repo", "myrepo", "--path"])
        .arg(&repo_dir)
        .assert()
        .success();

    let ctx = bin(&home)
        .args(["--json", "context", "run_migration"])
        .assert()
        .success();
    let stdout = String::from_utf8(ctx.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(v["key"].as_str(), Some("run_migration"));
    assert!(v["memories"].is_array());
    // Symbol may be null if the empty memory index path triggered an early
    // return, but the bundle envelope must be present.
    assert!(
        v.get("symbol").is_some(),
        "bundle should have a 'symbol' field"
    );
}
