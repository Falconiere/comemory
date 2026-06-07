//! G4: `comemory save` must record graph upsert failures in the stats
//! `index_failures` table, not just emit a `tracing::warn!`. This gives
//! `comemory doctor` (and operators tailing the stats file) a durable
//! signal that the graph path is broken instead of a vanished log line.
//!
//! We simulate a graph open failure by placing a regular file where
//! `Paths::graph_dir()` expects to find a directory. `Graph::open` calls
//! `create_dir_all` first, which errors with "Not a directory", which is
//! caught and recorded via `record_index_failure_best_effort`.

use comemory::config::paths::Paths;
use comemory::memory::Kind;
use comemory::stats::StatsDb;

use super::common;

#[tokio::test]
async fn save_records_graph_failure_in_stats() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    // Sabotage the graph path: place a regular file where the kuzu directory
    // is expected. `Graph::open` will fail at `create_dir_all`, which the
    // save flow swallows + records.
    let graph_path = paths.graph_dir();
    if let Some(parent) = graph_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&graph_path, b"NOT A DIRECTORY").unwrap();

    let args = comemory::cli::save::Args {
        body: Some("graph failure smoke body".into()),
        kind: Kind::Note,
        repo: "r".into(),
        tags: String::new(),
        author: "a".into(),
        quality: 3,
        // Skip the index path so this test isolates the graph failure
        // signal. Without --no-index the FTS/dense path would also record
        // a failure (different reason), masking the contract under test.
        no_index: true,
    };
    let result = comemory::cli::save::run(args, false, Some(paths.data_dir().to_path_buf())).await;
    assert!(
        result.is_ok(),
        "save must succeed despite graph failure (markdown is the source of truth), got {result:?}"
    );

    // The stats DB must reflect the swallowed graph failure.
    let db = StatsDb::open(paths.stats_db()).unwrap();
    let count = db.index_failure_count().unwrap();
    assert!(
        count >= 1,
        "graph failure must increment index_failures, got count={count}"
    );
    let last = db
        .last_index_failure()
        .unwrap()
        .expect("at least one row recorded");
    assert!(
        last.1.starts_with("graph: "),
        "graph failure rows are prefixed 'graph: ', got: {:?}",
        last.1
    );
}
