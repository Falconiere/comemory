#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Canonical-JSON encoding: key order cannot change the bytes or the digest,
//! and a digest is taken over the bytes that actually arrived.

use serde_json::json;

use super::{bytes_and_digest, digest_of, to_bytes};

#[test]
fn key_insertion_order_does_not_change_the_bytes_or_the_digest() {
    // The same memory payload assembled in two orders, as two code paths
    // building the same record would.
    let one = json!({
        "frontmatter": {"id": "a1b2c3d4", "kind": "decision", "tags": ["sync", "journal"]},
        "body": "The engine orders changes by server acceptance.",
    });
    let two = json!({
        "body": "The engine orders changes by server acceptance.",
        "frontmatter": {"tags": ["sync", "journal"], "kind": "decision", "id": "a1b2c3d4"},
    });

    let (bytes_one, digest_one) = bytes_and_digest(&one).expect("canonical one");
    let (bytes_two, digest_two) = bytes_and_digest(&two).expect("canonical two");

    assert_eq!(bytes_one, bytes_two);
    assert_eq!(digest_one, digest_two);
    assert_eq!(digest_one.len(), 64);
    assert_eq!(
        String::from_utf8(bytes_one).expect("utf8"),
        r#"{"body":"The engine orders changes by server acceptance.","frontmatter":{"id":"a1b2c3d4","kind":"decision","tags":["sync","journal"]}}"#
    );
}

#[test]
fn array_order_is_content_and_is_preserved() {
    let ordered = json!({"tags": ["b", "a"]});
    let reversed = json!({"tags": ["a", "b"]});
    assert_ne!(
        to_bytes(&ordered).expect("bytes"),
        to_bytes(&reversed).expect("bytes"),
        "array order is payload content, not key order"
    );
}

#[test]
fn a_changed_metadata_field_changes_the_digest() {
    let before = json!({"frontmatter": {"id": "a1b2c3d4", "tags": ["sync"]}, "body": "same"});
    let after =
        json!({"frontmatter": {"id": "a1b2c3d4", "tags": ["sync", "journal"]}, "body": "same"});
    let (_, before_digest) = bytes_and_digest(&before).expect("before");
    let (_, after_digest) = bytes_and_digest(&after).expect("after");
    assert_ne!(
        before_digest, after_digest,
        "the digest covers metadata, not the body alone"
    );
}

#[test]
fn digest_of_hashes_the_bytes_that_arrived() {
    let (bytes, digest) = bytes_and_digest(&json!({"a": 1})).expect("canonical");
    assert_eq!(digest_of(&bytes), digest);
    let mut tampered = bytes.clone();
    let last = tampered.len() - 2;
    tampered[last] = b'2';
    assert_ne!(digest_of(&tampered), digest);
}

#[test]
fn sorting_reaches_objects_nested_inside_arrays() {
    // A shallow sort would leave these two byte strings different, and two
    // machines would disagree on the digest of the same references list.
    let one = json!({"references": [{"symbol": "run", "path": "src/lib.rs"}]});
    let two = json!({"references": [{"path": "src/lib.rs", "symbol": "run"}]});
    assert_eq!(
        to_bytes(&one).expect("one"),
        to_bytes(&two).expect("two"),
        "objects inside arrays must be sorted too"
    );
}
