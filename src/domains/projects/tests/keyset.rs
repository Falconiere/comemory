#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/keyset.rs`: the platform's own
//! encode/decode outputs (bun-generated fixture, comemory.io b86dec53),
//! including every refusal, which must be a schema-edge `400`.

use comemory::domains::projects::keyset::{decode, encode};
use comemory::errors::Error;
use comemory::utilities::error_code::{Class, classify};
use serde_json::Value;

/// The committed platform vectors.
fn vectors() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/projects/platform_vectors.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn encode_matches_the_platform() {
    for case in vectors()["encode"].as_array().unwrap() {
        let at = case["atMillis"].as_i64().unwrap();
        assert_eq!(
            encode(at, case["id"].as_str().unwrap()),
            case["cursor"].as_str().unwrap()
        );
    }
}

#[test]
fn decode_accepts_and_refuses_exactly_what_the_platform_does() {
    let vectors = vectors();
    let cases = vectors["decode"].as_array().unwrap();
    assert!(cases.iter().any(|c| c["ok"] == false) && cases.iter().any(|c| c["ok"] == true));
    for case in cases {
        let raw = case["cursor"].as_str().unwrap();
        match decode(raw) {
            Ok(cursor) => {
                assert_eq!(case["ok"], true, "{raw:?} accepted, platform refused");
                assert_eq!(cursor.at_ms, case["atMillis"].as_i64().unwrap());
                assert_eq!(cursor.id, case["id"].as_str().unwrap());
            }
            Err(e) => {
                assert_eq!(case["ok"], false, "{raw:?} refused, platform accepted");
                assert_eq!(classify(&e), ("invalid_request", Class::BadRequest));
                assert_eq!(e.to_string(), case["message"].as_str().unwrap());
                assert!(matches!(e, Error::Project(_)));
            }
        }
    }
}

#[test]
fn a_unicode_digit_is_not_a_cursor_digit() {
    assert!(decode("١٢٣:0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f").is_err());
}
