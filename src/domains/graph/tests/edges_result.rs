#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Behavior tests for [`comemory::domains::graph::edges_result`] — the `--json`
//! envelope both `comemory edges` and `GET /api/v1/edges` serialize.
//!
//! The envelope is driven directly from [`EdgeFtsHit`] values in the exact
//! shapes `store::edge_fts::search_edges` produces (a memory→memory relation
//! rendered as `kind slug`, and a prefix-stripped file endpoint), so the
//! contract is pinned without a database.

use comemory::domains::graph::edges_result;
use comemory::store::edge_fts::EdgeFtsHit;

/// A memory→memory `supersedes` hit, rendered the way `refresh` renders a
/// live memory endpoint: `<kind> <slug>`.
fn supersedes_hit() -> EdgeFtsHit {
    EdgeFtsHit {
        src_kind: "memory".into(),
        src_id: "d3715797".into(),
        src_text: "decision queue-design-v2".into(),
        rel: "supersedes".into(),
        dst_kind: "memory".into(),
        dst_id: "13414461".into(),
        dst_text: "decision queue-design-v1".into(),
        weight: 1,
        score: 1.25,
    }
}

/// A file→file `co_changed` hit carrying a weight above the default, so the
/// row is proven to carry the stored value rather than a constant.
fn co_changed_hit() -> EdgeFtsHit {
    EdgeFtsHit {
        src_kind: "file".into(),
        src_id: "file:demo:src/queue.rs".into(),
        src_text: "demo:src/queue.rs".into(),
        rel: "co_changed".into(),
        dst_kind: "file".into(),
        dst_id: "file:demo:src/worker.rs".into(),
        dst_text: "demo:src/worker.rs".into(),
        weight: 7,
        score: 0.5,
    }
}

#[test]
fn json_envelope_carries_every_documented_row_field() {
    let hits = vec![supersedes_hit()];
    let page = edges_result::envelope(&hits, 12, 0, false);
    let v = serde_json::to_value(&page).expect("serialize envelope");

    let row = &v["items"][0];
    assert_eq!(row["src_kind"], "memory");
    assert_eq!(row["src_id"], "d3715797");
    assert_eq!(row["src_text"], "decision queue-design-v2");
    assert_eq!(row["rel"], "supersedes");
    assert_eq!(row["dst_kind"], "memory");
    assert_eq!(row["dst_id"], "13414461");
    assert_eq!(row["dst_text"], "decision queue-design-v1");
    assert_eq!(row["weight"], 1);
    assert_eq!(row["score"], 1.25);
    // Exactly the nine documented fields — a silent addition would break
    // consumers pinned to this shape.
    assert_eq!(
        row.as_object().expect("row is an object").len(),
        9,
        "unexpected row shape: {row}"
    );
}

#[test]
fn json_envelope_carries_the_shared_page_cursor() {
    let hits = vec![supersedes_hit(), co_changed_hit()];
    let page = edges_result::envelope(&hits, 2, 4, true);
    let v = serde_json::to_value(&page).expect("serialize envelope");

    assert_eq!(v["limit"], 2);
    assert_eq!(v["offset"], 4);
    assert_eq!(v["has_more"], true);
    // `total` is deliberately null: the page is SQL-sliced behind a k+1
    // probe, so the full match count is never taken.
    assert!(v["total"].is_null(), "total must stay uncounted: {v}");
    assert_eq!(v["items"].as_array().expect("items array").len(), 2);
}
