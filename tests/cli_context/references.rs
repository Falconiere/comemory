//! Context code-reference status and graph-neighbor integration tests.

use comemory::store::connection;
use serde_json::Value;
use tempfile::TempDir;

use super::{
    bin, context_json, extract_saved_id, git_commit, git_repo, index_alpha_repo, save_memory,
};

/// Save a memory with `--ref-symbol <ref>` run from inside `repo` so the anchor
/// captures the file's HEAD blob. Returns the saved id.
fn save_with_symbol_ref(
    home: &TempDir,
    repo: &std::path::Path,
    body: &str,
    sym_ref: &str,
) -> String {
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
        ])
        .assert()
        .success();
    extract_saved_id(&String::from_utf8(out.get_output().stdout.clone()).expect("utf8"))
}

/// Find the code ref with the given `id` in a context bundle.
fn find_ref<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["code_refs"]
        .as_array()
        .expect("code_refs array")
        .iter()
        .find(|r| r["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("ref {id} missing from bundle: {v}"))
}

/// A pinned symbol ref whose file's HEAD blob is unchanged since save, with the
/// index current, reports `status: "fresh"` and surfaces line + signature.
#[test]
fn context_symbol_ref_status_fresh() {
    let home = TempDir::new().expect("tempdir");
    let workspace = TempDir::new().expect("workspace");
    let repo = index_alpha_repo(&home, &workspace);
    save_with_symbol_ref(
        &home,
        &repo,
        "pin to alpha_router behavior",
        "alpha.rs:alpha_router",
    );

    let v = context_json(&home, "pin alpha_router behavior", &[]);
    let r = find_ref(&v, "r:alpha.rs:alpha_router");
    assert_eq!(
        r["status"], "fresh",
        "unchanged pinned symbol must be fresh: {v}"
    );
    assert_eq!(r["line"], 1, "fresh symbol carries its line: {v}");
    assert_eq!(
        r["signature"], "fn alpha_router() {}",
        "fresh symbol carries signature: {v}"
    );
}

/// Editing and committing the referenced file changes its blob; after a
/// re-index (so the index is current) the pinned symbol ref reports `stale`.
#[test]
fn context_symbol_ref_status_stale_after_committed_edit() {
    let home = TempDir::new().expect("tempdir");
    let workspace = TempDir::new().expect("workspace");
    let repo = index_alpha_repo(&home, &workspace);
    save_with_symbol_ref(
        &home,
        &repo,
        "pin to alpha_router for stale check",
        "alpha.rs:alpha_router",
    );

    // Change the file's committed blob, then re-index so symbol_present is known.
    git_commit::commit_files(
        &repo,
        &[(
            "alpha.rs",
            "fn alpha_router() { let _ = 1; }\nfn unrelated_helper() {}\n",
        )],
        "edit alpha_router",
    );
    bin(&home)
        .args(["index-code", "--repo", "r", "--path"])
        .arg(&repo)
        .assert()
        .success();

    let v = context_json(&home, "pin alpha_router stale check", &[]);
    let r = find_ref(&v, "r:alpha.rs:alpha_router");
    assert_eq!(
        r["status"], "stale",
        "committed blob change must be stale: {v}"
    );
}

/// A `--ref-file` reference surfaces in the bundle with file-level fields null
/// and a status decided purely by the HEAD-tree blob (index-independent).
#[test]
fn context_file_ref_surfaces_with_status() {
    let home = TempDir::new().expect("tempdir");
    let workspace = TempDir::new().expect("workspace");
    let repo = index_alpha_repo(&home, &workspace);
    bin(&home)
        .current_dir(&repo)
        .args([
            "save",
            "pin the whole alpha file",
            "--kind",
            "decision",
            "--repo",
            "r",
            "--ref-file",
            "alpha.rs",
        ])
        .assert()
        .success();

    let v = context_json(&home, "pin whole alpha file", &[]);
    let r = find_ref(&v, "r:alpha.rs");
    assert_eq!(
        r["status"], "fresh",
        "unchanged pinned file must be fresh: {v}"
    );
    assert_eq!(r["symbol"], "", "file ref has no symbol: {v}");
    assert!(r["line"].is_null(), "file ref has no line: {v}");
    assert!(r["signature"].is_null(), "file ref has no signature: {v}");
}

/// A ref whose repo has no `repo_marker.root_path` on disk cannot be verified:
/// `resolve_root` fails, so `repo_on_disk` is false and the status is `unknown`.
#[test]
fn context_symbol_ref_status_unknown_when_repo_not_on_disk() {
    let home = TempDir::new().expect("tempdir");
    let workspace = TempDir::new().expect("workspace");
    let repo = index_alpha_repo(&home, &workspace);
    let id = save_with_symbol_ref(
        &home,
        &repo,
        "pin alpha for unknown check",
        "alpha.rs:alpha_router",
    );

    // Point the code_ref's repo at an unindexed label (no repo_marker row), so
    // resolve_root errors -> repo_on_disk=false -> Unknown. The anchor stays
    // pinned (a non-null blob), which is what separates Unknown from Unpinned.
    let db = home.path().join(".comemory").join("comemory.db");
    let conn = connection::open(&db).expect("open");
    conn.execute(
        "UPDATE code_ref SET dst_id = 'ghostrepo:alpha.rs:alpha_router' WHERE memory_id = ?1",
        [&id],
    )
    .expect("repoint code_ref");
    conn.execute(
        "UPDATE edges SET dst_id = 'ghostrepo:alpha.rs:alpha_router' \
         WHERE src_id = ?1 AND rel = 'references_symbol'",
        [&id],
    )
    .expect("repoint edge");

    let v = context_json(&home, "pin alpha unknown check", &[]);
    let r = find_ref(&v, "ghostrepo:alpha.rs:alpha_router");
    assert_eq!(
        r["status"], "unknown",
        "pinned ref in an off-disk repo must be unknown: {v}"
    );
}

/// Index a fixture repo at `<workspace>/neighbor-repo` containing `a.rs`
/// (`mod b;\nfn alpha() {}\n`, a real Rust import of `b.rs`) and `b.rs`
/// (`fn beta() {}\n`), committed together in ONE commit so the real
/// co-change miner records a real `co_changed` edge alongside the real
/// `imports` edge `index-code` resolves from `mod b;`. Indexed under repo
/// label `n`.
fn index_neighbor_repo(home: &TempDir, workspace: &TempDir) -> std::path::PathBuf {
    let repo = workspace.path().join("neighbor-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[
            ("a.rs", "mod b;\nfn alpha() {}\n"),
            ("b.rs", "fn beta() {}\n"),
        ],
        "init",
    );
    bin(home)
        .args(["index-code", "--repo", "n", "--path"])
        .arg(&repo)
        .assert()
        .success();
    repo
}

/// AC-30: `comemory context <query> --json` returns a `neighbors` array
/// carrying the file's REAL `imports` and `co_changed` counterparts — mined
/// and materialized by the REAL `comemory index-code` over a real git repo
/// — while the pre-existing `memories`, `code_refs`, and `relations` fields
/// keep working exactly as before (the additive guarantee).
#[test]
fn context_neighbors_include_real_imports_and_co_changed_edges() {
    let home = TempDir::new().expect("tempdir");
    let workspace = TempDir::new().expect("workspace");
    index_neighbor_repo(&home, &workspace);

    save_memory(
        &home,
        "neighbor decision cites n:a.rs:alpha for dispatch",
        "decision",
    );

    let v = context_json(&home, "neighbor decision dispatch", &[]);

    // Pre-existing fields: unchanged shape and values.
    let mems = v["memories"].as_array().expect("memories array");
    assert_eq!(mems.len(), 1, "one memory matched: {v}");
    let code_refs = v["code_refs"].as_array().expect("code_refs array");
    assert!(
        code_refs.iter().any(|r| r["id"] == "n:a.rs:alpha"),
        "the cited symbol must still resolve as a code ref: {v}"
    );
    // The pre-existing `relations` field stays exactly what it always was —
    // the MEMORY's own reference-edge walk (here, the two `references_*`
    // edges the cross-link save wrote) — and never gains an `imports` /
    // `co_changed` row: that walk lives entirely in the new `neighbors`
    // field instead.
    let relations = v["relations"].as_array().expect("relations array");
    assert_eq!(
        relations
            .iter()
            .map(|r| r["rel"].as_str())
            .collect::<Vec<_>>(),
        vec![Some("references_file"), Some("references_symbol")],
        "relations must stay the memory's own reference-edge walk: {v}"
    );
    assert!(
        relations
            .iter()
            .all(|r| r["rel"] != "imports" && r["rel"] != "co_changed"),
        "relations must never carry the code-graph walk: {v}"
    );

    // The new field: a.rs's real one-hop neighborhood.
    let neighbors = v["neighbors"].as_array().expect("neighbors array");
    assert_eq!(
        neighbors.len(),
        2,
        "exactly the imports + co_changed edge to b.rs: {v}"
    );
    for rel in ["imports", "co_changed"] {
        let n = neighbors
            .iter()
            .find(|n| n["rel"] == rel)
            .unwrap_or_else(|| panic!("{rel} neighbor missing: {v}"));
        assert_eq!(n["repo"], "n", "{rel} neighbor repo: {v}");
        assert_eq!(n["path"], "b.rs", "{rel} neighbor path: {v}");
        assert_eq!(n["weight"], 1, "{rel} neighbor weight: {v}");
    }
}

/// A bundle whose code refs are empty (no cross-links in the body) returns
/// an empty `neighbors` array and the command still exits 0.
#[test]
fn context_neighbors_empty_when_bundle_has_no_code_refs() {
    let home = TempDir::new().expect("tempdir");
    save_memory(&home, "plain body with no code references at all", "note");

    let v = context_json(&home, "plain body code references", &[]);
    assert!(
        v["code_refs"]
            .as_array()
            .expect("code_refs array")
            .is_empty(),
        "no cross-links in body: {v}"
    );
    assert!(
        v["neighbors"]
            .as_array()
            .expect("neighbors array")
            .is_empty(),
        "no code refs must yield an empty neighborhood: {v}"
    );
}
