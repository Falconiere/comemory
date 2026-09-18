#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for the shared `<prefix>-<yyyymmdd>-<8hex>` id shape.
//!
//! The point of the shared primitive is that two prefixes of different widths
//! validate the same way, so a new contract cannot silently accept a shape the
//! old one rejects.

use comemory::utilities::dated_id::{dated_id, is_valid_dated_id};

#[test]
fn prefixes_of_different_widths_round_trip() {
    let fixed = time::macros::datetime!(2026-09-18 12:34:56.789 UTC);
    for prefix in ["q", "rr"] {
        let id = dated_id(prefix, "bounded subprocess", fixed);
        assert!(is_valid_dated_id(&id, prefix), "must validate: {id}");
        assert!(id.starts_with(&format!("{prefix}-20260918-")), "{id}");
        assert_eq!(id.len(), prefix.len() + 18);
    }
}

#[test]
fn a_prefix_does_not_validate_under_another() {
    let fixed = time::macros::datetime!(2026-09-18 00:00:00 UTC);
    let query = dated_id("q", "seed", fixed);
    let request = dated_id("rr", "seed", fixed);
    assert!(!is_valid_dated_id(&query, "rr"));
    assert!(!is_valid_dated_id(&request, "q"));
}

#[test]
fn malformed_shapes_are_rejected() {
    let bad = [
        "",
        "rr-2026091-a1b2c3d4",   // 7-digit date
        "rr-20260918-A1B2C3D4",  // uppercase hex
        "rr-20260918-a1b2c3",    // short hex
        "rr-20260918-a1b2c3d4x", // trailing garbage
        "rrx20260918-a1b2c3d4",  // missing separator
    ];
    for s in bad {
        assert!(!is_valid_dated_id(s, "rr"), "must reject {s:?}");
    }
}

#[test]
fn the_same_seed_twice_yields_distinct_ids() {
    // The timestamp is part of the digest input, so an id is not a content
    // hash: two runs of the same query must not collide.
    let a = dated_id(
        "rr",
        "same",
        time::macros::datetime!(2026-09-18 00:00:00 UTC),
    );
    let b = dated_id(
        "rr",
        "same",
        time::macros::datetime!(2026-09-18 00:00:01 UTC),
    );
    assert_ne!(a, b);
}
