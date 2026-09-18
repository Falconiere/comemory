#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for the run environment against a real database: the corpus digest
//! has to move when the corpus does and repeat when it does not, and the
//! portable memory figure has to be present on every platform.

use comemory::domains::learning::evaluation::run_environment::{self, RunEnvironment};
use comemory::store::connection;

use crate::test_common::benchmark_corpus::seeded_home;

#[test]
fn the_corpus_snapshot_counts_live_rows_and_digests_them() {
    let (_tmp, paths, cfg) = seeded_home();
    let conn = connection::open(paths.db_path()).expect("open db");
    let version = run_environment::retrieval_version(&cfg, &conn).expect("version");

    assert_eq!(version.corpus.memories, 3);
    assert_eq!(version.corpus.code_symbols, 0);
    assert_eq!(version.corpus.documents, 0);
    assert!(version.corpus.repos.is_empty(), "nothing has been indexed");
    assert_eq!(version.corpus.digest.len(), 64);
    assert_eq!(
        version.schema_version,
        comemory::store::migrate::CURRENT_VERSION
    );
    assert_eq!(version.binary_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(version.knobs_hash.len(), 64);
}

#[test]
fn the_corpus_digest_repeats_for_one_corpus_and_moves_when_it_grows() {
    let (_tmp, paths, cfg) = seeded_home();
    let conn = connection::open(paths.db_path()).expect("open db");
    let first = run_environment::retrieval_version(&cfg, &conn).expect("version");
    let again = run_environment::retrieval_version(&cfg, &conn).expect("version");
    assert_eq!(first.corpus.digest, again.corpus.digest);

    conn.execute(
        "UPDATE memories SET deleted_at = '2026-09-18T00:00:00Z' WHERE id = \
         (SELECT id FROM memories LIMIT 1)",
        [],
    )
    .expect("soft-delete one memory");
    let after = run_environment::retrieval_version(&cfg, &conn).expect("version");
    assert_eq!(after.corpus.memories, 2, "a soft-deleted row is not live");
    assert_ne!(
        first.corpus.digest, after.corpus.digest,
        "a moved corpus must be a different snapshot"
    );
}

#[test]
fn the_knobs_hash_separates_two_pinned_configurations() {
    let (_tmp, paths, cfg) = seeded_home();
    let conn = connection::open(paths.db_path()).expect("open db");
    let live = run_environment::retrieval_version(&cfg, &conn).expect("version");
    let mut frozen = cfg.clone();
    frozen.rank.decay = 0.0;
    let pinned = run_environment::retrieval_version(&frozen, &conn).expect("version");

    assert!(!live.knobs.decay_frozen(), "the shipped default decays");
    assert!(pinned.knobs.decay_frozen());
    assert_ne!(live.knobs_hash, pinned.knobs_hash);
    assert_eq!(
        live.corpus.digest, pinned.corpus.digest,
        "the corpus did not move; only the knobs did"
    );
}

#[test]
fn the_environment_reports_hardware_and_a_portable_memory_figure() {
    let mut env = RunEnvironment::capture();
    assert!(!env.os.is_empty());
    assert!(!env.arch.is_empty());
    assert_eq!(env.observation_bytes, 0, "nothing has been observed yet");

    env.record_observation_bytes(120);
    env.record_observation_bytes(40);
    assert_eq!(env.observation_bytes, 160);
    assert_eq!(
        env.peak_task_observation_bytes, 120,
        "the peak is the largest task"
    );
    // Process RSS is Linux-only by design; the portable figure above is not.
    assert!(env.peak_rss_bytes.is_none_or(|bytes| bytes > 0));
}
