//! Mirror for `src/retrieval/hybrid.rs`. The module now hosts only
//! `search_code`; memory-layer dense retrieval moved into the unified
//! `retrieval::fuse::search_memory_fused_with_fts(idx, None, ...)` entry
//! point.
//!
//! The dual-layer test in `dual.rs` already exercises `search_code` end-
//! to-end against an indexed repo. This file holds a low-cost smoke test
//! that opens an empty CodeIndex and asserts `search_code` returns `[]`
//! when no `code_chunks` table exists yet — the cheap fast path.

use comemory::config::paths::Paths;
use comemory::index::CodeIndex;
use comemory::retrieval::hybrid::search_code;

use super::common;

#[tokio::test]
async fn search_code_returns_empty_when_table_missing() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    let cidx = CodeIndex::open(paths.vectors_dir(), 768).await.unwrap();
    let zero = vec![0.0f32; 768];
    let hits = search_code(&cidx, &zero, 5, 0.0).await.unwrap();
    assert!(
        hits.is_empty(),
        "missing code_chunks table must short-circuit to []"
    );
}
