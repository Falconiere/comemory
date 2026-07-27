//! Test mirror for `src/cli/when.rs` — the `--since` / `--until` /
//! `--as-of` value parser and the [`TimeScope`] both `search` and
//! `context` build from those flags.

use comemory::cli::when::{DayEdge, parse_when, scope_from_flags};
use comemory::retrieval::scope::TimeScope;
use time::macros::datetime;

#[test]
fn rfc3339_timestamps_pass_through_verbatim() {
    let utc = parse_when("2026-03-10T14:30:00Z", DayEdge::Start).expect("rfc3339 utc");
    assert_eq!(utc, datetime!(2026-03-10 14:30:00 UTC));

    // The day edge is ignored once the value names an instant.
    let same = parse_when("2026-03-10T14:30:00Z", DayEdge::End).expect("rfc3339 end edge");
    assert_eq!(same, utc);
}

#[test]
fn rfc3339_offset_forms_keep_their_offset() {
    let plus = parse_when("2026-03-10T14:30:00+02:00", DayEdge::Start).expect("positive offset");
    assert_eq!(plus, datetime!(2026-03-10 14:30:00 +2));
    assert_eq!(
        plus,
        datetime!(2026-03-10 12:30:00 UTC),
        "a +02:00 wall clock is 12:30 UTC"
    );

    let minus = parse_when("2026-03-10T09:15:00-05:00", DayEdge::End).expect("negative offset");
    assert_eq!(minus, datetime!(2026-03-10 14:15:00 UTC));

    let fractional =
        parse_when("2026-03-10T14:30:00.250Z", DayEdge::Start).expect("fractional seconds");
    assert_eq!(fractional, datetime!(2026-03-10 14:30:00.250 UTC));
}

#[test]
fn bare_date_expands_to_the_requested_day_edge() {
    assert_eq!(
        parse_when("2026-03-10", DayEdge::Start).expect("start edge"),
        datetime!(2026-03-10 00:00:00 UTC)
    );
    assert_eq!(
        parse_when("2026-03-10", DayEdge::End).expect("end edge"),
        datetime!(2026-03-10 23:59:59.999999999 UTC)
    );
}

#[test]
fn surrounding_whitespace_is_tolerated() {
    assert_eq!(
        parse_when("  2026-03-10  ", DayEdge::Start).expect("padded bare date"),
        datetime!(2026-03-10 00:00:00 UTC)
    );
    assert_eq!(
        parse_when(" 2026-03-10T14:30:00Z ", DayEdge::Start).expect("padded timestamp"),
        datetime!(2026-03-10 14:30:00 UTC)
    );
}

#[test]
fn unparsable_values_are_usage_errors_naming_both_formats() {
    for bad in [
        "yesterday",
        "",
        "2026-13-40",
        "03/10/2026",
        "2026-03-10T14:30:00",
        "2026-03-10 extra",
    ] {
        let err = parse_when(bad, DayEdge::Start).expect_err("must reject {bad}");
        assert!(
            matches!(err, comemory::errors::Error::Usage(_)),
            "`{bad}` must map to the usage-class error, got: {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("RFC3339") && msg.contains("YYYY-MM-DD"),
            "`{bad}` error must name both accepted formats, got: {msg}"
        );
        assert!(
            msg.contains(bad),
            "error must echo the offending value, got: {msg}"
        );
    }
}

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
        assert!(
            msg.contains("--since") && msg.contains(cutoff_flag),
            "the error must name both bounds, got: {msg}"
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
