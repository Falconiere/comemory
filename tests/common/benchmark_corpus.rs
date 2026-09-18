#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The memory half of the benchmark fixture corpus, saved through the real
//! `domains::memories::save` path so the ids are the content-derived ones the
//! shipped set `tests/common/fixtures/benchmark-mixed-v1.yaml` names.
//!
//! Shared by the colocated benchmark tests and `tests/cli__benchmark.rs`, which
//! adds the code and document halves through the real binary.

use comemory::config::Config;
use comemory::config::paths::Paths;
use comemory::domains::memories::frontmatter::Kind;
use comemory::domains::memories::save;
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use tempfile::TempDir;

/// `(kind, body)` for the three fixture memories. The bodies are load-bearing:
/// `memories.id` is the 8-hex prefix of `sha256(body.trim_end())`, so editing
/// one invalidates the ids the shipped benchmark set judges.
pub const MEMORIES: [(Kind, &str); 3] = [
    (
        Kind::Bug,
        "Paged search reorders tied results.\n\nAccess tracking bumps only the returned page, so a gapped bump set floats over the rows it was behind.",
    ),
    (
        Kind::Decision,
        "Activation decay must be pinned for measurement.\n\nDisabling access tracking does not freeze activation decay; only a zero decay exponent removes the wall clock.",
    ),
    (
        Kind::Note,
        "Document chunking splits on paragraph boundaries.\n\nThe shared splitter keeps a heading breadcrumb for every passage it emits.",
    ),
];

/// The content-derived id of each entry in [`MEMORIES`], in the same order.
pub const MEMORY_IDS: [&str; 3] = ["3f116117", "0568bffe", "278ef8f4"];

/// The repo label every fixture memory is filed under.
pub const REPO: &str = "demo";

/// A tempdir data dir holding the three fixture memories.
pub fn seeded_home() -> (TempDir, Paths, Config) {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Paths::new(tmp.path().join(".comemory"));
    paths.ensure_dirs().expect("ensure dirs");
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).expect("open db");
    for (kind, body) in MEMORIES {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let saved =
            save::run(&mut ctx, request(kind, body), false, None).expect("save fixture memory");
        assert!(
            MEMORY_IDS.contains(&saved.id.as_str()),
            "fixture body hashed to {} which is not one of {MEMORY_IDS:?}; the bodies are \
             load-bearing and the shipped benchmark set's judgments name these ids",
            saved.id
        );
    }
    (tmp, paths, cfg)
}

/// One fixture memory's save request.
fn request(kind: Kind, body: &str) -> save::Request {
    save::Request {
        body: body.to_string(),
        title: None,
        kind,
        repo: REPO.to_string(),
        tags: Vec::new(),
        author: String::new(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    }
}
