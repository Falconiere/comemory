#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/timestamp.rs`.

use comemory::domains::projects::timestamp::{iso, now_ms, parse_target_date};
use comemory::utilities::error_code::{Class, classify};

#[test]
fn iso_renders_like_to_iso_string() {
    // `new Date(1727481600123).toISOString()`
    assert_eq!(iso(1_727_481_600_123).unwrap(), "2024-09-28T00:00:00.123Z");
    assert_eq!(iso(0).unwrap(), "1970-01-01T00:00:00.000Z");
    assert!(now_ms() > 1_727_481_600_000);
}

#[test]
fn target_dates_are_utc() {
    assert_eq!(parse_target_date("2024-09-28").unwrap(), 1_727_481_600_000);
    assert_eq!(
        parse_target_date("2024-09-28T02:00:00+02:00").unwrap(),
        1_727_481_600_000
    );
    for bad in ["2024-13-01", "tomorrow", "2024-09-28T10:00", ""] {
        let e = parse_target_date(bad).unwrap_err();
        assert_eq!(
            classify(&e),
            ("invalid_request", Class::BadRequest),
            "{bad}"
        );
    }
}
