//! `comemory list` integration tests against the real binary: the `--json`
//! output is now the shared `Page` envelope (not a bare array), `--limit` /
//! `--offset` page correctly with exact `total` / `has_more`, `--repo` /
//! `--kind` still filter, and the per-item row keeps its `id`/`kind`/`repo`/
//! `slug` shape. Memories are created through `comemory save` (real markdown +
//! mirror writes) so the listing reads the same SQLite mirror the CLI writes.

use assert_cmd::Command;

/// Save a memory through the real binary so both the markdown file and the
/// SQLite mirror row exist for `list` to read.
fn save(home: &tempfile::TempDir, body: &str, kind: &str, repo: &str) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["save", body, "--kind", kind, "--repo", repo])
        .assert()
        .success();
}

/// Run `comemory --json list [extra...]` and parse the `Page` envelope.
fn list_json(home: &tempfile::TempDir, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["--json", "list"];
    args.extend_from_slice(extra);
    let out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(&args)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("list --json not valid JSON: {e}\n{stdout}"))
}

/// Six memories across two repos and two kinds.
fn seeded_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("tempdir");
    save(&home, "alpha decision one", "decision", "alpha");
    save(&home, "alpha bug two", "bug", "alpha");
    save(&home, "beta decision three", "decision", "beta");
    save(&home, "beta bug four", "bug", "beta");
    save(&home, "alpha decision five", "decision", "alpha");
    save(&home, "beta bug six", "bug", "beta");
    home
}

fn item_count(v: &serde_json::Value) -> usize {
    v["items"].as_array().expect("items array").len()
}

#[test]
fn list_json_is_page_envelope_not_bare_array() {
    let home = seeded_home();
    let v = list_json(&home, &[]);
    assert!(v.is_object(), "list --json must be a Page object, got: {v}");
    assert_eq!(v["total"], serde_json::json!(6));
    assert_eq!(item_count(&v), 6);
    // Default limit (50) > 6, so no further pages.
    assert_eq!(v["has_more"], serde_json::json!(false));
    // Each item keeps the legacy row shape.
    let first = &v["items"][0];
    for field in ["id", "kind", "repo", "slug"] {
        assert!(
            first.get(field).is_some(),
            "row must carry `{field}`: {first}"
        );
    }
}

#[test]
fn list_json_limit_and_offset_page_correctly() {
    let home = seeded_home();
    let v = list_json(&home, &["--limit", "2", "--offset", "0"]);
    assert_eq!(item_count(&v), 2);
    assert_eq!(v["limit"], serde_json::json!(2));
    assert_eq!(v["offset"], serde_json::json!(0));
    assert_eq!(v["total"], serde_json::json!(6));
    // 0 + 2 shown < 6 total -> more pages remain.
    assert_eq!(v["has_more"], serde_json::json!(true));

    // Walk every window and confirm the ids partition the full set with no
    // overlap or gap (stable order across paged calls).
    let mut seen: Vec<String> = Vec::new();
    for off in [0usize, 2, 4] {
        let page = list_json(&home, &["--limit", "2", "--offset", &off.to_string()]);
        for item in page["items"].as_array().expect("items") {
            seen.push(item["id"].as_str().expect("id").to_string());
        }
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 6, "paged windows must cover all 6 ids once");
}

#[test]
fn list_json_last_page_has_no_more() {
    let home = seeded_home();
    let v = list_json(&home, &["--limit", "2", "--offset", "4"]);
    assert_eq!(item_count(&v), 2);
    assert_eq!(v["total"], serde_json::json!(6));
    // 4 + 2 == 6 -> nothing beyond this window.
    assert_eq!(v["has_more"], serde_json::json!(false));
}

#[test]
fn list_json_repo_filter() {
    let home = seeded_home();
    let v = list_json(&home, &["--repo", "beta"]);
    assert_eq!(v["total"], serde_json::json!(3));
    assert_eq!(item_count(&v), 3);
    for item in v["items"].as_array().expect("items") {
        assert_eq!(item["repo"], serde_json::json!("beta"));
    }
}

#[test]
fn list_json_kind_filter_is_case_insensitive() {
    let home = seeded_home();
    // Mixed-case `--kind` must match the canonical lowercase value, mirroring
    // the legacy `eq_ignore_ascii_case` filter.
    let v = list_json(&home, &["--kind", "Decision"]);
    assert_eq!(v["total"], serde_json::json!(3));
    assert_eq!(item_count(&v), 3);
    for item in v["items"].as_array().expect("items") {
        assert_eq!(item["kind"], serde_json::json!("decision"));
    }
}

#[test]
fn list_json_repo_and_kind_filters_combine() {
    let home = seeded_home();
    let v = list_json(&home, &["--repo", "alpha", "--kind", "decision"]);
    assert_eq!(v["total"], serde_json::json!(2));
    assert_eq!(item_count(&v), 2);
    for item in v["items"].as_array().expect("items") {
        assert_eq!(item["repo"], serde_json::json!("alpha"));
        assert_eq!(item["kind"], serde_json::json!("decision"));
    }
}

#[test]
fn list_json_limit_zero_returns_all() {
    let home = seeded_home();
    let v = list_json(&home, &["--limit", "0"]);
    assert_eq!(item_count(&v), 6);
    assert_eq!(v["limit"], serde_json::json!(0));
    assert_eq!(v["has_more"], serde_json::json!(false));
}

#[test]
fn list_tty_prints_pagination_footer() {
    let home = seeded_home();
    let out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["list", "--limit", "2"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        stdout.contains("showing 1\u{2013}2 of 6 (--offset 0)"),
        "TTY footer missing; got: {stdout:?}"
    );
}
