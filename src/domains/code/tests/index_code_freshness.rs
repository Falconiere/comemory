#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Code index reconciliation and local model regressions.

use crate::test_common::git_sample;
use comemory::config::{Config, Paths};
use comemory::domains::code::index_code::IndexMode;
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use tempfile::tempdir;

fn ctx_over(home: &std::path::Path) -> (Paths, Config, rusqlite::Connection) {
    let paths = Paths::new(home);
    paths.ensure_dirs().expect("ensure dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (paths, Config::defaults(), conn)
}

#[test]
fn incremental_reconciles_unstaged_edits_and_deleted_tracked_files() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(home.path());
    let req = || crate::domains::code::index_code::Request {
        repo: "sample".into(),
        path: repo.to_string_lossy().into_owned(),
        mode: IndexMode::Incremental,
    };
    crate::domains::code::index_code::run(&mut Ctx::borrowed(&paths, &cfg, &mut conn), req())
        .expect("first index");
    std::fs::write(repo.join("src.rs"), "fn new_symbol() {}\n").expect("edit");
    let result =
        crate::domains::code::index_code::run(&mut Ctx::borrowed(&paths, &cfg, &mut conn), req())
            .expect("index unstaged edit");
    assert_eq!(result.files_indexed, 1);
    let names: Vec<String> = conn
        .prepare("SELECT symbol FROM code_symbols WHERE repo='sample'")
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("names");
    assert_eq!(names, ["new_symbol"]);
    std::fs::remove_file(repo.join("src.rs")).expect("delete");
    crate::domains::code::index_code::run(&mut Ctx::borrowed(&paths, &cfg, &mut conn), req())
        .expect("index deletion");
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM code_symbols WHERE repo='sample'",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(count, 0);
}

#[test]
fn folder_indexes_untracked_code_and_retries_missing_embeddings() {
    let home = tempdir().expect("tempdir");
    let folder = tempdir().expect("folder");
    std::fs::write(folder.path().join("example.rs"), "fn example() {}\n").expect("source");
    let (paths, mut cfg, mut conn) = ctx_over(home.path());
    let req = || crate::domains::code::index_code::Request {
        repo: "folder-source".into(),
        path: folder.path().to_string_lossy().into_owned(),
        mode: IndexMode::Incremental,
    };
    cfg.code_embed_cmd = Some("exit 1".into());
    cfg.code_embed_hint = Some("test-model".into());
    let first = crate::domains::code::index_code::run_folder_with_progress(
        &mut Ctx::borrowed(&paths, &cfg, &mut conn),
        req(),
        None,
    )
    .expect("lexical indexing survives provider failure");
    assert_eq!(first.files_indexed, 1);
    assert_eq!(first.embedding_failed, 1);
    let payload = home.path().join("embedding.json");
    std::fs::write(
        &payload,
        serde_json::json!({"embedding": vec![0.5; 768]}).to_string(),
    )
    .expect("payload");
    cfg.code_embed_cmd = Some(format!("cat '{}'", payload.display()));
    let second = crate::domains::code::index_code::run_folder_with_progress(
        &mut Ctx::borrowed(&paths, &cfg, &mut conn),
        req(),
        None,
    )
    .expect("retry");
    assert_eq!(second.files_indexed, 0);
    assert_eq!(second.embedded, 1);
    let third = crate::domains::code::index_code::run_folder_with_progress(
        &mut Ctx::borrowed(&paths, &cfg, &mut conn),
        req(),
        None,
    )
    .expect("unchanged");
    assert_eq!(third.embedded, 0);
    let full_req = || crate::domains::code::index_code::Request {
        mode: IndexMode::Full,
        ..req()
    };
    let full = crate::domains::code::index_code::run_folder_with_progress(
        &mut Ctx::borrowed(&paths, &cfg, &mut conn),
        full_req(),
        None,
    )
    .expect("full refresh");
    assert_eq!(full.embedded, 1);
    let vector_before: Vec<u8> = conn
        .query_row("SELECT embedding FROM code_vec", [], |row| row.get(0))
        .expect("vector before failed refresh");
    std::fs::write(&payload, "provider returned invalid JSON").expect("fail same provider");
    let failed = crate::domains::code::index_code::run_folder_with_progress(
        &mut Ctx::borrowed(&paths, &cfg, &mut conn),
        full_req(),
        None,
    )
    .expect("failed refresh retains lexical index");
    assert_eq!(failed.embedding_failed, 1);
    let vector_after: Vec<u8> = conn
        .query_row("SELECT embedding FROM code_vec", [], |row| row.get(0))
        .expect("vector after failed refresh");
    assert_eq!(vector_before, vector_after);
    std::fs::write(
        &payload,
        serde_json::json!({"embedding": vec![0.5; 768]}).to_string(),
    )
    .expect("repair provider");
    cfg.code_embed_cmd = Some(format!("cat '{}' # changed provider", payload.display()));
    let command_changed = crate::domains::code::index_code::run_folder_with_progress(
        &mut Ctx::borrowed(&paths, &cfg, &mut conn),
        req(),
        None,
    )
    .expect("provider command change");
    assert_eq!(command_changed.files_indexed, 0);
    assert_eq!(command_changed.embedded, 1);
    cfg.code_embed_hint = Some("next-model".into());
    let changed = crate::domains::code::index_code::run_folder_with_progress(
        &mut Ctx::borrowed(&paths, &cfg, &mut conn),
        req(),
        None,
    )
    .expect("model change");
    assert_eq!(changed.embedded, 1);
    cfg.indexing.max_file_bytes = 4;
    let oversized = crate::domains::code::index_code::run_folder_with_progress(
        &mut Ctx::borrowed(&paths, &cfg, &mut conn),
        req(),
        None,
    )
    .expect("oversized source is skipped");
    assert_eq!(oversized.unsupported, 1);
    assert_eq!(oversized.removed, 1);
    let symbols: i64 = conn
        .query_row(
            "SELECT count(*) FROM code_symbols WHERE repo = 'folder-source'",
            [],
            |row| row.get(0),
        )
        .expect("count symbols");
    assert_eq!(symbols, 0);
}

#[test]
#[ignore = "requires COMEMORY_TEST_CODE_EMBED_CMD with a local 768-dimensional model"]
fn real_code_inference_survives_rename_and_removes_deleted_results() {
    let cmd = std::env::var("COMEMORY_TEST_CODE_EMBED_CMD").expect("code model command");
    let home = tempdir().expect("tempdir");
    let folder = tempdir().expect("folder");
    let source = folder.path().join("embedding.rs");
    std::fs::write(&source, include_str!("../index_code/embedding.rs")).expect("real source");
    let (paths, mut cfg, mut conn) = ctx_over(home.path());
    cfg.code_embed_cmd = Some(cmd.clone());
    cfg.code_embed_hint = Some("test-code-model".into());
    let req = || crate::domains::code::index_code::Request {
        repo: "inference".into(),
        path: folder.path().to_string_lossy().into_owned(),
        mode: IndexMode::Incremental,
    };
    let run = |conn: &mut rusqlite::Connection| {
        crate::domains::code::index_code::run_folder_with_progress(
            &mut Ctx::borrowed(&paths, &cfg, conn),
            req(),
            None,
        )
        .expect("index")
    };
    assert!(run(&mut conn).embedded > 0);
    assert_eq!(run(&mut conn).embedded, 0);
    let query =
        comemory::utilities::embed::embed_query(&cmd, "validate and persist embedding generation")
            .expect("query embedding");
    let hits = comemory::store::vector::knn_code(&conn, &query, 10, Some("inference"), None)
        .expect("ANN search");
    assert!(!hits.is_empty());
    std::fs::rename(&source, folder.path().join("renamed.rs")).expect("rename");
    let renamed = run(&mut conn);
    assert_eq!(renamed.removed, 1);
    assert!(renamed.embedded > 0);
    std::fs::remove_file(folder.path().join("renamed.rs")).expect("delete");
    assert_eq!(run(&mut conn).removed, 1);
    assert!(
        comemory::store::vector::knn_code(&conn, &query, 10, Some("inference"), None)
            .expect("ANN search")
            .is_empty()
    );
}
