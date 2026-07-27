//! Tests for [`comemory::retrieval::graph_route::expand_memory_seeds`].
//!
//! Every case runs against a real migrated `comemory.db` with edges written
//! in the production id forms: memory nodes are `('memory', <8-hex>)`,
//! `references_symbol` destinations are the bare `<repo>:<path>:<symbol>`
//! text `cross_link` emits, and hub nodes are `('repo'|'author'|'tag', …)`.

#[path = "common/code_seed.rs"]
mod code_seed;

use comemory::config::Config;
use comemory::retrieval::graph_route;
use comemory::retrieval::router::{RoutedHit, Source};

/// Default config with the walk depth pinned to `hops`.
fn cfg_hops(hops: u32) -> Config {
    let mut cfg = Config::defaults();
    cfg.retrieval.graph_hops = hops;
    cfg
}

/// Insert a live `memories` row: kind `note`, no repo.
fn seed_memory(conn: &rusqlite::Connection, id: &str) {
    seed_memory_scoped(conn, id, "note", None);
}

/// Insert a live `memories` row with an explicit kind and repo.
fn seed_memory_scoped(conn: &rusqlite::Connection, id: &str, kind: &str, repo: Option<&str>) {
    conn.execute(
        "INSERT INTO memories(id,slug,kind,repo,content_hash,body,created_at,updated_at,md_path) \
         VALUES(?1,'s',?2,?3,'h','body','t','t','p.md')",
        rusqlite::params![id, kind, repo],
    )
    .expect("seed memory");
}

/// Insert one directed edge in the production `(kind, id)` addressing.
fn edge(conn: &rusqlite::Connection, src: (&str, &str), dst: (&str, &str), rel: &str) {
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
         VALUES(?1,?2,?3,?4,?5,'t')",
        rusqlite::params![src.0, src.1, dst.0, dst.1, rel],
    )
    .expect("seed edge");
}

/// Expand from `seeds` with no repo/kind filter and a pool of 50.
fn expand(conn: &rusqlite::Connection, cfg: &Config, seeds: &[&str]) -> Vec<RoutedHit> {
    let ids: Vec<String> = seeds.iter().map(|s| (*s).to_string()).collect();
    graph_route::expand_memory_seeds(conn, cfg, &ids, None, None, 50).expect("expand")
}

/// Result ids in returned order.
fn ids(hits: &[RoutedHit]) -> Vec<&str> {
    hits.iter().map(|h| h.memory_id.as_str()).collect()
}

#[test]
fn one_hop_memory_relations_expand_in_both_directions() {
    let (_d, conn) = code_seed::open_db();
    for id in ["aaaa0001", "bbbb0002", "cccc0003", "dddd0004"] {
        seed_memory(&conn, id);
    }
    edge(
        &conn,
        ("memory", "aaaa0001"),
        ("memory", "bbbb0002"),
        "derived_from",
    );
    edge(
        &conn,
        ("memory", "aaaa0001"),
        ("memory", "cccc0003"),
        "supersedes",
    );
    // Written the other way round: the walk is undirected, so an incoming
    // edge must expand exactly like an outgoing one.
    edge(
        &conn,
        ("memory", "dddd0004"),
        ("memory", "aaaa0001"),
        "conflicts_with",
    );

    let hits = expand(&conn, &cfg_hops(1), &["aaaa0001"]);
    assert_eq!(ids(&hits), ["bbbb0002", "cccc0003", "dddd0004"]);
    assert!(
        hits.iter()
            .all(|h| h.source == Source::Graph && h.tier == 0),
        "graph candidates carry Source::Graph at tier 0: {hits:?}"
    );
    assert!(
        hits.iter().all(|h| (h.score - 0.5).abs() < 1e-6),
        "one hop scores 1/(1+1): {hits:?}"
    );
}

#[test]
fn seed_ids_are_never_returned_as_candidates() {
    let (_d, conn) = code_seed::open_db();
    seed_memory(&conn, "aaaa0001");
    seed_memory(&conn, "bbbb0002");
    edge(
        &conn,
        ("memory", "aaaa0001"),
        ("memory", "bbbb0002"),
        "derived_from",
    );

    // Both endpoints seeded: the walk reaches each from the other, but the
    // leg may only contribute candidates the base ranking does not have.
    let hits = expand(&conn, &cfg_hops(2), &["aaaa0001", "bbbb0002"]);
    assert!(hits.is_empty(), "seeds must be excluded: {hits:?}");
}

#[test]
fn shared_symbol_reaches_a_dark_memory_at_two_hops_only() {
    // A →references_symbol→ S ←references_symbol← B, both edges written
    // forward as production writes them: B is reachable only by traversing
    // the second edge in reverse, on hop 2.
    let (_d, conn) = code_seed::open_db();
    seed_memory(&conn, "aaaa0001");
    seed_memory(&conn, "bbbb0002");
    code_seed::seed_symbol(&conn, "demo", "src/lib.rs", "run");
    let symbol = ("symbol", "demo:src/lib.rs:run");
    edge(&conn, ("memory", "aaaa0001"), symbol, "references_symbol");
    edge(&conn, ("memory", "bbbb0002"), symbol, "references_symbol");

    let two = expand(&conn, &cfg_hops(2), &["aaaa0001"]);
    assert_eq!(ids(&two), ["bbbb0002"]);
    assert!(
        (two[0].score - 1.0 / 3.0).abs() < 1e-6,
        "two hops score 1/(1+2): {two:?}"
    );

    let one = expand(&conn, &cfg_hops(1), &["aaaa0001"]);
    assert!(
        one.is_empty(),
        "the symbol is one hop away, the memory behind it is two: {one:?}"
    );
}

#[test]
fn hub_relations_never_link_two_memories() {
    // in_repo / authored_by / tagged put every memory in a repo, by an
    // author, or under a tag one hop from the same node — traversing them
    // would connect the whole corpus at depth 2.
    let (_d, conn) = code_seed::open_db();
    seed_memory(&conn, "aaaa0001");
    seed_memory(&conn, "bbbb0002");
    for (kind, id, rel) in [
        ("repo", "demo", "in_repo"),
        ("author", "falconiere", "authored_by"),
        ("tag", "database", "tagged"),
    ] {
        edge(&conn, ("memory", "aaaa0001"), (kind, id), rel);
        edge(&conn, ("memory", "bbbb0002"), (kind, id), rel);
    }

    let hits = expand(&conn, &cfg_hops(4), &["aaaa0001"]);
    assert!(
        hits.is_empty(),
        "hub relations must not be traversed at any depth: {hits:?}"
    );
}

#[test]
fn repo_and_kind_filters_drop_non_matching_neighbors() {
    let (_d, conn) = code_seed::open_db();
    seed_memory_scoped(&conn, "aaaa0001", "note", Some("alpha"));
    seed_memory_scoped(&conn, "bbbb0002", "decision", Some("alpha"));
    seed_memory_scoped(&conn, "cccc0003", "decision", Some("beta"));
    seed_memory_scoped(&conn, "dddd0004", "bug", Some("alpha"));
    for dst in ["bbbb0002", "cccc0003", "dddd0004"] {
        edge(&conn, ("memory", "aaaa0001"), ("memory", dst), "relates_to");
    }

    let cfg = cfg_hops(1);
    let filtered = |repo, kind| -> Vec<String> {
        graph_route::expand_memory_seeds(&conn, &cfg, &["aaaa0001".to_string()], repo, kind, 50)
            .expect("expand")
            .into_iter()
            .map(|h| h.memory_id)
            .collect()
    };

    assert_eq!(filtered(None, None).len(), 3, "unfiltered sees all three");
    assert_eq!(filtered(Some("alpha"), None), ["bbbb0002", "dddd0004"]);
    assert_eq!(filtered(None, Some("decision")), ["bbbb0002", "cccc0003"]);
    assert_eq!(filtered(Some("alpha"), Some("decision")), ["bbbb0002"]);
}

#[test]
fn a_neighbor_on_two_paths_appears_once_at_its_shortest_hop() {
    // B sits one hop from A directly and two hops away via C.
    let (_d, conn) = code_seed::open_db();
    for id in ["aaaa0001", "bbbb0002", "cccc0003"] {
        seed_memory(&conn, id);
    }
    edge(
        &conn,
        ("memory", "aaaa0001"),
        ("memory", "bbbb0002"),
        "derived_from",
    );
    edge(
        &conn,
        ("memory", "aaaa0001"),
        ("memory", "cccc0003"),
        "relates_to",
    );
    edge(
        &conn,
        ("memory", "cccc0003"),
        ("memory", "bbbb0002"),
        "relates_to",
    );

    let hits = expand(&conn, &cfg_hops(2), &["aaaa0001"]);
    assert_eq!(ids(&hits), ["bbbb0002", "cccc0003"], "one row per memory");
    assert!(
        (hits[0].score - 0.5).abs() < 1e-6,
        "the shorter path wins: B ranks at 1 hop, not 2: {hits:?}"
    );
}

#[test]
fn edges_to_deleted_or_missing_memories_are_dropped() {
    let (_d, conn) = code_seed::open_db();
    seed_memory(&conn, "aaaa0001");
    seed_memory(&conn, "bbbb0002");
    conn.execute(
        "UPDATE memories SET deleted_at = '2026-07-01T00:00:00Z' WHERE id = 'bbbb0002'",
        [],
    )
    .expect("soft delete");
    edge(
        &conn,
        ("memory", "aaaa0001"),
        ("memory", "bbbb0002"),
        "derived_from",
    );
    // A target `index-code` never filled in — edges may dangle by design.
    edge(
        &conn,
        ("memory", "aaaa0001"),
        ("memory", "eeee0009"),
        "derived_from",
    );

    let hits = expand(&conn, &cfg_hops(2), &["aaaa0001"]);
    assert!(
        hits.is_empty(),
        "soft-deleted and dangling targets must not surface: {hits:?}"
    );
}

#[test]
fn empty_seeds_and_zero_hops_expand_to_nothing() {
    let (_d, conn) = code_seed::open_db();
    seed_memory(&conn, "aaaa0001");
    seed_memory(&conn, "bbbb0002");
    edge(
        &conn,
        ("memory", "aaaa0001"),
        ("memory", "bbbb0002"),
        "derived_from",
    );

    let no_seeds =
        graph_route::expand_memory_seeds(&conn, &cfg_hops(2), &[], None, None, 50).expect("expand");
    assert!(no_seeds.is_empty(), "no seeds, no walk: {no_seeds:?}");

    let disabled = expand(&conn, &cfg_hops(0), &["aaaa0001"]);
    assert!(
        disabled.is_empty(),
        "hops = 0 disables the leg: {disabled:?}"
    );
}

#[test]
fn ordering_is_hops_then_id_and_stable_across_calls() {
    // Two-hop neighbours deliberately carry ids that sort before the
    // one-hop ones, so only a (hops, id) sort produces the expected order.
    let (_d, conn) = code_seed::open_db();
    for id in ["aaaa0001", "wwww0001", "xxxx0002", "bbbb0003", "cccc0004"] {
        seed_memory(&conn, id);
    }
    for dst in ["xxxx0002", "wwww0001"] {
        edge(&conn, ("memory", "aaaa0001"), ("memory", dst), "relates_to");
    }
    for dst in ["cccc0004", "bbbb0003"] {
        edge(
            &conn,
            ("memory", "wwww0001"),
            ("memory", dst),
            "derived_from",
        );
    }

    let cfg = cfg_hops(2);
    let first = expand(&conn, &cfg, &["aaaa0001"]);
    assert_eq!(
        ids(&first),
        ["wwww0001", "xxxx0002", "bbbb0003", "cccc0004"],
        "one-hop neighbours first, ids ascending within each depth"
    );
    let second = expand(&conn, &cfg, &["aaaa0001"]);
    assert_eq!(ids(&first), ids(&second), "repeated calls must not reorder");
}

#[test]
fn pool_truncates_to_the_nearest_then_smallest_ids() {
    let (_d, conn) = code_seed::open_db();
    seed_memory(&conn, "aaaa0000");
    for id in ["bbbb0001", "cccc0002", "dddd0003", "eeee0004"] {
        seed_memory(&conn, id);
        edge(&conn, ("memory", "aaaa0000"), ("memory", id), "relates_to");
    }

    let seeds = ["aaaa0000".to_string()];
    let hits = graph_route::expand_memory_seeds(&conn, &cfg_hops(1), &seeds, None, None, 2)
        .expect("expand");
    assert_eq!(
        ids(&hits),
        ["bbbb0001", "cccc0002"],
        "pool caps the leg, keeping the lexicographically smallest ids"
    );

    let none = graph_route::expand_memory_seeds(&conn, &cfg_hops(1), &seeds, None, None, 0)
        .expect("expand");
    assert!(none.is_empty(), "a zero pool asks for no candidates");
}
