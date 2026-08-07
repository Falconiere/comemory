#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end tests for `comemory edges` against the real binary.
//!
//! Every edge these tests query is written by a production path — `comemory
//! save --supersedes`, `comemory delete`, `comemory rebuild`, `comemory
//! index-code` — except the three mined kinds `rebuild` copies rather than
//! replays, which are seeded by raw INSERT in the exact id forms their
//! writers emit. No mocks, no hand-built `edge_fts` rows: the point is that
//! the derived index tracks the graph at every seam.

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::{TempDir, tempdir};

/// Run `comemory <args…>` under `home`'s data dir, returning stdout.
fn run(home: &TempDir, args: &[&str]) -> String {
    let out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).expect("stdout is utf-8")
}

/// Save a memory via the real binary and return its id.
fn save_id(home: &TempDir, args: &[&str]) -> String {
    let mut full_args = vec!["--json", "save"];
    full_args.extend_from_slice(args);
    let v: Value = serde_json::from_str(&run(home, &full_args)).expect("save emits json");
    v["id"].as_str().expect("save emits an id").to_string()
}

/// Run `comemory edges --json <args…>` and parse the envelope.
fn edges_json(home: &TempDir, args: &[&str]) -> Value {
    let mut full_args = vec!["--json", "edges"];
    full_args.extend_from_slice(args);
    serde_json::from_str(&run(home, &full_args)).expect("edges emits json")
}

/// One row's string field, naming the row when it is missing.
fn field(row: &Value, key: &str) -> String {
    row[key]
        .as_str()
        .unwrap_or_else(|| panic!("row missing {key}: {row}"))
        .to_string()
}

/// The `(rel, src_id, dst_id)` triples in one `--json` page, in page order.
fn triples(page: &Value) -> Vec<(String, String, String)> {
    page["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|r| (field(r, "rel"), field(r, "src_id"), field(r, "dst_id")))
        .collect()
}

/// Open `home`'s database with the identifier tokenizer registered — the
/// `edge_fts` vtab cannot be opened on a bare `rusqlite` connection.
fn open_db(home: &TempDir) -> Connection {
    comemory::store::connection::open(home.path().join("comemory.db")).expect("open db")
}

#[test]
fn a_supersede_edge_is_findable_and_leaves_on_delete() {
    let home = tempdir().expect("tempdir");
    let old = save_id(
        &home,
        &["--kind", "decision", "--repo", "demo", "queue design v1"],
    );
    let new = save_id(
        &home,
        &[
            "--kind",
            "decision",
            "--repo",
            "demo",
            "--supersedes",
            &old,
            "queue design v2",
        ],
    );

    // The save seam refreshed the index, so the relation answers immediately.
    assert_eq!(
        triples(&edges_json(&home, &["supersedes queue design"])),
        vec![("supersedes".into(), new.clone(), old.clone())],
    );
    // TTY renders the same relation as a triplet built from kind + slug.
    let tty = run(&home, &["edges", "supersedes queue design"]);
    assert!(
        tty.contains(
            "decision queue-design-v2 \u{2014}supersedes\u{2192} decision queue-design-v1"
        ),
        "unexpected TTY rendering: {tty}"
    );

    run(&home, &["delete", &old]);

    // The delete seam dropped every edge touching the removed endpoint and
    // the index followed. The page is not asserted empty: with the strict
    // tier now matching nothing, the word-OR fallback still answers with the
    // surviving memory's other relations. What must be gone is any triplet
    // naming the deleted memory.
    let after = triples(&edges_json(&home, &["supersedes queue design"]));
    assert!(
        !after.iter().any(|(_, src, dst)| src == &old || dst == &old),
        "deleting an endpoint must remove its relations from the index: {after:?}",
    );
    assert!(
        !after.iter().any(|(rel, _, _)| rel == "supersedes"),
        "the supersede relation must not survive its target: {after:?}",
    );
}

/// Seed the three mined edge kinds `comemory rebuild` copies from the old DB
/// rather than replaying from markdown (`co_changed` / `imports` from git
/// history, `co_activated` from the co-activation reward), in the id forms
/// `graph::materialize` writes.
fn seed_mined_edges(conn: &Connection, memory_id: &str) {
    let rows: [(&str, &str, &str, &str, i64); 3] = [
        (
            "file",
            "file:demo:src/queue.rs",
            "co_changed",
            "file:demo:src/worker.rs",
            3,
        ),
        (
            "file",
            "file:demo:src/worker.rs",
            "imports",
            "file:demo:src/queue.rs",
            1,
        ),
        (
            "memory",
            memory_id,
            "co_activated",
            "file:demo:src/queue.rs",
            1,
        ),
    ];
    for (src_kind, src_id, rel, dst_id, weight) in rows {
        conn.execute(
            "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, weight, created_at)
             VALUES(?1, ?2, 'file', ?3, ?4, ?5, '2026-07-01T00:00:00Z')",
            rusqlite::params![src_kind, src_id, dst_id, rel, weight],
        )
        .expect("seed mined edge");
    }
}

#[test]
fn rebuild_repopulates_replayed_and_copied_edges_alike() {
    let home = tempdir().expect("tempdir");
    let old = save_id(
        &home,
        &["--kind", "note", "--repo", "demo", "queue design v1"],
    );
    let new = save_id(
        &home,
        &[
            "--kind",
            "note",
            "--repo",
            "demo",
            "--supersedes",
            &old,
            "queue design v2",
        ],
    );
    {
        let conn = open_db(&home);
        seed_mined_edges(&conn, &new);
    }

    run(&home, &["rebuild"]);

    // Markdown-sourced relations come back through the replay…
    assert_eq!(
        triples(&edges_json(&home, &["supersedes queue design"])),
        vec![("supersedes".into(), new.clone(), old)],
    );
    // …and the three copied kinds come back through the ATTACH copy, with
    // the rebuild's derived-refresh indexing both sources in one pass.
    for (query, rel) in [
        ("co_changed queue", "co_changed"),
        ("imports queue", "imports"),
        ("co_activated queue", "co_activated"),
    ] {
        let rels: Vec<String> = triples(&edges_json(&home, &[query]))
            .into_iter()
            .map(|(r, _, _)| r)
            .collect();
        assert!(
            rels.contains(&rel.to_string()),
            "rebuild lost the {rel} edges: query `{query}` returned {rels:?}",
        );
    }
}

#[test]
fn index_code_makes_mined_import_edges_findable() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = workspace.path().join("import-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[
            ("main.rs", "mod parser;\n\nfn main() {}\n"),
            ("parser.rs", "fn parse() {}\n"),
        ],
        "seed the import pair",
    );

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "demo", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();

    // The index-code seam — added by F4 — refreshes the derived artifacts
    // after `materialize` writes the mined edges, so the import is queryable
    // with no intervening save.
    let found = triples(&edges_json(&home, &["imports parser"]));
    assert!(
        found.contains(&(
            "imports".to_string(),
            "file:demo:main.rs".to_string(),
            "file:demo:parser.rs".to_string(),
        )),
        "index-code did not surface the main.rs -> parser.rs import: {found:?}",
    );
}

#[test]
fn an_empty_index_self_heals_on_the_first_query() {
    let home = tempdir().expect("tempdir");
    let old = save_id(
        &home,
        &["--kind", "decision", "--repo", "demo", "queue design v1"],
    );
    let new = save_id(
        &home,
        &[
            "--kind",
            "decision",
            "--repo",
            "demo",
            "--supersedes",
            &old,
            "queue design v2",
        ],
    );

    // Simulate a database upgraded across migration 0012: `edges` is
    // populated, `edge_fts` is the empty table the migration created (it
    // deliberately ships no backfill — the rendering lives in Rust).
    {
        let conn = open_db(&home);
        conn.execute("DELETE FROM edge_fts", [])
            .expect("empty edge_fts");
        let indexed: i64 = conn
            .query_row("SELECT count(*) FROM edge_fts", [], |r| r.get(0))
            .expect("count edge_fts");
        assert_eq!(
            indexed, 0,
            "upgrade simulation must start from an empty index"
        );
    }

    assert_eq!(
        triples(&edges_json(&home, &["supersedes queue design"])),
        vec![("supersedes".into(), new, old)],
        "the first query must populate the index and answer in the same run",
    );
}

#[test]
fn json_rows_carry_the_documented_shape() {
    let home = tempdir().expect("tempdir");
    let old = save_id(
        &home,
        &["--kind", "decision", "--repo", "demo", "queue design v1"],
    );
    save_id(
        &home,
        &[
            "--kind",
            "decision",
            "--repo",
            "demo",
            "--supersedes",
            &old,
            "queue design v2",
        ],
    );

    let page = edges_json(&home, &["supersedes queue design"]);
    let row = &page["items"][0];
    assert_eq!(field(row, "src_kind"), "memory");
    assert_eq!(field(row, "dst_kind"), "memory");
    assert_eq!(field(row, "src_text"), "decision queue-design-v2");
    assert_eq!(field(row, "dst_text"), "decision queue-design-v1");
    assert_eq!(row["weight"], 1);
    assert!(
        row["score"].as_f64().is_some_and(|s| s > 0.0),
        "score must be the higher-is-better negated BM25: {row}"
    );
}

#[test]
fn paging_walks_distinct_rows_and_stops_at_the_end() {
    let home = tempdir().expect("tempdir");
    let old = save_id(
        &home,
        &[
            "--kind",
            "decision",
            "--repo",
            "demo",
            "--tags",
            "queue,design",
            "queue design v1",
        ],
    );
    save_id(
        &home,
        &[
            "--kind",
            "decision",
            "--repo",
            "demo",
            "--supersedes",
            &old,
            "queue design v2",
        ],
    );

    let first = edges_json(&home, &["queue", "--k", "1"]);
    assert_eq!(first["limit"], 1);
    assert_eq!(first["offset"], 0);
    assert_eq!(first["has_more"], true);
    assert!(first["total"].is_null(), "total stays uncounted: {first}");
    assert_eq!(triples(&first).len(), 1);

    // `--limit` is the documented alias for `--k`, and the second page holds
    // a different relation than the first.
    let second = edges_json(&home, &["queue", "--limit", "1", "--offset", "1"]);
    assert_eq!(triples(&second).len(), 1);
    assert_ne!(
        triples(&second),
        triples(&first),
        "page 2 must not repeat page 1",
    );

    let beyond = edges_json(&home, &["queue", "--k", "1", "--offset", "500"]);
    assert!(triples(&beyond).is_empty(), "past the end must be empty");
    assert_eq!(
        beyond["has_more"], false,
        "an exhausted page must not claim more",
    );
}
