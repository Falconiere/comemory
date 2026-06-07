//! C6: `comemory save --no-index` must skip the dense embed + FTS upsert
//! so batch / scripted saves don't pay the per-save embedder cold-load.
//! Markdown is still written; graph upsert is best-effort. The dense
//! `MemoryIndex` table and the SQLite FTS row count both remain empty.

use comemory::config::paths::Paths;
use comemory::index::{Fts, MemoryIndex};
use comemory::memory::{Kind, MemoryStore};

use super::common;

#[tokio::test]
async fn save_no_index_skips_dense_and_fts_writes() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    let args = comemory::cli::save::Args {
        body: Some("Postgres analytics decision".into()),
        kind: Kind::Decision,
        repo: "r".into(),
        tags: String::new(),
        author: "a".into(),
        quality: 3,
        no_index: true,
    };
    comemory::cli::save::run(args, false, Some(paths.data_dir().to_path_buf()))
        .await
        .unwrap();

    // Markdown was written.
    let store = MemoryStore::new(paths.clone());
    let listed = store.list().unwrap();
    assert_eq!(listed.len(), 1, "markdown should still be written");

    // Dense vector table holds no row — search returns empty. Use a zero
    // query vector so we don't load the embedder; if the LanceDB table
    // doesn't exist yet, `search` short-circuits to `[]` by design.
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    let zero = vec![0.0f32; 768];
    let hits = idx.search(&zero, 5).await.unwrap();
    assert!(
        hits.is_empty(),
        "--no-index must leave the dense MemoryIndex empty, got {hits:?}"
    );

    // FTS database may not exist yet (skipped entirely); when present it
    // must hold zero rows. Either way: no FTS row was upserted.
    let fts_path = paths.index_dir().join("fts.sqlite");
    if fts_path.exists() {
        let fts = Fts::open(&fts_path).unwrap();
        assert_eq!(fts.count().unwrap(), 0, "--no-index must leave FTS empty");
    }
}
