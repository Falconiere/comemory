#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/limits.rs`: every cap accepts its
//! limit and refuses one past it with a `422` naming field, reason and limit.

use comemory::domains::projects::limits::{self, key_prefix, list, page, text};
use comemory::errors::Error;
use comemory::utilities::error_code::{Class, classify};

/// The refusal's class and serialized details.
fn refusal(result: comemory::errors::Result<impl std::fmt::Debug>) -> (Class, String, String) {
    let e = result.expect_err("expected a refusal");
    let Error::Project(project) = &e else {
        panic!("not a project refusal: {e:?}")
    };
    let details = serde_json::to_string(&project.details()).unwrap();
    (classify(&e).1, details, e.to_string())
}

#[test]
fn text_counts_utf16_units_like_the_platform() {
    let emoji = "😀".repeat(60); // 120 UTF-16 units, 60 chars
    text("name", &emoji, 1, limits::NAME_MAX).unwrap();
    let (class, details, message) = refusal(text("name", &format!("{emoji}x"), 1, 120));
    assert_eq!(class, Class::Unprocessable);
    assert_eq!(
        details,
        r#"{"field":"name","reason":"too_long","limit":120}"#
    );
    assert_eq!(message, "name is too_long (limit 120)");
    let (_, details, _) = refusal(text("outcome", "", 1, limits::OUTCOME_MAX));
    assert_eq!(
        details,
        r#"{"field":"outcome","reason":"too_short","limit":1}"#
    );
}

#[test]
fn list_refuses_the_count_before_any_item() {
    let fifty = vec!["x".to_string(); 50];
    list("constraints", &fifty, 50, 500).unwrap();
    let mut fifty_one = fifty.clone();
    fifty_one.push(String::new());
    let (_, details, _) = refusal(list("constraints", &fifty_one, 50, 500));
    assert_eq!(
        details,
        r#"{"field":"constraints","reason":"too_many","limit":50}"#
    );
    let items = vec!["ok".to_string(), "y".repeat(501)];
    let (_, details, _) = refusal(list("nonGoals", &items, 50, 500));
    assert_eq!(
        details,
        r#"{"field":"nonGoals.1","reason":"too_long","limit":500}"#
    );
}

#[test]
fn key_prefix_checks_length_then_pattern() {
    for good in ["AB", "A1", "ABCDEFGHIJ", "Q3X"] {
        key_prefix(good).unwrap();
    }
    let (_, details, _) = refusal(key_prefix("A"));
    assert_eq!(
        details,
        r#"{"field":"keyPrefix","reason":"too_short","limit":2}"#
    );
    let (_, details, _) = refusal(key_prefix("ABCDEFGHIJK"));
    assert_eq!(
        details,
        r#"{"field":"keyPrefix","reason":"too_long","limit":10}"#
    );
    for bad in ["1A", "ab", "A-B", "AÉ"] {
        let (class, details, _) = refusal(key_prefix(bad));
        assert_eq!(class, Class::Unprocessable, "{bad}");
        assert_eq!(
            details,
            r#"{"field":"keyPrefix","reason":"invalid_format"}"#
        );
    }
}

#[test]
fn page_defaults_to_twenty_and_caps_at_one_hundred() {
    assert_eq!(page(None).unwrap(), 20);
    assert_eq!(page(Some(1)).unwrap(), 1);
    assert_eq!(page(Some(100)).unwrap(), 100);
    let (_, details, _) = refusal(page(Some(101)));
    assert_eq!(
        details,
        r#"{"field":"limit","reason":"too_large","limit":100}"#
    );
    let (_, details, _) = refusal(page(Some(0)));
    assert_eq!(
        details,
        r#"{"field":"limit","reason":"too_small","limit":1}"#
    );
}
