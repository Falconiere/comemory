//! Shared ranking smoke corpus: 20 realistic engineering memories plus the
//! recall@3 query set asserted by `tests/cli_rank_smoke.rs`.
//!
//! Content rules (measured, not aspirational):
//! - varied kinds / tags / quality, real engineering phrasing;
//! - deliberate identifier mentions (`VecDimMismatch`, `fs::rename`, ...)
//!   so the identifier tokenizer is exercised end-to-end;
//! - exactly one near-duplicate pair: the two `cargo nextest` notes differ
//!   by one trailing token and sit at SimHash Hamming 6 — within
//!   `NEAR_DUP_HAMMING = 8` (measured via `comemory::simhash::of_body`;
//!   pinned by `corpus_contains_exactly_one_near_duplicate_pair`). The pair
//!   doubles as the conceptually supersede-able pair (same fact, different
//!   wording). Every other pair sits above `NEAR_DUP_HAMMING` (the guard
//!   test enforces the count, not a specific floor; ≥ 19 was the measured
//!   minimum when the corpus was authored).

use std::collections::HashMap;

use comemory::eval::golden::GoldenPair;

/// `(kind, body, tags, quality)` rows fed to `comemory save`.
pub const CORPUS: &[(&str, &str, &str, u8)] = &[
    (
        "bug",
        "Postgres connection pool exhausts under load spikes; raise max_connections to 50 and add pgbouncer in transaction mode",
        "database,postgres",
        4,
    ),
    (
        "decision",
        "We store embeddings as little-endian f32 blobs in sqlite-vec vec0 tables; dims are baked into DDL at migration time",
        "sqlite,vectors",
        5,
    ),
    (
        "bug",
        "VecDimMismatch fires when the Ollama embedder returns 768 dims but memory_vec expects 1024 — check COMEMORY_EMBED_HINT",
        "vectors,ollama",
        4,
    ),
    (
        "convention",
        "All CLI subcommands accept --json and emit a single-line JSON envelope on stdout; exit codes follow sysexits.h",
        "cli,output",
        5,
    ),
    (
        "discovery",
        "FTS5 bm25() returns negative scores — lower is better; ORDER BY score ASC, not DESC",
        "sqlite,fts5",
        4,
    ),
    (
        "pattern",
        "Use tracing::warn for best-effort failures that must not break the read path, e.g. access tracking updates",
        "errors,tracing",
        3,
    ),
    (
        "bug",
        "git2 vendored-libgit2 build breaks on alpine without cmake; pin builder image to debian-slim",
        "ci,git",
        3,
    ),
    (
        "decision",
        "Tests live strictly in tests/ mirroring src/ 1:1; pub(crate) items get promoted to pub when integration tests need them",
        "testing,conventions",
        5,
    ),
    (
        "note",
        "cargo nextest profile serializes the embedder test group to avoid model download races",
        "testing,nextest",
        3,
    ),
    (
        "discovery",
        "ast-grep pattern '$A.unwrap()' finds unwraps; pair with scripts/no-bypass-check.sh allowlist for tests/",
        "ast-grep,lint",
        4,
    ),
    (
        "convention",
        "Conventional Commits with scope, e.g. feat(retrieval): ...; release tags are v*",
        "git,conventions",
        4,
    ),
    (
        "bug",
        "OAuth refresh race: two concurrent refreshes invalidate each other's tokens; serialize via per-user mutex",
        "auth,oauth",
        5,
    ),
    (
        "pattern",
        "RRF fusion with k=60 over FTS5 + vec0 KNN lists; candidates capped at 50 before rerank",
        "retrieval,ranking",
        4,
    ),
    (
        "note",
        "Homebrew tap publishes via cargo-dist on v* tags; PRs only get a dry-run plan",
        "release,homebrew",
        3,
    ),
    (
        "decision",
        "Markdown files under ~/.comemory/memories are the source of truth; comemory rebuild reconstructs the DB",
        "architecture,storage",
        5,
    ),
    (
        "bug",
        "Long camelCase identifiers like VecDimMismatch were unfindable before the identifier tokenizer split subtokens",
        "search,fts5",
        4,
    ),
    (
        "discovery",
        "SQLite ALTER TABLE ADD COLUMN cannot default to another column; backfill with a follow-up UPDATE",
        "sqlite,migrations",
        4,
    ),
    (
        "pattern",
        "Atomic file writes: stage to .{id}.tmp then fs::rename; remove the tmp on any failure",
        "io,reliability",
        4,
    ),
    // Deliberate near-duplicate of the `cargo nextest` note above (one-token
    // edit, SimHash Hamming 6): exercises the save-time duplicate warning and
    // the diversify collapse without sitting in any smoke query's target set.
    (
        "note",
        "cargo nextest profile serializes the embedder test group to avoid model download stampedes",
        "testing,nextest",
        2,
    ),
    (
        "convention",
        "Doc comments on every public item; rustfmt 100-col, 4-space indent",
        "style,docs",
        4,
    ),
];

/// `(query, expected substring of at least one top-3 body)` recall@3 floor.
///
/// Queries 2 and 3 both expect the "768 dims" memory, but the camelCase
/// tokenizer memory also mentions `VecDimMismatch` — the top-3 tolerance
/// absorbs that overlap by design.
pub const SMOKE_QUERIES: &[(&str, &str)] = &[
    ("postgres pool exhausted", "pgbouncer"),
    ("VecDimMismatch", "768 dims"),
    ("vec dim mismatch", "768 dims"),
    ("bm25 negative score", "lower is better"),
    ("oauth token race", "per-user mutex"),
    ("rrf fusion constant", "k=60"),
    ("rebuild database from markdown", "source of truth"),
    ("camelcase identifier search", "identifier tokenizer"),
    ("alter table default backfill", "follow-up UPDATE"),
    ("atomic write rename", "fs::rename"),
];

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
