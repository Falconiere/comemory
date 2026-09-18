#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for the `q-<yyyymmdd>-<8hex>` query-id contract, extracted from
//! `domains::learning::feedback_tracking` by #166. The writer and the checker must agree for every
//! id the generator emits, and the two auto-reinforcement sentinel ids must
//! stay deliberately invalid so `eval::golden::harvest` can never mint a
//! golden pair from them.

use comemory::utilities::query_id::{generate_query_id, is_valid_query_id};
use comemory::utilities::telemetry::{COACTIVATION_QUERY_ID, SEARCH_EDIT_QUERY_ID};

#[test]
fn valid_query_id_shape_is_accepted() {
    assert!(is_valid_query_id("q-20260610-a1b2c3d4"));
}

#[test]
fn malformed_query_ids_are_rejected() {
    let bad = [
        "",                     // empty
        "q-2026061-a1b2c3d4",   // 7-digit date
        "q-20260610-A1B2C3D4",  // uppercase hex
        "q-20260610-a1b2c3",    // short hex
        "x-20260610-a1b2c3d4",  // wrong prefix
        "q-20260610-a1b2c3d4x", // trailing garbage
    ];
    for s in bad {
        assert!(!is_valid_query_id(s), "must reject {s:?}");
    }
}

#[test]
fn generated_query_id_always_validates() {
    // Round-trip of the writer/checker contract: every id the generator
    // emits must pass the validator, for both a fixed and a live clock.
    let fixed = time::macros::datetime!(2026-06-10 12:34:56.789 UTC);
    for query in ["", "sqlite busy", "Café VecDimMismatch \"quoted\""] {
        let id = generate_query_id(query, fixed);
        assert!(is_valid_query_id(&id), "generated id must validate: {id}");
        assert!(id.starts_with("q-20260610-"), "day-sortable prefix: {id}");
    }
    let live = generate_query_id("any query", time::OffsetDateTime::now_utc());
    assert!(
        is_valid_query_id(&live),
        "live-clock id must validate: {live}"
    );
}

#[test]
fn the_reinforcement_sentinels_are_not_valid_query_ids() {
    assert!(!is_valid_query_id(COACTIVATION_QUERY_ID));
    assert!(!is_valid_query_id(SEARCH_EDIT_QUERY_ID));
}
