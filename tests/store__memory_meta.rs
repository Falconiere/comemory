//! Mirror tests for `src/store/memory_meta.rs`. Drives the real `comemory
//! save` binary to populate `comemory.db` (markdown + SQLite mirror + edges),
//! then asserts [`fetch_meta`] returns the navigation metadata each hit
//! carries: md_path, repo, kind, slug, tags, and the code references mined
//! from the body. No mocks — every row is produced by the production save
//! path.

use assert_cmd::Command;
use comemory::store::connection;
use comemory::store::memory_meta::fetch_meta;
use tempfile::tempdir;

/// Run `comemory save <body> --kind <kind> --repo <repo> [--tags <tags>]`
/// under `home`, returning the new memory id parsed from `--json` output.
fn save(home: &std::path::Path, body: &str, kind: &str, repo: &str, tags: &str) -> String {
    let mut args = vec![
        "--json", "save", "--kind", kind, "--repo", repo, "--tags", tags, body,
    ];
    // An empty `--tags ""` is accepted by the CLI, but skip the flag entirely
    // when no tags so the assertion exercises the no-tag path too.
    if tags.is_empty() {
        args = vec!["--json", "save", "--kind", kind, "--repo", repo, body];
    }
    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home)
        .args(&args)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).to_string();
    let v: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("save --json emits one JSON object");
    v.get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id in save output")
        .to_string()
}

#[test]
fn fetch_meta_returns_navigation_fields_for_real_saves() {
    let home = tempdir().expect("tempdir");

    // Body references a file (`qwick-backend:src/db.rs`) and a symbol
    // (`qwick-backend:src/db.rs:connect`) via backtick-free qualified tokens,
    // so cross-link extraction emits references_file + references_symbol edges.
    let id_a = save(
        home.path(),
        "decided to pool connections in qwick-backend:src/db.rs:connect for the api",
        "decision",
        "qwick-backend",
        "database,postgres",
    );
    let id_b = save(
        home.path(),
        "ast-grep finds patterns fast",
        "note",
        "tooling",
        "",
    );

    let db_path = home.path().join("comemory.db");
    let conn = connection::open(&db_path).expect("open db");

    let map = fetch_meta(&conn, &[id_a.as_str(), id_b.as_str()]).expect("fetch_meta");
    assert_eq!(
        map.len(),
        2,
        "both saved ids must resolve: {:?}",
        map.keys().collect::<Vec<_>>()
    );

    assert_ref_memory(map.get(&id_a).expect("id_a meta"), &id_a);
    assert_plain_memory(map.get(&id_b).expect("id_b meta"));
}

/// Assert the metadata for the tagged, ref-carrying decision (`id_a`).
fn assert_ref_memory(a: &comemory::store::memory_meta::MemoryMeta, id_a: &str) {
    assert_eq!(a.repo.as_deref(), Some("qwick-backend"));
    assert_eq!(a.kind, "decision");
    assert!(
        a.md_path.contains(id_a) && a.md_path.ends_with(".md"),
        "md_path must point at the markdown file: {}",
        a.md_path
    );
    assert!(!a.slug.is_empty(), "slug must be derived: {:?}", a.slug);
    let mut tags = a.tags.clone();
    tags.sort();
    assert_eq!(tags, vec!["database".to_string(), "postgres".to_string()]);
    // References are the bare qualified strings the edge dst_id carries.
    assert_eq!(
        a.references.files,
        vec!["qwick-backend:src/db.rs".to_string()]
    );
    assert_eq!(
        a.references.symbols,
        vec!["qwick-backend:src/db.rs:connect".to_string()]
    );
}

/// Assert the metadata for the untagged, ref-free note (`id_b`).
fn assert_plain_memory(b: &comemory::store::memory_meta::MemoryMeta) {
    assert_eq!(b.repo.as_deref(), Some("tooling"));
    assert_eq!(b.kind, "note");
    assert!(b.tags.is_empty(), "no tags were saved: {:?}", b.tags);
    assert!(
        b.references.files.is_empty() && b.references.symbols.is_empty(),
        "no refs in body: {:?}",
        b.references
    );
}

#[test]
fn fetch_meta_empty_ids_returns_empty_map() {
    let home = tempdir().expect("tempdir");
    // Open (and migrate) a DB without saving anything.
    let conn = connection::open(home.path().join("comemory.db")).expect("open db");
    let map = fetch_meta(&conn, &[]).expect("fetch_meta empty");
    assert!(map.is_empty(), "empty ids → empty map");
}

#[test]
fn fetch_meta_tolerates_missing_id() {
    let home = tempdir().expect("tempdir");
    let id = save(home.path(), "a real memory", "note", "r", "");
    let conn = connection::open(home.path().join("comemory.db")).expect("open db");

    // Mix a real id with one that was never saved: the missing id is simply
    // absent from the map, the real one resolves.
    let map = fetch_meta(&conn, &[id.as_str(), "deadbeef"]).expect("fetch_meta");
    assert_eq!(
        map.len(),
        1,
        "only the real id resolves: {:?}",
        map.keys().collect::<Vec<_>>()
    );
    assert!(map.contains_key(&id));
    assert!(!map.contains_key("deadbeef"));
}
