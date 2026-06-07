//! Task 7: `comemory save` must populate the dense `MemoryIndex` and the
//! lexical `Fts` table, in addition to writing markdown + the graph node.
//! Both index writes are best-effort — markdown remains the source of
//! truth — but on a happy-path save the row count and a topK hit MUST
//! reflect the new memory.

use comemory::config::paths::Paths;
use comemory::index::{Embedder, Fts, MemoryIndex};
use comemory::memory::{Kind, MemoryStore};

use super::common;

#[tokio::test]
async fn save_writes_into_memory_index_and_fts() {
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
        no_index: false,
    };
    comemory::cli::save::run(args, false, Some(paths.data_dir().to_path_buf()))
        .await
        .unwrap();

    let store = MemoryStore::new(paths.clone());
    let listed = store.list().unwrap();
    assert_eq!(listed.len(), 1);
    let id = listed[0].frontmatter.id.clone();

    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    let mut emb = Embedder::nomic_text().unwrap();
    let q = emb.embed_one("postgres analytics").unwrap();
    let hits = idx.search(&q, 5).await.unwrap();
    assert!(hits.iter().any(|h| h.id == id), "vector index missing save");

    let fts = Fts::open(paths.fts_db()).unwrap();
    assert_eq!(fts.count().unwrap(), 1);
}
