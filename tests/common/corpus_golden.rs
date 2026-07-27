//! Golden-pair builder over the shared smoke corpus.
//!
//! Split out of `corpus.rs` so binaries that only need the corpus data
//! (`CORPUS` / `SMOKE_QUERIES`) can `#[path]`-include the data file alone —
//! every `pub` item a test binary pulls in is then exercised, keeping
//! `-D warnings` (dead_code) green without `#[allow]`. Same rationale as
//! the `git_repo.rs` / `git_commit.rs` split.

use std::collections::HashMap;

use comemory::eval::golden::GoldenPair;

use crate::corpus::SMOKE_QUERIES;

/// Build golden eval pairs from the saved corpus: each smoke query's
/// relevant set is the saved id whose body contains the expected
/// substring. Generated per-run rather than checked in — corpus ids are
/// content-derived (8-hex of the body), so a static golden file would rot
/// on any body edit. Panics when a substring resolves to anything but
/// exactly one body: the recall@k bar in `tests/cli_rank_smoke.rs` assumes
/// single-id relevant sets (mirroring the old "expected substring appears
/// in the top-3" semantics), and a multi-match would silently weaken it.
pub fn golden_pairs(bodies: &HashMap<String, String>) -> Vec<GoldenPair> {
    SMOKE_QUERIES
        .iter()
        .map(|(query, expected)| {
            let relevant: Vec<String> = bodies
                .iter()
                .filter(|(_, body)| body.contains(expected))
                .map(|(id, _)| id.clone())
                .collect();
            assert_eq!(
                relevant.len(),
                1,
                "smoke query {query:?}: expected substring {expected:?} must identify exactly \
                 one corpus body, got {relevant:?}"
            );
            GoldenPair {
                query: (*query).to_string(),
                relevant,
                repo: None,
                kind: None,
            }
        })
        .collect()
}
