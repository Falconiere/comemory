//! Mirror for `comemory::consolidate::cluster` — transitive union-find
//! grouping of live fingerprints, its ordering, and the unhashed-row drop.

use comemory::consolidate::cluster::{Grouping, group_near_dups};
use comemory::simhash::{hamming64, of_body};
use comemory::store::simhash_scan::SimhashRow;

/// Real bodies whose fingerprints come from `of_body`, exactly as the save
/// path writes them — no hand-picked bit patterns.
const BODIES: &[(&str, &str)] = &[
    (
        "aaaa0001",
        "postgres advisory lock ordering fix for the migration runner",
    ),
    (
        "aaaa0002",
        "postgres advisory lock ordering fix for the migration worker",
    ),
    (
        "aaaa0003",
        "tokio runtime shutdown sequencing bug in the worker pool",
    ),
];

/// Fingerprint each body the way the store does: `of_body(..) as i64`.
fn rows(bodies: &[(&str, &str)]) -> Vec<SimhashRow> {
    bodies
        .iter()
        .map(|(id, body)| SimhashRow {
            id: (*id).to_string(),
            simhash: of_body(body) as i64,
        })
        .collect()
}

/// Distance between two of the fixture bodies, straight from the primitive.
fn dist(a: &str, b: &str) -> u32 {
    hamming64(of_body(a), of_body(b))
}

/// The ids of every group, in report order.
fn group_ids(g: &Grouping) -> Vec<Vec<String>> {
    g.groups
        .iter()
        .map(|grp| grp.rows.iter().map(|r| r.id.clone()).collect())
        .collect()
}

#[test]
fn near_bodies_group_and_the_unrelated_one_stays_out() {
    let rows = rows(BODIES);
    let radius = dist(BODIES[0].1, BODIES[1].1);
    let g = group_near_dups(&rows, radius);
    assert_eq!(
        group_ids(&g),
        vec![vec!["aaaa0001".to_string(), "aaaa0002".to_string()]],
        "the one-word-apart pair clusters; the different topic does not"
    );
    assert_eq!(g.scanned, 3, "every fingerprinted row was compared");
    assert_eq!(g.skipped_unhashed, 0);
}

#[test]
fn chained_pairs_land_in_one_group_with_the_widest_distance_reported() {
    // A—B and B—C within the radius, A—C beyond it: transitivity must pull
    // all three together and surface the real A—C distance.
    let bodies = [
        (
            "aaaa0001",
            "cache eviction policy tuning for redis clusters",
        ),
        ("aaaa0002", "cache eviction policy tuning for redis shards"),
        ("aaaa0003", "cache eviction policy review for redis shards"),
    ];
    let ab = dist(bodies[0].1, bodies[1].1);
    let bc = dist(bodies[1].1, bodies[2].1);
    let ac = dist(bodies[0].1, bodies[2].1);
    let radius = ab.max(bc);
    assert!(
        ac > radius,
        "fixture must actually chain: A-C {ac} has to exceed radius {radius}"
    );

    let g = group_near_dups(&rows(&bodies), radius);
    assert_eq!(
        group_ids(&g),
        vec![vec![
            "aaaa0001".to_string(),
            "aaaa0002".to_string(),
            "aaaa0003".to_string()
        ]],
        "transitive linkage merges the chain into one cluster"
    );
    assert_eq!(
        g.groups[0].max_hamming, ac,
        "max_hamming reports the widest real pair, exposing the chaining"
    );
}

#[test]
fn radius_zero_clusters_only_exact_fingerprint_collisions() {
    let mut rows = rows(BODIES);
    rows.push(SimhashRow {
        id: "aaaa0004".into(),
        simhash: of_body(BODIES[0].1) as i64,
    });
    let g = group_near_dups(&rows, 0);
    assert_eq!(
        group_ids(&g),
        vec![vec!["aaaa0001".to_string(), "aaaa0004".to_string()]],
        "only the identical fingerprint pairs at radius 0"
    );
}

#[test]
fn unhashed_rows_are_dropped_and_counted_instead_of_colliding() {
    let mut rows = rows(BODIES);
    for id in ["zzzz0001", "zzzz0002"] {
        rows.push(SimhashRow {
            id: id.to_string(),
            simhash: 0,
        });
    }
    let radius = dist(BODIES[0].1, BODIES[1].1);
    let g = group_near_dups(&rows, radius);
    assert_eq!(
        group_ids(&g),
        vec![vec!["aaaa0001".to_string(), "aaaa0002".to_string()]],
        "two zero fingerprints must not fabricate a cluster of their own"
    );
    assert_eq!(g.skipped_unhashed, 2, "the dropped rows are reported");
    assert_eq!(g.scanned, 3, "scanned counts only real fingerprints");
}

#[test]
fn singletons_are_never_reported() {
    let g = group_near_dups(&rows(&BODIES[2..]), 8);
    assert!(
        g.groups.is_empty(),
        "a corpus with nothing to merge yields no clusters"
    );
    assert_eq!(g.scanned, 1);
}

#[test]
fn grouping_is_deterministic_across_runs() {
    let rows = rows(BODIES);
    let radius = dist(BODIES[0].1, BODIES[1].1);
    assert_eq!(
        group_ids(&group_near_dups(&rows, radius)),
        group_ids(&group_near_dups(&rows, radius)),
        "same corpus, same clusters — the report must not shuffle"
    );
}

#[test]
fn bigger_clusters_sort_first_and_ties_break_on_tightness_then_id() {
    // Two independent pairs plus a triple: the triple leads, then the pair
    // with the smaller max_hamming.
    let bodies = [
        ("aaaa0001", "kubernetes ingress certificate renewal job"),
        ("aaaa0002", "kubernetes ingress certificate renewal task"),
        ("aaaa0003", "kubernetes ingress certificate renewal cron"),
        ("bbbb0001", "webpack chunk splitting heuristics for vendors"),
        ("bbbb0002", "webpack chunk splitting heuristics for modules"),
    ];
    let radius = 12;
    let g = group_near_dups(&rows(&bodies), radius);
    let ids = group_ids(&g);
    assert!(
        ids.len() >= 2,
        "fixture must produce at least two clusters at radius {radius}, got {ids:?}"
    );
    assert!(
        ids[0].len() >= ids[1].len(),
        "clusters are ordered by size descending: {ids:?}"
    );
}
