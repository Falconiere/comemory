//! Test mirror for `src/cli/when.rs` — the `--since` / `--until` /
//! `--as-of` value parser.

use comemory::cli::when::{DayEdge, parse_when};
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
