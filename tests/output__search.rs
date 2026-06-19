//! Mirror tests for `src/output/search.rs`. Pins the `comemory search
//! --json` envelope contract (`score_parts` is a stable, documented
//! surface — M2 tuning reads it) via an insta snapshot, and locks in that
//! `output::search::emit` accepts an empty hit slice without panicking.

use std::collections::HashMap;
use std::path::Path;

use comemory::memory::{Ref, References};
use comemory::output::search::{self, PageMeta};
use comemory::retrieval::rerank::{Reranked, ScoreParts};
use comemory::retrieval::router::Source;
use comemory::store::memory_meta::MemoryMeta;

/// Empty navigation map: hits degrade to empty path/kind/tags. Used by the
/// tests that only exercise the score/source/tier shape.
fn no_meta() -> HashMap<String, MemoryMeta> {
    HashMap::new()
}

/// Data dir for path resolution in the navigation line / `path` field.
fn data_dir() -> &'static Path {
    Path::new("/data")
}

/// Representative pagination cursor for the unpaginated first page: page
/// size 12 (default `top_k`), no offset, single in-window hit.
fn meta() -> PageMeta {
    PageMeta {
        limit: 12,
        offset: 0,
        has_more: false,
        total: Some(1),
    }
}

fn sample_hits() -> Vec<Reranked> {
    vec![Reranked {
        memory_id: "aaaa0001".into(),
        source: Source::Hybrid,
        tier: 1,
        parts: ScoreParts {
            rrf: 0.016,
            activation: 1.0,
            feedback: 1.0,
            quality: 1.0,
            supersede: 1.0,
            final_score: 0.016,
        },
        superseded_by: None,
        body: "Use Postgres for analytics\nmore detail".into(),
        simhash: 0,
    }]
}

/// Deterministic navigation metadata for the `aaaa0001` sample hit so the
/// JSON contract snapshot is stable. The `md_path` is relative so resolving
/// it against `data_dir()` yields a fixed absolute path.
fn sample_meta() -> HashMap<String, MemoryMeta> {
    let mut map = HashMap::new();
    map.insert(
        "aaaa0001".to_string(),
        MemoryMeta {
            md_path: "memories/aaaa0001-use-postgres.md".into(),
            repo: Some("qwick-backend".into()),
            kind: "decision".into(),
            slug: "use-postgres".into(),
            tags: vec!["database".into(), "postgres".into()],
            references: References {
                symbols: vec![Ref::new("qwick-backend:src/db.rs:connect")],
                files: vec![Ref::new("qwick-backend:src/db.rs")],
            },
        },
    );
    map
}

#[test]
fn emit_accepts_empty_hits_in_json_mode() {
    // Smoke test: emitting zero hits in JSON mode must succeed. The full
    // JSON envelope shape is pinned by `search_json_envelope_contract`.
    let hits: Vec<Reranked> = Vec::new();
    search::emit(&hits, None, meta(), true, &no_meta(), data_dir())
        .expect("emit must succeed for empty hits");
}

#[test]
fn tty_footer_omits_feedback_hint_for_empty_hits() {
    let hits: Vec<Reranked> = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    search::write_tty(
        &mut buf,
        &hits,
        Some("q-20260610-a1b2c3d4"),
        &no_meta(),
        data_dir(),
    )
    .expect("write_tty");
    let out = String::from_utf8(buf).expect("utf8");
    assert!(
        out.contains("query: q-20260610-a1b2c3d4"),
        "query id footer must survive empty hits: {out}"
    );
    assert!(
        !out.contains("feedback:"),
        "no feedback hint without hits: {out}"
    );
}

#[test]
fn tty_footer_includes_feedback_hint_with_hits() {
    let hits = sample_hits();
    let mut buf: Vec<u8> = Vec::new();
    search::write_tty(
        &mut buf,
        &hits,
        Some("q-20260610-a1b2c3d4"),
        &sample_meta(),
        data_dir(),
    )
    .expect("write_tty");
    let out = String::from_utf8(buf).expect("utf8");
    assert!(out.contains("feedback:"), "feedback hint expected: {out}");
    // The navigation line carries the resolved absolute markdown path and the
    // body-derived title so the TTY view is navigable.
    assert!(
        out.contains("/data/memories/aaaa0001-use-postgres.md"),
        "nav line must carry the absolute md path: {out}"
    );
    assert!(
        out.contains("Use Postgres for analytics"),
        "nav line must carry the body title: {out}"
    );
}

#[test]
fn search_json_envelope_contract() {
    insta::assert_json_snapshot!(search::envelope(
        &sample_hits(),
        None,
        meta(),
        &sample_meta(),
        data_dir()
    ));
}

#[test]
fn envelope_carries_query_id_when_present() {
    let hits = sample_hits();
    let v = serde_json::to_value(search::envelope(
        &hits,
        Some("q-20260610-a1b2c3d4"),
        meta(),
        &sample_meta(),
        data_dir(),
    ))
    .expect("serialize");
    assert_eq!(
        v.get("query_id").and_then(serde_json::Value::as_str),
        Some("q-20260610-a1b2c3d4")
    );
}

/// Build a `Reranked` fixture with a given `memory_id` and `tier`.
fn make_hit(memory_id: &str, tier: u8, final_score: f64) -> Reranked {
    Reranked {
        memory_id: memory_id.into(),
        source: Source::Lexical,
        tier,
        parts: ScoreParts {
            rrf: final_score as f32,
            activation: 1.0,
            feedback: 1.0,
            quality: 1.0,
            supersede: 1.0,
            final_score,
        },
        superseded_by: None,
        body: String::new(),
        simhash: 0,
    }
}

/// Kill mutant `src/output/search.rs:81`: `hit.tier == TIER_EXPANDED` → `hit.tier != TIER_EXPANDED`.
///
/// Original: tier-4 → `[expanded]`, tier-1 → nothing.
/// Mutant:   tier-4 → nothing,       tier-1 → `[expanded]`.
/// Both assertions must hold to distinguish the two.
#[test]
fn tty_expanded_label_appears_only_for_tier4() {
    // TIER_EXPANDED == 4 must show "[expanded]"; tier 1 must not.
    let hits = vec![
        make_hit("bbbb0004", 4, 0.010),
        make_hit("cccc0001", 1, 0.020),
    ];
    let mut buf: Vec<u8> = Vec::new();
    search::write_tty(&mut buf, &hits, None, &no_meta(), data_dir()).expect("write_tty");
    let out = String::from_utf8(buf).expect("utf8");

    let tier4_line = out
        .lines()
        .find(|l| l.contains("bbbb0004"))
        .expect("tier4 line");
    assert!(
        tier4_line.contains("[expanded]"),
        "tier-4 hit must be labelled [expanded]: {tier4_line:?}"
    );

    let tier1_line = out
        .lines()
        .find(|l| l.contains("cccc0001"))
        .expect("tier1 line");
    assert!(
        !tier1_line.contains("[expanded]"),
        "tier-1 hit must not be labelled [expanded]: {tier1_line:?}"
    );
}

#[test]
fn envelope_omits_query_id_when_absent() {
    let hits = sample_hits();
    let v = serde_json::to_value(search::envelope(
        &hits,
        None,
        meta(),
        &sample_meta(),
        data_dir(),
    ))
    .expect("serialize");
    assert!(
        v.get("query_id").is_none(),
        "query_id must be skipped when None: {v}"
    );
}
