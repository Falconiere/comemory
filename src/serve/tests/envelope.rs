#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for `src/serve/envelope.rs`: the full `Error → (StatusCode,
//! code)` table (§Interfaces "Response envelope") and the `{ok,data,meta}` /
//! `{ok,error,meta}` JSON shapes each constructor produces, asserted by
//! deserializing the real `axum::Response` body.

use axum::body::to_bytes;
use axum::http::{StatusCode, header};
use axum::response::Response;
use comemory::errors::Error;
use comemory::serve::envelope::{self, Envelope};

/// Collect a response body into a `serde_json::Value`.
async fn json_body(res: Response) -> serde_json::Value {
    let status = res.status();
    let bytes = to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap_or_else(|e| panic!("collect ({status}) response body: {e}"));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("response body ({status}) is not valid JSON: {e}"))
}

/// Assert one `status_and_code` row from the spec table.
fn assert_row(err: &Error, expected_status: StatusCode, expected_code: &str) {
    let (status, code) = envelope::status_and_code(err);
    assert_eq!(status, expected_status, "status for {err:?}");
    assert_eq!(code, expected_code, "code for {err:?}");
}

#[test]
fn status_and_code_covers_the_named_client_error_rows() {
    assert_row(
        &Error::NotFound("x".into()),
        StatusCode::NOT_FOUND,
        "not_found",
    );
    assert_row(
        &Error::Forbidden("x".into()),
        StatusCode::FORBIDDEN,
        "forbidden",
    );
    assert_row(
        &Error::BadRequest("x".into()),
        StatusCode::BAD_REQUEST,
        "bad_request",
    );
    assert_row(
        &Error::VecDimMismatch {
            expected: 1024,
            got: 768,
        },
        StatusCode::UNPROCESSABLE_ENTITY,
        "vec_dim_mismatch",
    );
    assert_row(
        &Error::Unavailable("x".into()),
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
    );
}

#[test]
fn status_and_code_covers_the_grouped_bad_request_rows() {
    let json_err = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
    assert_row(&Error::Usage("x".into()), StatusCode::BAD_REQUEST, "usage");
    assert_row(
        &Error::Config("x".into()),
        StatusCode::BAD_REQUEST,
        "config",
    );
    assert_row(
        &Error::Frontmatter("x".into()),
        StatusCode::BAD_REQUEST,
        "frontmatter",
    );
    assert_row(
        &Error::Document("x".into()),
        StatusCode::BAD_REQUEST,
        "document",
    );
    assert_row(&Error::Ast("x".into()), StatusCode::BAD_REQUEST, "ast");
    assert_row(&Error::Json(json_err), StatusCode::BAD_REQUEST, "json");
}

#[test]
fn status_and_code_covers_io_and_the_fallback_row() {
    let io_not_found = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
    let io_other = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "nope");
    assert_row(&Error::Io(io_not_found), StatusCode::NOT_FOUND, "not_found");
    assert_row(
        &Error::Io(io_other),
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
    );
    assert_row(
        &Error::Other("x".into()),
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
    );
}

/// The explicit `Error::Migration(_) | Error::SchemaTooNew(_)` arm (F8): a
/// broken migration chain and a database written by a newer comemory are
/// both server-side schema problems, mapped to `500`/`"internal"` — the
/// same bucket `main.rs::exit_code` puts them in. Listed explicitly in
/// `status_and_code` rather than left to the `_` fallback, so this row is
/// asserted directly instead of only incidentally by the fallback's own
/// coverage.
#[test]
fn status_and_code_covers_the_explicit_migration_and_schema_too_new_row() {
    assert_row(
        &Error::Migration("x".into()),
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
    );
    assert_row(
        &Error::SchemaTooNew("x".into()),
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
    );
}

#[tokio::test]
async fn ok_envelope_has_the_data_and_meta_shape() {
    let res = Envelope::ok("search", serde_json::json!({"hits": []}), 12);
    assert_eq!(res.status(), StatusCode::OK);

    let body = json_body(res).await;
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["data"], serde_json::json!({"hits": []}));
    assert_eq!(body["meta"]["command"], "search");
    assert_eq!(body["meta"]["elapsed_ms"], 12);
    assert!(body.get("error").is_none(), "no error key on success");
}

#[tokio::test]
async fn err_envelope_has_the_error_and_meta_shape() {
    let e = Error::NotFound("ab12cd34".into());
    let res = Envelope::err("memories.delete", &e, 1);
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let body = json_body(res).await;
    assert_eq!(body["ok"], serde_json::json!(false));
    assert_eq!(body["error"]["code"], "not_found");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("ab12cd34")),
        "message carries the underlying detail: {body}"
    );
    assert_eq!(body["meta"]["command"], "memories.delete");
    assert_eq!(body["meta"]["elapsed_ms"], 1);
    assert!(body.get("data").is_none(), "no data key on error");
}

#[tokio::test]
async fn busy_envelope_is_503_with_retry_after_and_the_busy_code() {
    let res = Envelope::busy("code.index");
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let retry_after = res
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    assert_eq!(retry_after.as_deref(), Some("5"));

    let body = json_body(res).await;
    assert_eq!(body["error"]["code"], envelope::CODE_BUSY);
    assert_eq!(body["meta"]["command"], "code.index");
}

#[tokio::test]
async fn read_only_envelope_is_405_with_the_read_only_code() {
    let res = Envelope::read_only("memories.save");
    assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);

    let body = json_body(res).await;
    assert_eq!(body["error"]["code"], envelope::CODE_READ_ONLY);
    assert_eq!(body["meta"]["command"], "memories.save");
}

#[tokio::test]
async fn confirmation_required_envelope_is_400_with_its_code() {
    let res = Envelope::confirmation_required("memories.delete");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body = json_body(res).await;
    assert_eq!(body["error"]["code"], envelope::CODE_CONFIRMATION_REQUIRED);
    assert_eq!(body["meta"]["command"], "memories.delete");
}

#[tokio::test]
async fn unauthorized_envelope_is_401_with_its_code() {
    let res = Envelope::unauthorized("auth");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let body = json_body(res).await;
    assert_eq!(body["error"]["code"], envelope::CODE_UNAUTHORIZED);
    assert_eq!(body["meta"]["command"], "auth");
}
