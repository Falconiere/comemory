#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration tests for `comemory show`, driven as a real subprocess (spec
//! AC-6, AC-7, AC-8, AC-10). The corpus is built through the real `comemory
//! save` / `index-code` / `search` commands — never hand-inserted rows —
//! mirroring `tests/cli__context.rs`'s git-repo harness. AC-9 (the HTTP
//! surface answering identically) lives in
//! `tests/serve__routes__memories__mod.rs`, which already owns the spawned
//! server.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

/// Run `comemory show <id> --json` and parse the response object.
fn show_json(home: &TempDir, id: &str) -> Value {
    let out = bin(home).args(["show", id, "--json"]).assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    serde_json::from_str(&stdout).expect("show --json parses")
}

/// Save a memory with the given extra flags, returning the parsed
/// `save --json` object.
fn save_json(home: &TempDir, body: &str, extra: &[&str]) -> Value {
    let mut args = vec!["save", body, "--json"];
    args.extend_from_slice(extra);
    let out = bin(home).args(&args).assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    serde_json::from_str(&stdout).expect("save --json parses")
}

/// AC-6: a memory saved with tags, `--quality 4`, and a backtick-fenced
/// `repo:path:symbol` body reference returns the body verbatim, quality 4,
/// the tag list, and exactly one `code_refs` entry (the implied file ref
/// for the same path collapses into the symbol ref — see
/// `api::show::code_refs_for`).
#[test]
fn show_returns_full_body_quality_tags_and_one_code_ref() {
    let home = TempDir::new().expect("tempdir");
    let body = "the ranker reads frontmatter, never the body\n\nsee `demo:src/lib.rs:foo_fn`";
    let saved = save_json(
        &home,
        body,
        &[
            "--kind",
            "decision",
            "--repo",
            "demo",
            "--tags",
            "ranking,frontmatter",
            "--quality",
            "4",
        ],
    );
    let id = saved["id"].as_str().expect("id").to_string();

    let v = show_json(&home, &id);
    assert_eq!(v["body"], body, "body must round-trip verbatim: {v}");
    assert_eq!(v["quality"], 4);
    let tags: Vec<&str> = v["tags"]
        .as_array()
        .expect("tags array")
        .iter()
        .map(|t| t.as_str().expect("tag string"))
        .collect();
    assert!(tags.contains(&"ranking"), "got {tags:?}");
    assert!(tags.contains(&"frontmatter"), "got {tags:?}");
    let refs = v["code_refs"].as_array().expect("code_refs array");
    assert_eq!(refs.len(), 1, "one code ref expected: {v}");
    assert_eq!(refs[0]["anchor"], "demo:src/lib.rs:foo_fn");
}

/// AC-7: `show` on an unknown id, and on a soft-deleted id, exits with the
/// `NotFound` sysexit code (64) and prints no partial object to stdout.
#[test]
fn show_unknown_and_soft_deleted_id_are_not_found_with_no_partial_output() {
    let home = TempDir::new().expect("tempdir");

    let out = bin(&home).args(["show", "deadbeef"]).assert().code(64);
    assert!(
        out.get_output().stdout.is_empty(),
        "no partial object on unknown-id failure"
    );

    let saved = save_json(&home, "a memory that will be soft-deleted", &[]);
    let id = saved["id"].as_str().expect("id").to_string();
    bin(&home).args(["delete", &id]).assert().success();

    let out = bin(&home).args(["show", &id]).assert().code(64);
    assert!(
        out.get_output().stdout.is_empty(),
        "no partial object on soft-deleted-id failure"
    );
}

/// Index a fixture repo at `<workspace>/code-repo` containing `alpha.rs`
/// with two top-level functions, committed and indexed under repo label
/// `r`. Returns the repo path so callers can mutate + recommit it.
fn index_alpha_repo(home: &TempDir, workspace: &TempDir) -> std::path::PathBuf {
    let repo = workspace.path().join("code-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[(
            "alpha.rs",
            "fn alpha_router() {}\nfn unrelated_helper() {}\n",
        )],
        "init",
    );
    bin(home)
        .args(["index-code", "--repo", "r", "--path"])
        .arg(&repo)
        .assert()
        .success();
    repo
}

fn reindex(home: &TempDir, repo: &Path) {
    bin(home)
        .args(["index-code", "--repo", "r", "--path"])
        .arg(repo)
        .assert()
        .success();
}

/// Save a memory with `--ref-symbol <ref>` run from inside `repo`, so the
/// anchor captures the file's HEAD blob. Returns the saved id.
fn save_with_symbol_ref(home: &TempDir, repo: &Path, body: &str, sym_ref: &str) -> String {
    let out = bin(home)
        .current_dir(repo)
        .args([
            "save",
            body,
            "--kind",
            "decision",
            "--repo",
            "r",
            "--ref-symbol",
            sym_ref,
            "--json",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    let v: Value = serde_json::from_str(&stdout).expect("save --json parses");
    v["id"].as_str().expect("id").to_string()
}

fn find_ref<'a>(v: &'a Value, anchor: &str) -> &'a Value {
    v["code_refs"]
        .as_array()
        .expect("code_refs array")
        .iter()
        .find(|r| r["anchor"].as_str() == Some(anchor))
        .unwrap_or_else(|| panic!("ref {anchor} missing from show output: {v}"))
}

/// AC-8: a pinned symbol ref walks `fresh` -> `stale` -> `ghost` as the
/// underlying repo changes and is reindexed.
#[test]
fn show_code_ref_status_walks_fresh_stale_ghost() {
    let home = TempDir::new().expect("tempdir");
    let workspace = TempDir::new().expect("workspace");
    let repo = index_alpha_repo(&home, &workspace);
    let id = save_with_symbol_ref(
        &home,
        &repo,
        "pin to alpha_router behavior",
        "alpha.rs:alpha_router",
    );

    // Unchanged pinned blob, current index -> fresh.
    let v = show_json(&home, &id);
    let r = find_ref(&v, "r:alpha.rs:alpha_router");
    assert_eq!(r["status"], "fresh", "unchanged pinned symbol: {v}");

    // Committed edit changes the blob; reindex so symbol_present is known.
    git_commit::commit_files(
        &repo,
        &[(
            "alpha.rs",
            "fn alpha_router() { let _ = 1; }\nfn unrelated_helper() {}\n",
        )],
        "edit alpha_router",
    );
    reindex(&home, &repo);
    let v = show_json(&home, &id);
    let r = find_ref(&v, "r:alpha.rs:alpha_router");
    assert_eq!(r["status"], "stale", "committed blob change: {v}");

    // Delete the symbol from the file and reindex -> the current index no
    // longer has a matching code_symbols row -> ghost.
    git_commit::commit_files(
        &repo,
        &[("alpha.rs", "fn unrelated_helper() {}\n")],
        "delete alpha_router",
    );
    reindex(&home, &repo);
    let v = show_json(&home, &id);
    let r = find_ref(&v, "r:alpha.rs:alpha_router");
    assert_eq!(r["status"], "ghost", "symbol removed + reindexed: {v}");
}

/// AC-10: after `comemory search` returns a memory, `show` reports
/// `access_count` incremented and `last_accessed` non-null. Activation is
/// asserted strictly increasing between the **second and third** access,
/// never the first — see `retrieval::score::activation`: `access_count`
/// 0 and 1 both map to `ln(max(n,1)) = ln(1) = 0`, so a same-day first
/// search cannot raise it (asserting across the first access would be
/// flaky).
#[test]
fn search_bumps_access_count_and_activation_rises_from_second_to_third_access() {
    let home = TempDir::new().expect("tempdir");
    let saved = save_json(
        &home,
        "the pgbouncer transaction-mode migration runner decision",
        &["--kind", "decision", "--repo", "demo"],
    );
    let id = saved["id"].as_str().expect("id").to_string();

    // First access: establishes access_count = 1, activation stays 0.
    bin(&home)
        .args(["search", "pgbouncer transaction-mode migration runner"])
        .assert()
        .success();

    // Second access: access_count = 2.
    bin(&home)
        .args(["search", "pgbouncer transaction-mode migration runner"])
        .assert()
        .success();
    let after_second = show_json(&home, &id);
    assert_eq!(after_second["access_count"], 2);
    assert!(after_second["last_accessed"].is_string());
    let activation_second = after_second["activation"].as_f64().expect("activation");

    // Third access: access_count = 3.
    bin(&home)
        .args(["search", "pgbouncer transaction-mode migration runner"])
        .assert()
        .success();
    let after_third = show_json(&home, &id);
    assert_eq!(after_third["access_count"], 3);
    let activation_third = after_third["activation"].as_f64().expect("activation");

    assert!(
        activation_third > activation_second,
        "activation must strictly increase from the second to the third access: \
         {activation_second} -> {activation_third}"
    );
}
