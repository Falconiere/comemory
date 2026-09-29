#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/serve/routes/project_lifecycle.rs`: each verb's path
//! is the platform's, and a lifecycle body parses into the core's `Body` or
//! answers the schema-edge `400` naming its field.

use comemory::domains::projects::lifecycle::{Body, Kind};
use comemory::errors::Error;
use comemory::serve::routes::project_lifecycle::path;
use comemory::serve::routes::project_request::body;
use comemory::utilities::error_code::{Class, classify};

#[test]
fn each_verb_is_posted_to_its_own_segment() {
    let paths: Vec<String> = Kind::ALL.into_iter().map(path).collect();
    assert_eq!(
        paths,
        [
            "/api/v1/projects/{id}/archive",
            "/api/v1/projects/{id}/restore",
            "/api/v1/projects/{id}/pause",
            "/api/v1/projects/{id}/resume",
        ]
    );
}

#[test]
fn a_lifecycle_body_parses_or_names_the_field_it_refuses() {
    let parsed: Body =
        body(br#"{"idempotencyKey":"k","expectedVersion":3,"reason":"why","workspaceId":"w"}"#)
            .unwrap();
    let got = (
        parsed.idempotency_key.as_str(),
        parsed.expected_version,
        parsed.reason.as_deref(),
    );
    assert_eq!(got, ("k", 3, Some("why")));
    for (raw, field, reason) in [
        (r#"{"idempotencyKey":"k"}"#, "expectedVersion", "required"),
        (r#"{"expectedVersion":1}"#, "idempotencyKey", "required"),
        (
            r#"{"idempotencyKey":"k","expectedVersion":"1"}"#,
            "expectedVersion",
            "invalid",
        ),
        (
            r#"{"idempotencyKey":"k","expectedVersion":1,"status":"active"}"#,
            "status",
            "invalid",
        ),
    ] {
        let e = body::<Body>(raw.as_bytes()).unwrap_err();
        assert_eq!(classify(&e), ("invalid_request", Class::BadRequest), "{e}");
        let Error::Project(project) = &e else {
            panic!("{e:?}")
        };
        assert_eq!(
            serde_json::to_string(&project.details()).unwrap(),
            format!(r#"{{"field":"{field}","reason":"{reason}"}}"#),
            "{raw}"
        );
    }
}
