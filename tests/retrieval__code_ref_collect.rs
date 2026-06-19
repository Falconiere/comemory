//! Behavioral tests for [`comemory::retrieval::code_ref_collect`].
//!
//! The collect helpers are `pub(crate)`, so they are exercised through their
//! sole caller, [`comemory::retrieval::bundle::assemble`]. A `references_file`
//! edge must now surface as a file ref (symbol/line/signature null); a
//! `references_symbol` edge must resolve its `code_symbols` row (snippet, line,
//! signature); and a pinned `code_ref` anchor must flow onto the emitted ref.
//! Real migrated DB, no mocks.

#[path = "common/code_seed.rs"]
mod code_seed;

use comemory::config::Config;
use comemory::retrieval::bundle;
use comemory::retrieval::code_rerank::WorkingSet;

/// Seed a minimal live `memories` row.
fn seed_memory(conn: &rusqlite::Connection, id: &str) {
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES(?1,'s','note','h','body','t','t','p.md')",
        [id],
    )
    .expect("seed memory");
}

/// Seed a reference edge of `rel` from `memory_id` to `dst`.
fn seed_edge(conn: &rusqlite::Connection, memory_id: &str, dst_kind: &str, rel: &str, dst: &str) {
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
         VALUES('memory',?1,?2,?3,?4,'t')",
        rusqlite::params![memory_id, dst_kind, dst, rel],
    )
    .expect("seed edge");
}

/// Seed a `code_ref` anchor row carrying a pinned blob.
fn seed_anchor(conn: &rusqlite::Connection, memory_id: &str, rel: &str, dst: &str, blob: &str) {
    conn.execute(
        "INSERT INTO code_ref(memory_id, rel, dst_id, pinned_blob, created_at) \
         VALUES(?1, ?2, ?3, ?4, 't')",
        rusqlite::params![memory_id, rel, dst, blob],
    )
    .expect("seed anchor");
}

fn assemble<'a>(conn: &rusqlite::Connection, ids: &[String]) -> bundle::Bundle<'a> {
    bundle::assemble(conn, &Config::defaults(), "q", ids, &WorkingSet::default()).expect("assemble")
}

#[test]
fn file_reference_edge_surfaces_as_file_ref() {
    let (_d, conn) = code_seed::open_db();
    seed_memory(&conn, "m1");
    seed_edge(&conn, "m1", "file", "references_file", "demo:src/a.rs");

    let b = assemble(&conn, &["m1".to_string()]);
    let f = b
        .code_refs
        .iter()
        .find(|c| c.id == "demo:src/a.rs")
        .expect("file ref must surface");
    assert_eq!(f.symbol, "", "file ref has no symbol");
    assert_eq!(f.line, None, "file ref has no line");
    assert_eq!(f.signature, None, "file ref has no signature");
    // No repo on disk in this fixture, and the edge is unpinned -> unpinned.
    assert_eq!(f.status, "unpinned");
}

#[test]
fn symbol_reference_edge_resolves_snippet_and_line() {
    let (_d, conn) = code_seed::open_db();
    seed_memory(&conn, "m1");
    code_seed::seed_symbol(&conn, "demo", "z.rs", "z_run");
    seed_edge(
        &conn,
        "m1",
        "symbol",
        "references_symbol",
        "demo:z.rs:z_run",
    );

    let b = assemble(&conn, &["m1".to_string()]);
    let r = b
        .code_refs
        .iter()
        .find(|c| c.id == "demo:z.rs:z_run")
        .expect("symbol ref must surface");
    assert_eq!(r.symbol, "z_run");
    assert_eq!(r.line, Some(1), "resolved symbol carries its line_start");
    assert_eq!(
        r.signature.as_deref(),
        Some("fn body() {}"),
        "signature is the first snippet line"
    );
}

#[test]
fn pinned_anchor_flows_onto_emitted_ref_status() {
    let (_d, conn) = code_seed::open_db();
    seed_memory(&conn, "m1");
    seed_edge(&conn, "m1", "file", "references_file", "demo:src/a.rs");
    // A pinned anchor with no repo on disk -> Unknown (pinned but unverifiable),
    // proving the anchor lookup is wired (an unpinned edge would be Unpinned).
    seed_anchor(&conn, "m1", "references_file", "demo:src/a.rs", "deadbeef");

    let b = assemble(&conn, &["m1".to_string()]);
    let f = b
        .code_refs
        .iter()
        .find(|c| c.id == "demo:src/a.rs")
        .expect("file ref must surface");
    assert_eq!(
        f.status, "unknown",
        "a pinned anchor with no on-disk repo classifies Unknown, not Unpinned"
    );
}
