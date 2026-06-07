//! G7: `comemory index --rebuild` must re-embed every markdown memory whose
//! id is missing from the LanceDB `memory_chunks` table. The flow saves
//! two memories with `--no-index` (so the dense table stays empty), then
//! invokes the rebuild command and asserts both ids land in the index.

use comemory::config::paths::Paths;
use comemory::index::MemoryIndex;
use comemory::memory::{Kind, MemoryStore};

use super::common;

#[tokio::test]
async fn index_rebuild_reembeds_missing_memories() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    // Two memories saved without indexing — dense table stays empty.
    for (kind, body) in [
        (Kind::Decision, "Postgres analytics decision"),
        (Kind::Note, "Reminder about nextest serialisation"),
    ] {
        let args = comemory::cli::save::Args {
            body: Some(body.into()),
            kind,
            repo: "r".into(),
            tags: String::new(),
            author: "a".into(),
            quality: 3,
            no_index: true,
        };
        comemory::cli::save::run(args, false, Some(paths.data_dir().to_path_buf()))
            .await
            .unwrap();
    }

    // Sanity: the dense table is empty before rebuild.
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    let before = idx.list_ids().await.unwrap();
    assert!(
        before.is_empty(),
        "dense index must start empty, got {before:?}"
    );

    // Run rebuild.
    let args = comemory::cli::index::Args {
        rebuild: true,
        quiet: true,
    };
    comemory::cli::index::run(args, false, Some(paths.data_dir().to_path_buf()))
        .await
        .unwrap();

    // Both ids must now be present in the dense index.
    let store = MemoryStore::new(paths.clone());
    let on_disk: Vec<String> = store
        .list()
        .unwrap()
        .into_iter()
        .map(|m| m.frontmatter.id)
        .collect();
    assert_eq!(on_disk.len(), 2);

    let after = idx.list_ids().await.unwrap();
    for id in &on_disk {
        assert!(
            after.iter().any(|x| x == id),
            "rebuild missed id {id}; indexed ids: {after:?}"
        );
    }
}

/// `comemory index` without `--rebuild` must error: we don't silently no-op.
#[tokio::test]
async fn index_without_rebuild_errors() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    let args = comemory::cli::index::Args {
        rebuild: false,
        quiet: true,
    };
    let result = comemory::cli::index::run(args, false, Some(paths.data_dir().to_path_buf())).await;
    assert!(
        result.is_err(),
        "comemory index without --rebuild must error, got {result:?}"
    );
}
