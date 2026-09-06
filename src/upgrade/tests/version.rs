#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `Version` parsing, rendering, and ordering — the comparison `comemory
//! upgrade` bases its "newer?" decision on.

use comemory::errors::Error;
use comemory::upgrade::version::Version;

fn v(s: &str) -> Version {
    Version::parse(s).unwrap_or_else(|e| panic!("parse {s}: {e}"))
}

#[test]
fn parses_with_and_without_the_tag_prefix() {
    let plain = v("0.19.0");
    let tagged = v("v0.19.0");
    assert_eq!(plain, tagged);
    assert_eq!(plain.major, 0);
    assert_eq!(plain.minor, 19);
    assert_eq!(plain.patch, 0);
    assert_eq!(plain.pre, None);
    assert_eq!(plain.to_string(), "0.19.0");
    assert_eq!(plain.tag(), "v0.19.0");
}

#[test]
fn keeps_a_pre_release_suffix() {
    let rc = v("1.2.3-rc.1");
    assert_eq!(rc.pre.as_deref(), Some("rc.1"));
    assert_eq!(rc.to_string(), "1.2.3-rc.1");
    assert_eq!(rc.tag(), "v1.2.3-rc.1");
}

#[test]
fn orders_numerically_not_lexically() {
    assert!(v("0.10.0") > v("0.9.9"));
    assert!(v("1.0.0") > v("0.99.99"));
    assert!(v("0.18.2") < v("0.18.10"));
    assert_eq!(v("0.18.2").cmp(&v("v0.18.2")), std::cmp::Ordering::Equal);
}

#[test]
fn a_pre_release_sorts_below_its_final() {
    assert!(v("0.19.0-rc.1") < v("0.19.0"));
    assert!(v("0.19.0-rc.1") > v("0.18.9"));
    assert!(v("0.19.0-rc.1") < v("0.19.0-rc.2"));
}

#[test]
fn pre_release_segments_compare_numerically_like_semver() {
    assert!(v("1.0.0-rc.10") > v("1.0.0-rc.9"), "numeric, not lexical");
    assert!(
        v("1.0.0-rc") < v("1.0.0-rc.1"),
        "a prefix sorts below its extension"
    );
    assert!(v("1.0.0-alpha") < v("1.0.0-beta"));
    assert!(
        v("1.0.0-1") < v("1.0.0-alpha"),
        "numeric segments rank below alphanumeric"
    );
    assert!(v("1.0.0-alpha.1") < v("1.0.0-alpha.beta"));
    assert_eq!(
        v("1.0.0-rc.1").cmp(&v("1.0.0-rc.1")),
        std::cmp::Ordering::Equal
    );
}

#[test]
fn rejects_malformed_input_as_a_usage_error() {
    for bad in ["", "1.2", "1.2.3.4", "a.b.c", "1.x.0", "1.2.3-", "latest"] {
        match Version::parse(bad) {
            Err(Error::Usage(msg)) => assert!(
                msg.contains(&format!("`{bad}`")),
                "message for {bad:?} should quote the input: {msg}"
            ),
            other => panic!("{bad:?} should be a usage error, got {other:?}"),
        }
    }
}

#[test]
fn the_running_build_parses() {
    let current = comemory::upgrade::current_version().expect("CARGO_PKG_VERSION parses");
    assert_eq!(current.to_string(), env!("CARGO_PKG_VERSION"));
}
