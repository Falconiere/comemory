#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/learning/evaluation/dataset_split.rs`.
//!
//! Every identity here is produced by the contract's own codec — the reference
//! strings are parsed with `parse_ref` rather than constructed by hand — so a
//! change to the encoding fails these assertions instead of passing them.

use comemory::domains::learning::evaluation::candidate_identity::parse_ref;
use comemory::domains::learning::evaluation::dataset_record::Split;
use comemory::domains::learning::evaluation::dataset_split::{
    GroupInput, SplitConfig, content_group, plan, query_group,
};

const MEMORY_V1: &str = "memory:5a9f19bc:aaaa";
const MEMORY_V2: &str = "memory:5a9f19bc:bbbb";
const CODE_A: &str = "code:demo:src/ranking.rs:activation_boost:1111";
const CODE_B: &str = "code:demo:src/ranking.rs:clamp:2222";
const CODE_OTHER_FILE: &str = "code:demo:src/fuse.rs:rrf:3333";
const DOC_CHUNK_0: &str = "document:0f1e2d3c:guides/chunking.md:rev1:0";
const DOC_CHUNK_3: &str = "document:0f1e2d3c:guides/chunking.md:rev2:3";
const DOC_OTHER: &str = "document:0f1e2d3c:guides/ranking.md:rev1:0";
const AT: &str = "2026-09-18T10:00:00.000000000Z";

fn key(reference: &str) -> String {
    content_group(&parse_ref(reference).expect("a contract reference parses"))
}

fn row<'a>(query: &'a str, content: &'a str, repo: Option<&'a str>, at: &'a str) -> GroupInput<'a> {
    GroupInput {
        query_group: query,
        content_group: content,
        repo,
        at,
    }
}

fn config(ratios: [f64; 3]) -> SplitConfig {
    SplitConfig {
        seed: "test-seed".to_string(),
        ratios,
        holdout_repo: None,
        holdout_since: None,
    }
}

#[test]
fn near_duplicate_queries_share_one_query_key() {
    let canonical = query_group("frontmatter contract");
    assert_eq!(query_group("Frontmatter, contract?"), canonical);
    assert_eq!(query_group("contract   frontmatter"), canonical);
    assert_eq!(query_group("CONTRACT frontmatter contract"), canonical);
    assert_ne!(
        query_group("frontmatter"),
        canonical,
        "dropping a term must change the key, or the grouping would merge \
         everything that shares one word"
    );
    assert!(canonical.starts_with("qg-"));
}

#[test]
fn adjacent_passages_and_repeated_versions_share_one_content_key() {
    assert_eq!(
        key(MEMORY_V1),
        key(MEMORY_V2),
        "two content versions of one memory are the same content"
    );
    assert_eq!(
        key(CODE_A),
        key(CODE_B),
        "two symbols of one source file are the code analogue of adjacent chunks"
    );
    assert_eq!(
        key(DOC_CHUNK_0),
        key(DOC_CHUNK_3),
        "two chunks of one document, at two revisions, are one document"
    );
    assert_ne!(key(CODE_A), key(CODE_OTHER_FILE));
    assert_ne!(key(DOC_CHUNK_0), key(DOC_OTHER));
    assert_ne!(key(MEMORY_V1), key(CODE_A));
}

#[test]
fn a_component_never_spans_two_splits() {
    let (qa, qb, qc) = (
        query_group("activation decay"),
        query_group("decay activation pinned"),
        query_group("zebra widget calibration"),
    );
    let (shared, other) = (key(MEMORY_V1), key(CODE_OTHER_FILE));
    let rows = vec![
        row(&qa, &shared, None, AT),
        row(&qb, &shared, None, AT),
        row(&qc, &other, None, AT),
    ];

    let planned = plan(&rows, &config([0.34, 0.33, 0.33]));

    assert_eq!(
        planned.per_row[0], planned.per_row[1],
        "two queries that retrieved the same content are one component"
    );
    assert_ne!(
        planned.per_row[0].0, planned.per_row[2].0,
        "a disjoint query and content pair is its own component"
    );
    assert_eq!(planned.assignments.len(), 2);
    let merged = planned
        .assignments
        .iter()
        .find(|a| a.group_id == planned.per_row[0].0)
        .expect("the merged component is reported");
    assert_eq!(merged.queries, 2);
    assert_eq!(merged.contents, 1);
    assert_eq!(merged.rows, 2);
    assert!(
        planned
            .assignments
            .windows(2)
            .all(|w| w[0].group_id <= w[1].group_id)
    );
}

#[test]
fn an_added_disjoint_component_moves_no_existing_assignment() {
    let (qa, qb) = (query_group("activation decay"), query_group("zebra widget"));
    let (ka, kb) = (key(MEMORY_V1), key(CODE_OTHER_FILE));
    let cfg = config([0.34, 0.33, 0.33]);
    let before = plan(&[row(&qa, &ka, None, AT)], &cfg);

    let after = plan(&[row(&qa, &ka, None, AT), row(&qb, &kb, None, AT)], &cfg);

    assert_eq!(
        before.per_row[0], after.per_row[0],
        "a stable hash must keep an existing component on its split as the corpus grows"
    );
    assert_eq!(after.assignments.len(), 2);
}

#[test]
fn a_reserved_repository_outranks_the_hash_for_its_whole_component() {
    let q = query_group("activation decay");
    let (code, memory) = (key(CODE_A), key(MEMORY_V1));
    let mut cfg = config([1.0, 0.0, 0.0]);
    let all_train = plan(
        &[row(&q, &code, Some("demo"), AT), row(&q, &memory, None, AT)],
        &cfg,
    );
    assert!(
        all_train.per_row.iter().all(|(_, s)| *s == Split::Train),
        "with a 1.0 train ratio the hash sends everything to train"
    );

    cfg.holdout_repo = Some("demo".to_string());
    let reserved = plan(
        &[row(&q, &code, Some("demo"), AT), row(&q, &memory, None, AT)],
        &cfg,
    );

    assert!(
        reserved.per_row.iter().all(|(_, s)| *s == Split::Holdout),
        "the memory row shares a component with the reserved repo's code row, so it is \
         reserved too — that is what stops the component from leaking"
    );
    assert_eq!(
        reserved.assignments[0].forced.as_deref(),
        Some("repo"),
        "the manifest must say the hash was overridden and why"
    );

    cfg.holdout_repo = Some("absent".to_string());
    let untouched = plan(&[row(&q, &code, Some("demo"), AT)], &cfg);
    assert_eq!(untouched.per_row[0].1, Split::Train);
    assert_eq!(untouched.assignments[0].forced, None);
}

#[test]
fn a_reserved_time_slice_forces_every_component_inside_it() {
    let q = query_group("activation decay");
    let content = key(MEMORY_V1);
    let mut cfg = config([1.0, 0.0, 0.0]);
    cfg.holdout_since = Some("2026-09-18T00:00:00.000000000Z".to_string());

    let inside = plan(&[row(&q, &content, None, AT)], &cfg);
    let before = plan(
        &[row(&q, &content, None, "2026-09-17T23:59:59.000000000Z")],
        &cfg,
    );

    assert_eq!(inside.per_row[0].1, Split::Holdout);
    assert_eq!(inside.assignments[0].forced.as_deref(), Some("since"));
    assert_eq!(before.per_row[0].1, Split::Train);
    assert_eq!(before.assignments[0].forced, None);
}

#[test]
fn the_ratios_route_every_group_and_an_empty_input_plans_nothing() {
    let q = query_group("activation decay");
    let content = key(MEMORY_V1);
    let rows = vec![row(&q, &content, None, AT)];

    assert_eq!(
        plan(&rows, &config([1.0, 0.0, 0.0])).per_row[0].1,
        Split::Train
    );
    assert_eq!(
        plan(&rows, &config([0.0, 1.0, 0.0])).per_row[0].1,
        Split::Validation
    );
    assert_eq!(
        plan(&rows, &config([0.0, 0.0, 1.0])).per_row[0].1,
        Split::Holdout
    );

    let empty = plan(&[], &config([0.7, 0.15, 0.15]));
    assert!(empty.per_row.is_empty());
    assert!(empty.assignments.is_empty());
}

#[test]
fn a_group_id_is_independent_of_the_order_rows_arrived_in() {
    let (qa, qb) = (query_group("activation decay"), query_group("decay pinned"));
    let content = key(MEMORY_V1);
    let cfg = config([0.34, 0.33, 0.33]);

    let forward = plan(
        &[row(&qa, &content, None, AT), row(&qb, &content, None, AT)],
        &cfg,
    );
    let backward = plan(
        &[row(&qb, &content, None, AT), row(&qa, &content, None, AT)],
        &cfg,
    );

    assert_eq!(forward.per_row[0].0, backward.per_row[0].0);
    assert_eq!(
        forward.assignments[0].group_id,
        backward.assignments[0].group_id
    );
    assert_eq!(forward.assignments[0].split, backward.assignments[0].split);
}
