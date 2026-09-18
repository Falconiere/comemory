#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/domains/retrieval/scope.rs` — the [`TimeScope`] both
//! `search` and `context` build from the `--since` / `--until` / `--as-of`
//! flags.
//!
//! The window is the retrieval capability's own concept, so #178 moved
//! `scope_from_flags` here out of `utilities::when`, which now only parses an
//! instant. Parser coverage stays in `tests/utilities__when.rs`.

use comemory::domains::retrieval::scope::{TimeScope, scope_from_flags};

#[test]
fn no_flags_build_the_unbounded_scope() {
    let scope = scope_from_flags(None, None, None).expect("no flags");
    assert_eq!(scope, TimeScope::none(), "no flags must not scope anything");
    assert!(scope.is_unbounded());
}

#[test]
fn bounds_are_normalized_to_the_writer_timestamp_format() {
    // The store compares `created_at` through SQLite `datetime()`, but the
    // bounds still travel in the one format every writer emits, so a scoped
    // run compares like against like.
    let scope = scope_from_flags(Some("2026-03-10"), Some("2026-06-01"), None).expect("window");
    assert_eq!(
        scope.since.as_deref(),
        Some("2026-03-10T00:00:00.000000000Z")
    );
    assert_eq!(
        scope.cutoff.as_deref(),
        Some("2026-06-01T23:59:59.999999999Z")
    );
    assert!(!scope.as_of, "--until must not claim as-of semantics");
    assert_eq!(
        scope.as_of_cutoff(),
        None,
        "--until leaves the penalty alone"
    );
}

#[test]
fn as_of_fills_the_same_cutoff_but_scopes_the_penalty() {
    let scope = scope_from_flags(None, None, Some("2026-04-01")).expect("as-of");
    assert_eq!(
        scope.cutoff.as_deref(),
        Some("2026-04-01T23:59:59.999999999Z"),
        "--as-of is an --until plus supersede scoping"
    );
    assert!(scope.as_of);
    assert_eq!(
        scope.as_of_cutoff(),
        scope.cutoff.as_deref(),
        "the penalty is scoped to the same instant"
    );
}

#[test]
fn an_inverted_window_is_rejected_naming_both_flags() {
    for (until, as_of, cutoff_flag) in [
        (Some("2026-06-01"), None, "--until"),
        (None, Some("2026-06-01"), "--as-of"),
    ] {
        let err = scope_from_flags(Some("2026-06-02"), until, as_of)
            .expect_err("since after the cutoff must be rejected");
        assert!(
            matches!(err, comemory::errors::Error::Usage(_)),
            "an inverted window is a usage error, got: {err:?}"
        );
        let msg = err.to_string();
        // Both flags must appear as FLAG LABELS, not merely somewhere in the
        // text: the values are echoed too, so a bare `contains` would also pass
        // for a message that only quoted `--until` as the offending value.
        assert!(
            msg.contains("--since must not be later than"),
            "the error must lead with the --since bound it rejected, got: {msg}"
        );
        assert!(
            msg.contains(&format!("later than {cutoff_flag}")),
            "and must name {cutoff_flag} as the opposing bound, got: {msg}"
        );
    }
}

#[test]
fn a_bad_value_is_attributed_to_the_flag_that_carried_it() {
    for (since, until, as_of, flag) in [
        (Some("nope"), None, None, "--since"),
        (None, Some("nope"), None, "--until"),
        (None, None, Some("nope"), "--as-of"),
    ] {
        let err = scope_from_flags(since, until, as_of).expect_err("garbage must be rejected");
        let msg = err.to_string();
        assert!(
            msg.starts_with(flag),
            "the message must lead with the offending flag, got: {msg}"
        );
        assert!(
            msg.contains("RFC3339") && msg.contains("YYYY-MM-DD"),
            "and still list both accepted formats, got: {msg}"
        );
    }
}

#[test]
fn equal_bounds_are_a_legal_single_instant_window() {
    // Same bare date on both flags is the common "just that day" query:
    // start-of-day <= end-of-day, so it must not trip the ordering check.
    let scope = scope_from_flags(Some("2026-03-10"), Some("2026-03-10"), None)
        .expect("a one-day window is legal");
    assert!(
        scope.since < scope.cutoff,
        "start of day precedes end of day"
    );
}
