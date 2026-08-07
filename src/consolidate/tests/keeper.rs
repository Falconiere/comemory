#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror for `comemory::consolidate::keeper` — the keeper order and the
//! in-cluster supersede resolution.

use comemory::consolidate::cluster::Group;
use comemory::consolidate::keeper::build;
use comemory::simhash::of_body;
use comemory::store::simhash_scan::SimhashRow;

/// One live memory with the stats the keeper order reads.
struct Row<'a> {
    id: &'a str,
    body: &'a str,
    quality: u8,
    access_count: i64,
    last_accessed: &'a str,
    rank_score: f64,
}

impl<'a> Row<'a> {
    /// A neutral row — every keeper signal at its floor.
    fn new(id: &'a str, body: &'a str) -> Self {
        Self {
            id,
            body,
            quality: 3,
            access_count: 0,
            last_accessed: "2026-07-01T00:00:00Z",
            rank_score: 0.0,
        }
    }
}

/// Open a real store and insert every row with its production fingerprint.
fn store(rows: &[Row<'_>]) -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    for r in rows {
        conn.execute(
            "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                                  body, created_at, updated_at, md_path, simhash,
                                  access_count, last_accessed, rank_score)
             VALUES (?1, ?1, 'note', 'demo', 'f', ?2, 1, ?1, ?3,
                     '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z', ?1, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                r.id,
                r.quality,
                r.body,
                of_body(r.body) as i64,
                r.access_count,
                r.last_accessed,
                r.rank_score
            ],
        )
        .expect("insert memory");
    }
    (dir, conn)
}

/// The group covering every inserted row, as the clustering pass would emit.
fn group(rows: &[Row<'_>]) -> Group {
    Group {
        rows: rows
            .iter()
            .map(|r| SimhashRow {
                id: r.id.to_string(),
                simhash: of_body(r.body) as i64,
            })
            .collect(),
        max_hamming: 7,
    }
}

/// `src` supersedes `dst`, written the way `memory_row::insert_edges` does.
fn supersedes(conn: &rusqlite::Connection, src: &str, dst: &str) {
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
         VALUES('memory',?1,'memory',?2,'supersedes','2026-07-01T00:00:00Z')",
        rusqlite::params![src, dst],
    )
    .expect("insert supersedes edge");
}

/// Ids in reported order — index 0 is the suggested keeper.
fn order(conn: &rusqlite::Connection, rows: &[Row<'_>]) -> Vec<String> {
    build(conn, &group(rows))
        .expect("build cluster")
        .members
        .into_iter()
        .map(|m| m.id)
        .collect()
}

const BODY_A: &str = "postgres advisory lock ordering fix for the migration runner";
const BODY_B: &str = "postgres advisory lock ordering fix for the migration worker";

#[test]
fn quality_outranks_every_other_signal() {
    let mut low = Row::new("aaaa0001", BODY_A);
    low.access_count = 99;
    low.rank_score = 9.0;
    low.last_accessed = "2026-07-28T00:00:00Z";
    let mut high = Row::new("bbbb0001", BODY_B);
    high.quality = 5;
    let rows = [low, high];
    let (_d, conn) = store(&rows);
    assert_eq!(
        order(&conn, &rows)[0],
        "bbbb0001",
        "quality 5 keeps even against a heavily accessed, high-rank rival"
    );
}

#[test]
fn access_count_breaks_a_quality_tie() {
    let mut cold = Row::new("aaaa0001", BODY_A);
    cold.rank_score = 9.0;
    let mut hot = Row::new("bbbb0001", BODY_B);
    hot.access_count = 12;
    let rows = [cold, hot];
    let (_d, conn) = store(&rows);
    assert_eq!(
        order(&conn, &rows)[0],
        "bbbb0001",
        "with quality tied the more-retrieved memory keeps"
    );
}

#[test]
fn recency_breaks_a_quality_and_access_tie() {
    let stale = Row::new("aaaa0001", BODY_A);
    let mut fresh = Row::new("bbbb0001", BODY_B);
    fresh.last_accessed = "2026-07-28T00:00:00Z";
    let rows = [stale, fresh];
    let (_d, conn) = store(&rows);
    assert_eq!(
        order(&conn, &rows)[0],
        "bbbb0001",
        "same quality and access count falls through to last_accessed"
    );
}

#[test]
fn rank_score_breaks_the_next_tie() {
    let unranked = Row::new("aaaa0001", BODY_A);
    let mut ranked = Row::new("bbbb0001", BODY_B);
    ranked.rank_score = 0.42;
    let rows = [unranked, ranked];
    let (_d, conn) = store(&rows);
    assert_eq!(
        order(&conn, &rows)[0],
        "bbbb0001",
        "the materialized PageRank is the last real signal"
    );
}

#[test]
fn the_id_is_the_final_total_order_tiebreak() {
    let rows = [Row::new("bbbb0001", BODY_B), Row::new("aaaa0001", BODY_A)];
    let (_d, conn) = store(&rows);
    assert_eq!(
        order(&conn, &rows),
        vec!["aaaa0001".to_string(), "bbbb0001".to_string()],
        "all signals tied: the lowest id keeps, so the report never shuffles"
    );
}

#[test]
fn the_keeper_carries_its_own_stats_and_a_zero_distance() {
    let mut best = Row::new("bbbb0001", BODY_B);
    best.quality = 5;
    best.access_count = 4;
    let rows = [Row::new("aaaa0001", BODY_A), best];
    let (_d, conn) = store(&rows);
    let cluster = build(&conn, &group(&rows)).expect("build cluster");
    let keeper = &cluster.members[0];
    assert_eq!(keeper.id, "bbbb0001");
    assert_eq!(keeper.hamming_to_keeper, 0, "the keeper is its own anchor");
    assert_eq!(keeper.quality, 5);
    assert_eq!(keeper.access_count, 4);
    assert_eq!(keeper.repo.as_deref(), Some("demo"));
    assert!(
        cluster.members[1].hamming_to_keeper > 0,
        "a non-keeper reports its real distance to the keeper"
    );
}

#[test]
fn a_chain_ending_on_a_non_keeper_still_resolves_the_cluster() {
    // The superseder is the LOW-quality member, so a keeper-relative rule
    // would call this unresolved. Resolution is judged inside the cluster.
    let mut keeper = Row::new("aaaa0001", BODY_A);
    keeper.quality = 5;
    let rows = [keeper, Row::new("bbbb0001", BODY_B)];
    let (_d, conn) = store(&rows);
    supersedes(&conn, "bbbb0001", "aaaa0001");

    let cluster = build(&conn, &group(&rows)).expect("build cluster");
    assert_eq!(cluster.members[0].id, "aaaa0001", "quality still keeps");
    assert!(
        cluster.resolved,
        "all but one member superseded from inside the cluster = handled"
    );
    assert_eq!(
        cluster.members[0].superseded_by.as_deref(),
        Some("bbbb0001"),
        "the keeper records the member that supersedes it"
    );
    assert_eq!(cluster.members[1].superseded_by, None);
}

#[test]
fn a_partially_superseded_cluster_stays_unresolved() {
    let rows = [
        Row::new("aaaa0001", BODY_A),
        Row::new("bbbb0001", BODY_B),
        Row::new(
            "cccc0001",
            "postgres advisory lock ordering fix for the migration daemon",
        ),
    ];
    let (_d, conn) = store(&rows);
    supersedes(&conn, "aaaa0001", "bbbb0001");

    let cluster = build(&conn, &group(&rows)).expect("build cluster");
    assert!(
        !cluster.resolved,
        "one of three still unsuperseded leaves real work to do"
    );
    let superseded: Vec<&str> = cluster
        .members
        .iter()
        .filter(|m| m.superseded_by.is_some())
        .map(|m| m.id.as_str())
        .collect();
    assert_eq!(
        superseded,
        vec!["bbbb0001"],
        "only the member with an incoming edge is marked"
    );
}

#[test]
fn supersede_edges_pointing_outside_the_cluster_do_not_resolve_it() {
    let rows = [Row::new("aaaa0001", BODY_A), Row::new("bbbb0001", BODY_B)];
    let (_d, conn) = store(&rows);
    // An outsider supersedes a member: real edge, but it says nothing about
    // the duplication *inside* this cluster.
    supersedes(&conn, "dddd0001", "aaaa0001");

    let cluster = build(&conn, &group(&rows)).expect("build cluster");
    assert!(!cluster.resolved);
    assert!(
        cluster.members.iter().all(|m| m.superseded_by.is_none()),
        "only in-cluster superseders count"
    );
}
