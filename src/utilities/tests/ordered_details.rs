#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/utilities/ordered_details.rs`: pairs serialize in
//! push order even where that order is not alphabetical.

use comemory::utilities::ordered_details::OrderedDetails;
use serde_json::Value;

/// `coded` puts `code` first and keeps the rest in push order — the exact
/// bytes a sorted `serde_json::Map` would reorder.
#[test]
fn coded_serializes_code_first_then_push_order() {
    let details = OrderedDetails::coded(
        "proposal_stale",
        vec![
            ("basePlanVersion", Value::from(3)),
            ("currentPlanVersion", Value::from(5)),
        ],
    );
    assert_eq!(
        serde_json::to_string(&details).unwrap(),
        r#"{"code":"proposal_stale","basePlanVersion":3,"currentPlanVersion":5}"#
    );
}

/// `from_pairs` adds no `code` member, and an empty list is `{}`.
#[test]
fn from_pairs_adds_nothing() {
    let details = OrderedDetails::from_pairs(vec![
        ("reason", Value::from("cap_exceeded")),
        ("field", Value::from("operations")),
    ]);
    assert_eq!(
        serde_json::to_string(&details).unwrap(),
        r#"{"reason":"cap_exceeded","field":"operations"}"#
    );
    assert_eq!(
        serde_json::to_string(&OrderedDetails::default()).unwrap(),
        "{}"
    );
}
