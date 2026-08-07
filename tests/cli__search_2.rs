#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Time-travel tests for `comemory search` — split from `cli__search.rs`.
//!
//! Covers the `--since` / `--until` / `--as-of` flags end-to-end through the
//! real binary: the created-date window, the as-of supersede semantics
//! (across a `comemory rebuild`, which re-stamps every edge), the JSON
//! envelope echo, and the usage errors.

#[path = "common/time_travel.rs"]
mod corpus;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::tempdir;

use corpus::{NEWER_CREATED, OLDER_CREATED, QUERY};

/// Hit ids in ranked order.
fn ids(envelope: &Value) -> Vec<String> {
    envelope["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["memory_id"].as_str().expect("memory_id").to_string())
        .collect()
}

/// The hit for `id`, or `None` when this run did not surface it.
fn hit<'a>(envelope: &'a Value, id: &str) -> Option<&'a Value> {
    envelope["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .find(|h| h["memory_id"].as_str() == Some(id))
}

/// Run `comemory search` over an empty data dir and return the exit code
/// plus stderr — enough for every validation case, none of which reaches
/// the corpus.
fn reject(args: &[&str]) -> (Option<i32>, String) {
    let home = tempdir().expect("tempdir");
    let assertion = Command::cargo_bin("comemory")
        .expect("cargo_bin comemory")
        .env("COMEMORY_DATA_DIR", home.path())
        .arg("search")
        .arg(QUERY)
        .args(args)
        .assert()
        .failure();
    let out = assertion.get_output();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn until_and_since_bound_the_created_window() {
    // AC-1. The older memory is created 2026-03-10; a bare date expands to
    // the *inclusive* edge of that UTC day, so the same date both includes
    // it as an upper bound and includes it as a lower bound.
    let c = corpus::seed();

    let before = c.run("search", &["--until", "2026-03-01"]);
    assert!(
        ids(&before).is_empty(),
        "a cutoff before the whole corpus must match nothing: {before}"
    );

    let on_the_day = c.run("search", &["--until", OLDER_CREATED]);
    assert_eq!(
        ids(&on_the_day),
        vec![c.older.clone()],
        "--until <the memory's own date> includes it (end of day) and still \
         excludes the later one: {on_the_day}"
    );

    let day_after = c.run("search", &["--since", "2026-03-11"]);
    assert_eq!(
        ids(&day_after),
        vec![c.newer.clone()],
        "--since the day after excludes the older memory: {day_after}"
    );

    let same_day = c.run("search", &["--since", OLDER_CREATED]);
    assert_eq!(
        same_day.get("hits").and_then(Value::as_array).map(Vec::len),
        Some(2),
        "--since <the memory's own date> includes it (start of day): {same_day}"
    );
}

#[test]
fn as_of_hides_the_later_superseder_and_unwinds_its_penalty() {
    // AC-7. The corpus is rebuilt from markdown, so the supersede edge
    // carries a fresh `edges.created_at` — this passes only because the
    // penalty scoping keys on the superseder *memory's* created date.
    let c = corpus::seed();

    let now = c.run("search", &[]);
    let penalized = hit(&now, &c.older).expect("older memory present by default");
    assert_eq!(
        penalized["score_parts"]["supersede"].as_f64(),
        Some(0.2),
        "by default the older memory carries the supersede penalty: {now}"
    );
    assert_eq!(
        penalized["superseded_by"].as_str(),
        Some(c.newer.as_str()),
        "and names its superseder: {now}"
    );

    let back_then = c.run("search", &["--as-of", "2026-04-01"]);
    assert_eq!(
        ids(&back_then),
        vec![c.older.clone()],
        "the superseder did not exist yet, so it must be absent: {back_then}"
    );
    let unpenalized = hit(&back_then, &c.older).expect("older memory present under --as-of");
    assert_eq!(
        unpenalized["score_parts"]["supersede"].as_f64(),
        Some(1.0),
        "nothing superseded it yet, so the penalty must be unwound: {back_then}"
    );
    assert!(
        unpenalized.get("superseded_by").is_none(),
        "and no superseder may be reported: {back_then}"
    );
}

#[test]
fn until_filters_candidates_but_keeps_the_present_day_penalty() {
    // AC-3. The distinction that justifies two flags: --until narrows the
    // corpus, --as-of also rewinds the supersede state.
    let c = corpus::seed();

    let envelope = c.run("search", &["--until", "2026-04-01"]);
    assert_eq!(
        ids(&envelope),
        vec![c.older.clone()],
        "the later memory is out of the window: {envelope}"
    );
    let still_penalized = hit(&envelope, &c.older).expect("older memory present");
    assert_eq!(
        still_penalized["score_parts"]["supersede"].as_f64(),
        Some(0.2),
        "--until must not touch the supersede penalty: {envelope}"
    );
}

#[test]
fn envelope_echoes_only_the_flags_that_were_set() {
    // AC-11. Unset flags leave the envelope byte-identical to the
    // pre-time-travel contract, and `until` / `as_of` stay distinguishable
    // even though they share one cutoff internally.
    let c = corpus::seed();

    let plain = c.run("search", &[]);
    for key in ["since", "until", "as_of"] {
        assert!(
            plain.get(key).is_none(),
            "an unscoped run must not echo `{key}`: {plain}"
        );
    }

    let windowed = c.run(
        "search",
        &["--since", OLDER_CREATED, "--until", NEWER_CREATED],
    );
    assert_eq!(
        windowed["since"].as_str(),
        Some("2026-03-10T00:00:00.000000000Z"),
        "--since echoes the normalized start-of-day bound: {windowed}"
    );
    assert_eq!(
        windowed["until"].as_str(),
        Some("2026-06-01T23:59:59.999999999Z"),
        "--until echoes the normalized end-of-day bound: {windowed}"
    );
    assert!(
        windowed.get("as_of").is_none(),
        "a plain window must not claim as-of semantics: {windowed}"
    );

    let rewound = c.run("search", &["--as-of", "2026-04-01T12:00:00Z"]);
    assert_eq!(
        rewound["as_of"].as_str(),
        Some("2026-04-01T12:00:00.000000000Z"),
        "a full timestamp is echoed as given (normalized): {rewound}"
    );
    assert!(
        rewound.get("until").is_none(),
        "an as-of cutoff must not also surface as `until`: {rewound}"
    );
}

#[test]
fn unparsable_time_values_are_usage_errors_naming_the_flag() {
    // AC-9. Every flag reports itself plus both accepted grammars.
    for (flag, value) in [
        ("--as-of", "yesterday"),
        ("--since", "03/10/2026"),
        ("--until", "2026-13-40"),
    ] {
        let (code, stderr) = reject(&[flag, value]);
        assert_eq!(
            code,
            Some(64),
            "{flag} {value} must exit EX_USAGE: {stderr}"
        );
        assert!(
            stderr.contains(flag) && stderr.contains(value),
            "{flag} {value} must be named in the error: {stderr}"
        );
        assert!(
            stderr.contains("RFC3339") && stderr.contains("YYYY-MM-DD"),
            "{flag} {value} must list both accepted formats: {stderr}"
        );
    }
}

#[test]
fn since_later_than_the_cutoff_is_a_usage_error() {
    // AC-9. An inverted window is a typo, not a query that matches nothing.
    let (code, stderr) = reject(&["--since", "2026-06-02", "--until", "2026-06-01"]);
    assert_eq!(code, Some(64), "an inverted window must exit EX_USAGE");
    assert!(
        stderr.contains("--since") && stderr.contains("--until"),
        "the error must name both bounds: {stderr}"
    );

    let (as_of_code, as_of_stderr) = reject(&["--since", "2026-06-02", "--as-of", "2026-06-01"]);
    assert_eq!(as_of_code, Some(64), "same check against the as-of cutoff");
    assert!(
        as_of_stderr.contains("--since") && as_of_stderr.contains("--as-of"),
        "the error must name the cutoff flag that was actually used: {as_of_stderr}"
    );
}

#[test]
fn as_of_and_until_together_are_rejected() {
    // AC-9. clap owns this one — `--as-of` subsumes `--until`.
    let (code, stderr) = reject(&["--as-of", "2026-06-01", "--until", "2026-06-01"]);
    assert!(
        code.is_some_and(|c| c != 0),
        "the conflict must fail the run, got exit {code:?}: {stderr}"
    );
    assert!(
        stderr.contains("--as-of") && stderr.contains("--until"),
        "the conflict error must name both flags: {stderr}"
    );
}
