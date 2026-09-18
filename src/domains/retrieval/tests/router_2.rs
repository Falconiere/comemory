#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for the graph-expansion leg inside
//! [`comemory::retrieval::router::route`] — the overflow half of
//! `tests/retrieval__router.rs`, which is at its size cap.
//!
//! Every case is lexical-only (`vec = None`) and turns on one fixture: a
//! memory that matches the query, plus two that share no token with it and
//! can only arrive by walking `edges`.

use comemory::config::Config;
use comemory::retrieval::router::{self, RoutedHit, Source};
use comemory::retrieval::scope::{Filters, TimeScope};
use comemory::store::{connection, fts};
use tempfile::tempdir;

/// Insert a live `memories` row and index its body for FTS.
fn seed_indexed(conn: &rusqlite::Connection, id: &str, body: &str) {
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES(?1,'x','note','h',?2,'t','t','x.md')",
        rusqlite::params![id, body],
    )
    .expect("seed memory");
    fts::index_memory(conn, id, body, "").expect("fts");
}

/// Insert a memory→memory edge in the production `('memory', <id>)` form.
fn seed_relation(conn: &rusqlite::Connection, src: &str, dst: &str, rel: &str) {
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
         VALUES('memory',?1,'memory',?2,?3,'t')",
        rusqlite::params![src, dst, rel],
    )
    .expect("seed edge");
}

/// `aaaa0001` matches "advisory lock" lexically; `bbbb0002` (one hop) and
/// `cccc0003` (two hops, behind `bbbb0002`) share no token with the query,
/// so only `edges` can bring them in. `linked` writes those edges or not.
fn seed_dark_chain(conn: &rusqlite::Connection, linked: bool) {
    seed_indexed(conn, "aaaa0001", "advisory lock postgres migration");
    seed_indexed(conn, "bbbb0002", "kubernetes ingress rewrite rules");
    seed_indexed(conn, "cccc0003", "grafana dashboard retention window");
    if linked {
        seed_relation(conn, "bbbb0002", "aaaa0001", "derived_from");
        seed_relation(conn, "cccc0003", "bbbb0002", "derived_from");
    }
}

/// Route "advisory lock" over `conn` with no vector.
fn route_dark(cfg: &Config, conn: &rusqlite::Connection) -> Vec<RoutedHit> {
    router::route(
        cfg,
        conn,
        "advisory lock",
        None,
        Filters::none(),
        router::CANDIDATE_POOL,
    )
    .expect("route")
}

/// Open a migrated database seeded with the dark-chain fixture.
fn dark_db(dir: &std::path::Path, name: &str, linked: bool) -> rusqlite::Connection {
    let conn = connection::open(dir.join(name)).expect("open");
    seed_dark_chain(&conn, linked);
    conn
}

#[test]
fn graph_leg_surfaces_a_lexically_dark_neighbor() {
    let dir = tempdir().expect("tempdir");
    let conn = dark_db(dir.path(), "c.db", true);

    let hits = route_dark(&Config::defaults(), &conn);
    let dark = hits
        .iter()
        .find(|h| h.memory_id == "bbbb0002")
        .expect("edge-reachable memory must surface");
    assert_eq!(
        dark.source,
        Source::Graph,
        "graph-only hit is labeled Graph"
    );
    assert_eq!(dark.tier, 0, "a graph hit never walked the lexical ladder");

    let seed = hits
        .iter()
        .find(|h| h.memory_id == "aaaa0001")
        .expect("the lexical hit must survive fusion");
    assert_eq!(
        (seed.source, seed.tier),
        (Source::Lexical, 1),
        "ids the lexical leg produced keep today's label"
    );
}

#[test]
fn nearer_dark_neighbor_ranks_above_the_farther_one() {
    let dir = tempdir().expect("tempdir");
    let conn = dark_db(dir.path(), "c.db", true);

    let hits = route_dark(&Config::defaults(), &conn);
    let rank = |id: &str| {
        hits.iter()
            .position(|h| h.memory_id == id)
            .unwrap_or_else(|| panic!("{id} missing from {hits:?}"))
    };
    assert!(
        rank("bbbb0002") < rank("cccc0003"),
        "one hop must outrank two: {hits:?}"
    );
}

#[test]
fn graph_hops_zero_reproduces_the_edgeless_ranking() {
    // The same corpus twice: once with edges and the leg disabled, once
    // with no edges at all. Disabling must be indistinguishable from having
    // nothing to walk.
    let dir = tempdir().expect("tempdir");
    let linked = dark_db(dir.path(), "linked.db", true);
    let bare = dark_db(dir.path(), "bare.db", false);

    let mut off = Config::defaults();
    off.retrieval.graph_hops = 0;
    let disabled = route_dark(&off, &linked);
    let edgeless = route_dark(&Config::defaults(), &bare);

    let shape = |hits: &[RoutedHit]| -> Vec<(String, Source, u8)> {
        hits.iter()
            .map(|h| (h.memory_id.clone(), h.source, h.tier))
            .collect()
    };
    assert_eq!(shape(&disabled), shape(&edgeless));
    assert_eq!(
        shape(&disabled),
        vec![("aaaa0001".to_string(), Source::Lexical, 1)],
        "the dark pair must not surface with the leg off"
    );
}

/// Give a seeded row a real `created_at` (the shared fixture stores the
/// placeholder `'t'`, which no created-date window can compare against).
fn set_created(conn: &rusqlite::Connection, id: &str, created: &str) {
    conn.execute(
        "UPDATE memories SET created_at = ?2 WHERE id = ?1",
        rusqlite::params![id, created],
    )
    .expect("set created_at");
}

/// Route "advisory lock" with an inclusive created-date cutoff.
fn route_dark_until(cfg: &Config, conn: &rusqlite::Connection, cutoff: &str) -> Vec<RoutedHit> {
    let scope = TimeScope {
        cutoff: Some(cutoff.to_string()),
        ..TimeScope::none()
    };
    router::route(
        cfg,
        conn,
        "advisory lock",
        None,
        Filters {
            scope: &scope,
            ..Filters::none()
        },
        router::CANDIDATE_POOL,
    )
    .expect("route")
}

#[test]
fn graph_leg_dark_neighbor_is_hidden_by_an_earlier_cutoff() {
    // AC-5. The dark neighbour is only reachable through `edges`, so this
    // is the leg's own filter under test: the walk still *reaches*
    // bbbb0002 from the in-window seed, and the final join must drop it
    // for being created after the cutoff.
    let dir = tempdir().expect("tempdir");
    let conn = dark_db(dir.path(), "c.db", true);
    set_created(&conn, "aaaa0001", "2026-03-01T00:00:00Z");
    set_created(&conn, "bbbb0002", "2026-05-01T00:00:00Z");
    set_created(&conn, "cccc0003", "2026-05-02T00:00:00Z");
    let cfg = Config::defaults();

    let unscoped = route_dark(&cfg, &conn);
    assert!(
        unscoped.iter().any(|h| h.memory_id == "bbbb0002"),
        "baseline: the dark neighbour surfaces without a scope: {unscoped:?}"
    );

    let scoped = route_dark_until(&cfg, &conn, "2026-04-01T00:00:00Z");
    assert!(
        scoped.iter().any(|h| h.memory_id == "aaaa0001"),
        "the in-window lexical seed must survive: {scoped:?}"
    );
    assert!(
        !scoped.iter().any(|h| h.memory_id == "bbbb0002"),
        "a neighbour created after the cutoff must not surface: {scoped:?}"
    );
    assert!(
        !scoped.iter().any(|h| h.memory_id == "cccc0003"),
        "nor the two-hop neighbour behind it: {scoped:?}"
    );
}
