#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/serve/routes/project_request.rs`: each malformed body
//! or query — the list's and the change feed's — names its field the way the
//! platform's `invalidRequestFrom` does, always at the schema edge (`400`).

use comemory::domains::projects::create;
use comemory::errors::{Error, Result};
use comemory::serve::routes::project_request::{body, list_field, query};
use comemory::serve::routes::projects::changes_field;
use comemory::utilities::error_code::{Class, classify};

/// `(message, details)` of a schema-edge refusal.
fn refusal<T: std::fmt::Debug>(result: Result<T>) -> (String, String) {
    let e = result.expect_err("expected a refusal");
    assert_eq!(classify(&e), ("invalid_request", Class::BadRequest), "{e}");
    let Error::Project(project) = &e else {
        panic!("{e:?}")
    };
    (
        e.to_string(),
        serde_json::to_string(&project.details()).unwrap(),
    )
}

#[test]
fn body_refusals_name_the_field() {
    let cases: [(&str, &str, &str); 8] = [
        (
            "[]",
            "body must be an object",
            r#"{"field":"body","reason":"invalid"}"#,
        ),
        (
            "not json",
            "body must be an object",
            r#"{"field":"body","reason":"invalid"}"#,
        ),
        (
            r#"{"idempotencyKey":"k","keyPrefix":"AB","outcome":"o"}"#,
            "name is required",
            r#"{"field":"name","reason":"required"}"#,
        ),
        (
            r#"{"idempotencyKey":"k","name":5,"keyPrefix":"AB","outcome":"o"}"#,
            "name is invalid",
            r#"{"field":"name","reason":"invalid"}"#,
        ),
        (
            r#"{"idempotencyKey":"k","name":"n","keyPrefix":"AB","outcome":"o","successCriteria":["a",7]}"#,
            "successCriteria.1 is invalid",
            r#"{"field":"successCriteria.1","reason":"invalid"}"#,
        ),
        (
            r#"{"name":"n","keyPrefix":"AB","outcome":"o"}"#,
            "idempotencyKey is required",
            r#"{"field":"idempotencyKey","reason":"required"}"#,
        ),
        (
            r#"{"name":"n","keyPrefix":"AB","outcome":"o","idempotencyKey":7}"#,
            "idempotencyKey is invalid",
            r#"{"field":"idempotencyKey","reason":"invalid"}"#,
        ),
        (
            r#"{"name":"n","keyPrefix":"AB","outcome":"o","idempotencyKey":"k","extra":1}"#,
            "extra is invalid",
            r#"{"field":"extra","reason":"invalid"}"#,
        ),
    ];
    for (raw, message, details) in cases {
        let got = refusal(body::<create::Request>(raw.as_bytes()));
        assert_eq!(
            (got.0.as_str(), got.1.as_str()),
            (message, details),
            "{raw}"
        );
    }
    let ok: create::Request = body(
        br#"{"workspaceId":"ws","idempotencyKey":"k","name":"n","keyPrefix":"AB","outcome":"o"}"#,
    )
    .unwrap();
    assert_eq!(ok.name, "n");
}

/// `v` as owned query pairs.
fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

#[test]
fn query_pairs_become_a_list_request() {
    let pairs_in = pairs(&[
        ("limit", "5"),
        ("includeArchived", "true"),
        ("status", "active"),
        ("workspaceId", "ignored"),
    ]);
    let req = query(Ok::<_, ()>(pairs_in), list_field).unwrap();
    assert_eq!(
        (req.limit, req.include_archived, req.status.as_deref()),
        (Some(5), Some(true), Some("active"))
    );
    for bad in [
        vec![("limit", "ten")],
        vec![("includeArchived", "yes")],
        vec![("limit", "1"), ("limit", "2")],
        vec![("sort", "name")],
    ] {
        let field = bad[0].0;
        let (_, details) = refusal(query(Ok::<_, ()>(pairs(&bad)), list_field));
        assert_eq!(
            details,
            format!(r#"{{"field":"{field}","reason":"invalid"}}"#)
        );
    }
    let (_, details) = refusal(query(Err::<_, ()>(()), list_field));
    assert_eq!(details, r#"{"field":"query","reason":"invalid"}"#);
}

#[test]
fn query_pairs_become_a_changes_request() {
    let pairs_in = pairs(&[("after", "7"), ("limit", "500"), ("workspaceId", "ignored")]);
    let req = query(Ok::<_, ()>(pairs_in), changes_field).unwrap();
    assert_eq!((req.after, req.limit), (Some(7), Some(500)));
    let empty = query(Ok::<_, ()>(pairs(&[])), changes_field).unwrap();
    assert_eq!((empty.after, empty.limit), (None, None));
    for bad in [
        vec![("after", "x")],
        vec![("after", "1.5")],
        vec![("limit", "ten")],
        vec![("after", "1"), ("after", "2")],
        vec![("since", "0")],
        vec![("cursor", "0")],
    ] {
        let field = bad[0].0;
        let (_, details) = refusal(query(Ok::<_, ()>(pairs(&bad)), changes_field));
        assert_eq!(
            details,
            format!(r#"{{"field":"{field}","reason":"invalid"}}"#)
        );
    }
    let (_, details) = refusal(query(Err::<_, ()>(()), changes_field));
    assert_eq!(details, r#"{"field":"query","reason":"invalid"}"#);
}
