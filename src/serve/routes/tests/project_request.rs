#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/serve/routes/project_request.rs`: each malformed body
//! or query names its field the way the platform's `invalidRequestFrom`
//! does, always at the schema edge (`400`).

use comemory::domains::projects::create;
use comemory::errors::{Error, Result};
use comemory::serve::routes::project_request::{body, list_query};
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
    let cases: [(&str, &str, &str); 6] = [
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
            r#"{"keyPrefix":"AB","outcome":"o"}"#,
            "name is required",
            r#"{"field":"name","reason":"required"}"#,
        ),
        (
            r#"{"name":5,"keyPrefix":"AB","outcome":"o"}"#,
            "name is invalid",
            r#"{"field":"name","reason":"invalid"}"#,
        ),
        (
            r#"{"name":"n","keyPrefix":"AB","outcome":"o","successCriteria":["a",7]}"#,
            "successCriteria.1 is invalid",
            r#"{"field":"successCriteria.1","reason":"invalid"}"#,
        ),
        (
            r#"{"name":"n","keyPrefix":"AB","outcome":"o","idempotencyKey":"k"}"#,
            "idempotencyKey is invalid",
            r#"{"field":"idempotencyKey","reason":"invalid"}"#,
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
    let ok: create::Request =
        body(br#"{"workspaceId":"ws","name":"n","keyPrefix":"AB","outcome":"o"}"#).unwrap();
    assert_eq!(ok.name, "n");
}

#[test]
fn query_pairs_become_a_list_request() {
    let pairs = |v: &[(&str, &str)]| -> std::result::Result<Vec<(String, String)>, ()> {
        Ok(v.iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect())
    };
    let req = list_query(pairs(&[
        ("limit", "5"),
        ("includeArchived", "true"),
        ("status", "active"),
        ("workspaceId", "ignored"),
    ]))
    .unwrap();
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
        let (_, details) = refusal(list_query(pairs(&bad)));
        assert_eq!(
            details,
            format!(r#"{{"field":"{field}","reason":"invalid"}}"#)
        );
    }
    let (_, details) = refusal(list_query::<()>(Err(())));
    assert_eq!(details, r#"{"field":"query","reason":"invalid"}"#);
}
