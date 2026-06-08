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
fn doctor_reports_zero_memories_on_fresh_dir() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicates::str::contains("memories_count : 0"));
}

#[test]
fn doctor_count_grows_after_save() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["save", "first memory body", "--kind", "decision"])
        .assert()
        .success();
    bin(&home)
        .args(["save", "second memory body", "--kind", "bug"])
        .assert()
        .success();
    let doctor = bin(&home).arg("doctor").assert().success();
    let out = String::from_utf8(doctor.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        out.contains("memories_count : 2"),
        "doctor should report 2 memories, got: {out:?}"
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
fn feedback_records_used_and_irrelevant_ids() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["feedback", "q1", "--used", "aaa,bbb", "--irrelevant", "ccc"])
        .assert()
        .success()
        .stdout(predicates::str::contains("ok"));
}

#[test]
fn feedback_json_emits_counts() {
    // Under `--json`, `feedback` must emit a structured envelope reporting
    // how many used/irrelevant ids were recorded, instead of the bare `ok`
    // line. This is what callers piping into other tools rely on.
    let home = TempDir::new().expect("tempdir");
    let cmd = bin(&home)
        .args([
            "--json",
            "feedback",
            "q1",
            "--used",
            "aaa,bbb",
            "--irrelevant",
            "ccc",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(cmd.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(v["ok"].as_bool(), Some(true));
    assert_eq!(v["used"].as_u64(), Some(2));
    assert_eq!(v["irrelevant"].as_u64(), Some(1));
}

#[test]
fn supersedes_json_emits_ids() {
    // Under `--json`, `supersedes` must emit `{ok, new, old}` instead of the
    // bare `ok` line. The graph upsert is a no-op for unknown ids per
    // [`Graph::add_supersedes`] semantics — the JSON shape is what we lock in
    // here.
    let home = TempDir::new().expect("tempdir");
    let cmd = bin(&home)
        .args(["--json", "supersedes", "newid000000", "oldid000000"])
        .assert()
        .success();
    let stdout = String::from_utf8(cmd.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(v["ok"].as_bool(), Some(true));
    assert_eq!(v["new"].as_str(), Some("newid000000"));
    assert_eq!(v["old"].as_str(), Some("oldid000000"));
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
fn doctor_json_emits_object() {
    let home = TempDir::new().expect("tempdir");
    let doctor = bin(&home).args(["--json", "doctor"]).assert().success();
    let stdout = String::from_utf8(doctor.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert!(v["data_dir"].is_string());
    assert_eq!(v["memories_count"].as_u64(), Some(0));
}

#[test]
fn memory_for_unknown_qualified_returns_empty_json() {
    // `memory-for` filter logic exercised without any memory references on
    // disk: the result MUST be a JSON array (possibly empty) and never fail.
    // Once the save flow persists `references.symbols` (later task), this
    // test stays valid because the qualified key intentionally won't match
    // any frontmatter.
    let home = TempDir::new().expect("tempdir");
    let cmd = bin(&home)
        .args(["--json", "memory-for", "norepo:no/path:no_symbol"])
        .assert()
        .success();
    let stdout = String::from_utf8(cmd.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    let arr = v.as_array().expect("top-level JSON array");
    assert!(arr.is_empty(), "no memories should match: {arr:?}");
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
    let arr = v.as_array().expect("top-level JSON array");
    assert_eq!(arr.len(), 2, "expected 2 matches, got {arr:?}");
    let lines: Vec<u64> = arr
        .iter()
        .map(|r| r["line"].as_u64().expect("line"))
        .collect();
    assert!(
        lines.contains(&1) && lines.contains(&2),
        "lines were {lines:?}"
    );
}

#[test]
fn supersedes_then_walk_emits_json_chain() {
    // Save two memories via the CLI, manually upsert their `Memory` nodes into
    // the kuzu graph (the save command itself does not yet write the graph),
    // then exercise `comemory supersedes` + `comemory walk --edge supersedes --json`
    // and confirm the superseded id appears in the returned JSON array.
    use comemory::config::paths::Paths;
    use comemory::graph::Graph;
    use comemory::memory::MemoryStore;

    let home = TempDir::new().expect("tempdir");
    let save_a = bin(&home)
        .args([
            "save",
            "old decision body",
            "--kind",
            "decision",
            "--repo",
            "r",
        ])
        .assert()
        .success();
    let id_a = extract_saved_id(
        &String::from_utf8(save_a.get_output().stdout.clone()).expect("utf8 stdout"),
    );
    let save_b = bin(&home)
        .args([
            "save",
            "new decision body",
            "--kind",
            "decision",
            "--repo",
            "r",
        ])
        .assert()
        .success();
    let id_b = extract_saved_id(
        &String::from_utf8(save_b.get_output().stdout.clone()).expect("utf8 stdout"),
    );

    // Bootstrap graph nodes: the v1 save command writes only markdown, so we
    // seed the kuzu `Memory` nodes here. Once Task 18+ wires the upsert into
    // the save flow, the seeding block can be deleted without touching the
    // assertions below.
    let paths = Paths::new(home.path().join(".comemory"));
    paths.ensure_dirs().expect("ensure dirs");
    let store = MemoryStore::new(paths.clone());
    let mems = store.list().expect("list mems");
    let g = Graph::open(paths.graph_dir()).expect("graph open");
    for m in &mems {
        g.upsert_memory(m).expect("upsert memory");
    }
    drop(g);

    bin(&home)
        .args(["supersedes", &id_b, &id_a])
        .assert()
        .success()
        .stdout(predicates::str::contains("ok"));

    let walk = bin(&home)
        .args(["--json", "walk", "--from", &id_b, "--edge", "supersedes"])
        .assert()
        .success();
    let stdout = String::from_utf8(walk.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    let arr = v.as_array().expect("top-level JSON array");
    let ids: Vec<&str> = arr.iter().map(|x| x.as_str().expect("string id")).collect();
    assert!(
        ids.contains(&id_a.as_str()),
        "walk should include the superseded id {id_a}, got {ids:?}"
    );
}

#[test]
fn walk_unsupported_edge_fails() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["walk", "--from", "deadbeef0000", "--edge", "relates"])
        .assert()
        .failure();
}

#[test]
fn conflicts_for_unknown_id_emits_empty_json_array() {
    let home = TempDir::new().expect("tempdir");
    let cmd = bin(&home)
        .args(["--json", "conflicts", "deadbeef0000"])
        .assert()
        .success();
    let stdout = String::from_utf8(cmd.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    let arr = v.as_array().expect("top-level JSON array");
    assert!(arr.is_empty(), "no conflicts expected: {arr:?}");
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

#[path = "cli/graph_serve.rs"]
mod graph_serve;

#[path = "common/vectors.rs"]
mod vectors;

#[path = "cli/save.rs"]
mod save;

#[path = "cli/search.rs"]
mod search;

#[path = "cli/context.rs"]
mod context;

#[path = "cli/index_code.rs"]
mod index_code;

#[path = "cli/ingest_code.rs"]
mod ingest_code;
