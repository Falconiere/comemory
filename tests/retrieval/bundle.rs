//! Tests for [`comemory::retrieval::bundle::assemble`].
//!
//! Covers: empty bundle, single memory pull, multi-rel depth-2 edge walk
//! (references_symbol, relates_to, supersedes), and prior-ranked code refs:
//! resolved refs sorted by the four-prior product (with serialized
//! `rank_parts`), unresolved refs trailing without them.

use comemory::config::Config;
use comemory::retrieval::bundle::{self, Bundle};
use comemory::retrieval::code_rerank::WorkingSet;

use super::code_seed;

/// Assemble with the default config and an empty working set — the
/// fixed-arg shape every test here wants.
fn assemble(conn: &rusqlite::Connection, query: &'static str, ids: &[String]) -> Bundle<'static> {
    bundle::assemble(
        conn,
        &Config::defaults(),
        query,
        ids,
        &WorkingSet::default(),
    )
    .expect("assemble")
}

/// Insert a minimal live `memories` row with the given id.
fn seed_memory(conn: &rusqlite::Connection, id: &str) {
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES(?1,'s','note','h','body','t','t','p.md')",
        [id],
    )
    .expect("seed memory");
}

/// Insert a `references_symbol` edge from `memory_id` to the
/// `<repo>:<path>:<symbol>` destination `dst`.
fn seed_symbol_edge(conn: &rusqlite::Connection, memory_id: &str, dst: &str) {
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
         VALUES('memory',?1,'symbol',?2,'references_symbol','t')",
        rusqlite::params![memory_id, dst],
    )
    .expect("seed references_symbol edge");
}

#[test]
fn assemble_returns_empty_bundle_when_no_ids() {
    let (_d, conn) = code_seed::open_db();

    let b = assemble(&conn, "advisory lock", &[]);
    assert_eq!(b.query, "advisory lock");
    assert!(b.memories.is_empty());
    assert!(b.code_refs.is_empty());
    assert!(b.relations.is_empty());
}

#[test]
fn assemble_pulls_memory_rows_by_id() {
    let (_d, conn) = code_seed::open_db();

    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES('m1','s','decision','h','Use Postgres for analytics','t','t','x.md')",
        [],
    )
    .expect("seed");

    let b = assemble(&conn, "postgres", &["m1".to_string()]);
    assert_eq!(b.memories.len(), 1);
    assert_eq!(b.memories[0].id, "m1");
    assert_eq!(b.memories[0].kind, "decision");
    assert_eq!(b.memories[0].body, "Use Postgres for analytics");

    let v: serde_json::Value = serde_json::to_value(&b).expect("json");
    assert_eq!(v["query"], "postgres");
    assert_eq!(v["memories"][0]["id"], "m1");
}

#[test]
fn assemble_walks_supersedes_chain_to_depth_2() {
    // m1 supersedes m2, m2 supersedes m3 — depth-2 walk from m1 must
    // surface both edges.
    let (_d, conn) = code_seed::open_db();

    for id in ["m1", "m2", "m3"] {
        seed_memory(&conn, id);
    }
    for (src, dst) in [("m1", "m2"), ("m2", "m3")] {
        conn.execute(
            "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
             VALUES('memory',?1,'memory',?2,'supersedes','t')",
            rusqlite::params![src, dst],
        )
        .expect("edge");
    }

    let b = assemble(&conn, "q", &["m1".to_string()]);
    // Both hops must appear in relations.
    let rels: Vec<&str> = b.relations.iter().map(|r| r.rel.as_str()).collect();
    let supersedes_count = rels.iter().filter(|&&r| r == "supersedes").count();
    assert_eq!(
        supersedes_count,
        2,
        "expected 2 supersedes edges, got relations: {b:?}",
        b = b
            .relations
            .iter()
            .map(|r| format!("{}->{}", r.from, r.to))
            .collect::<Vec<_>>()
    );
}

#[test]
fn assemble_walks_relates_to_edges() {
    let (_d, conn) = code_seed::open_db();

    for id in ["m1", "m2"] {
        seed_memory(&conn, id);
    }
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
         VALUES('memory','m1','memory','m2','relates_to','t')",
        [],
    )
    .expect("edge");

    let b = assemble(&conn, "q", &["m1".to_string()]);
    assert!(
        b.relations.iter().any(|r| r.rel == "relates_to"),
        "relates_to edge missing from bundle relations: {rels:?}",
        rels = b.relations.iter().map(|r| &r.rel).collect::<Vec<_>>()
    );
}

#[test]
fn code_refs_rank_by_prior_product() {
    let (_d, conn) = code_seed::open_db();
    seed_memory(&conn, "m1");
    // `aaa_run` wins every lexical tie-break (its path and symbol both sort
    // first), so only the rank_score + recent-access priors on `zzz_run`
    // can put it on top.
    let cold = code_seed::seed_symbol(&conn, "demo", "aaa.rs", "aaa_run");
    let hot = code_seed::seed_symbol(&conn, "demo", "zzz.rs", "zzz_run");
    conn.execute(
        "UPDATE code_symbols SET rank_score = 0.9, access_count = 40 WHERE id = ?1",
        [hot],
    )
    .expect("boost hot");
    conn.execute(
        "UPDATE code_symbols SET rank_score = 0.1 WHERE id = ?1",
        [cold],
    )
    .expect("set cold");
    seed_symbol_edge(&conn, "m1", "demo:aaa.rs:aaa_run");
    seed_symbol_edge(&conn, "m1", "demo:zzz.rs:zzz_run");

    let b = assemble(&conn, "q", &["m1".to_string()]);
    assert_eq!(b.code_refs.len(), 2);
    assert_eq!(
        b.code_refs[0].symbol, "zzz_run",
        "prior-boosted ref must sort first"
    );
    let hot_parts = b.code_refs[0]
        .rank_parts
        .as_ref()
        .expect("resolved ref carries rank_parts");
    let cold_parts = b.code_refs[1]
        .rank_parts
        .as_ref()
        .expect("resolved ref carries rank_parts");
    assert!(
        hot_parts.rank > cold_parts.rank,
        "higher rank_score must carry the larger rank prior"
    );
    assert!(
        hot_parts.activation > 1.0,
        "recently accessed symbol must sit above the neutral activation"
    );
    assert!(hot_parts.final_score > cold_parts.final_score);

    // Serialized contract: rank_parts carries every prior plus the product.
    let v: serde_json::Value = serde_json::to_value(&b).expect("json");
    let parts = &v["code_refs"][0]["rank_parts"];
    for key in ["rank", "activation", "affinity", "feedback", "final_score"] {
        assert!(parts[key].is_number(), "rank_parts.{key} missing: {v}");
    }
}

#[test]
fn unresolved_code_refs_sort_after_ranked_without_rank_parts() {
    let (_d, conn) = code_seed::open_db();
    seed_memory(&conn, "m1");
    let _resolved = code_seed::seed_symbol(&conn, "demo", "zzz.rs", "zzz_run");
    seed_symbol_edge(&conn, "m1", "demo:zzz.rs:zzz_run");
    // Dangling refs (memory saved before `index-code` ran): they must trail
    // every ranked ref, ordered by (path, symbol) among themselves.
    seed_symbol_edge(&conn, "m1", "demo:bb.rs:bb_ghost");
    seed_symbol_edge(&conn, "m1", "demo:aa.rs:aa_ghost");

    let b = assemble(&conn, "q", &["m1".to_string()]);
    let order: Vec<&str> = b.code_refs.iter().map(|c| c.symbol.as_str()).collect();
    assert_eq!(
        order,
        ["zzz_run", "aa_ghost", "bb_ghost"],
        "ranked refs first, then unresolved by (path, symbol)"
    );
    assert!(b.code_refs[0].rank_parts.is_some());
    assert!(b.code_refs[1].rank_parts.is_none());
    assert!(b.code_refs[2].rank_parts.is_none());
    assert!(
        b.code_refs[1].snippet.is_empty(),
        "unresolved ref has no snippet to show"
    );

    let v: serde_json::Value = serde_json::to_value(&b).expect("json");
    assert!(v["code_refs"][0].get("rank_parts").is_some());
    assert!(
        v["code_refs"][1].get("rank_parts").is_none(),
        "rank_parts must be skipped when None: {v}"
    );
}
