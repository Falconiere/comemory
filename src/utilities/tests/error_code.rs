#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/utilities/error_code.rs`: every unguarded `Error`
//! variant's `(code, Class)`, the three guarded rows (`store_locked` against
//! a real SQLite lock, `Io` `NotFound` vs. any other `Io`), and the
//! remaining fallback-to-`Internal` rows.

use comemory::errors::Error;
use comemory::store::connection;
use comemory::utilities::error_code::{Class, classify};
use tempfile::tempdir;

/// One representative constructed value per `Error` variant that has no
/// guard, matched against the `(code, Class)` `classify` must return for it.
#[test]
fn classify_covers_every_unguarded_variant() {
    let cases: Vec<(Error, &str, Class)> = vec![
        (Error::NotFound("x".into()), "not_found", Class::NotFound),
        (Error::Forbidden("x".into()), "forbidden", Class::Forbidden),
        (
            Error::BadRequest("x".into()),
            "bad_request",
            Class::BadRequest,
        ),
        (
            Error::ConfirmationRequired("x".into()),
            "confirmation_required",
            Class::BadRequest,
        ),
        (Error::Usage("x".into()), "usage", Class::BadRequest),
        (Error::Config("x".into()), "config", Class::BadRequest),
        (
            Error::Frontmatter("x".into()),
            "frontmatter",
            Class::BadRequest,
        ),
        (Error::Document("x".into()), "document", Class::BadRequest),
        (Error::Ast("x".into()), "ast", Class::BadRequest),
        (
            Error::Json(serde_json::from_str::<serde_json::Value>("not json").unwrap_err()),
            "json",
            Class::BadRequest,
        ),
        (
            Error::VecDimMismatch {
                expected: 1024,
                got: 768,
            },
            "vec_dim_mismatch",
            Class::Unprocessable,
        ),
        (
            Error::SchemaTooNew("x".into()),
            "schema_mismatch",
            Class::Unprocessable,
        ),
        (
            Error::Unavailable("x".into()),
            "unavailable",
            Class::Unavailable,
        ),
        (
            Error::Embedder("x".into()),
            "embedder_unavailable",
            Class::Unavailable,
        ),
        (
            Error::IndexRunning {
                repo: "demo".into(),
                job_id: "0123456789abcdef".into(),
            },
            "index_running",
            Class::Conflict,
        ),
        (
            Error::IdCollision {
                id: "ab12cd34".into(),
            },
            "id_collision",
            Class::Conflict,
        ),
        (Error::Cancelled, "cancelled", Class::Conflict),
        (
            Error::Unsupported("x".into()),
            "unsupported",
            Class::NotImplemented,
        ),
        // Every variant with no explicit code word falls to `internal` /
        // `Class::Internal` rather than to a wildcard arm the compiler could
        // silently accept for a future variant.
        (
            Error::Yaml(serde_yaml::from_str::<serde_yaml::Value>("[").unwrap_err()),
            "internal",
            Class::Internal,
        ),
        (
            Error::Toml(toml::from_str::<toml::Value>("[").unwrap_err()),
            "internal",
            Class::Internal,
        ),
        (
            Error::Git(git2::Error::from_str("boom")),
            "internal",
            Class::Internal,
        ),
        (Error::Migration("x".into()), "internal", Class::Internal),
        (Error::Other("x".into()), "internal", Class::Internal),
    ];

    for (err, expected_code, expected_class) in &cases {
        let (code, class) = classify(err);
        assert_eq!(code, *expected_code, "code for {err:?}");
        assert_eq!(class, *expected_class, "class for {err:?}");
    }
}

/// `Error::Io` splits on kind: `NotFound` is a 404-shaped `not_found`, every
/// other kind falls to `internal`.
#[test]
fn classify_splits_io_on_kind() {
    let not_found = Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "gone"));
    assert_eq!(classify(&not_found), ("not_found", Class::NotFound));

    let other = Error::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "nope",
    ));
    assert_eq!(classify(&other), ("internal", Class::Internal));
}

/// `store_locked`/`Class::Locked` proven against a real `SQLITE_BUSY`: two
/// `store::connection::open` connections share one database file, the first
/// holds a `BEGIN IMMEDIATE` write transaction, and the second — with its
/// `busy_timeout` forced to `0` so it fails immediately instead of waiting
/// out the project's 5000ms default — attempts a write while that lock is
/// held. Mirrors `store::busy::is_locked`'s own `is_locked_true_for_a_real_sqlite_busy`
/// test, which `classify` delegates to for this row.
#[test]
fn classify_reports_store_locked_for_a_real_sqlite_busy() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("comemory.db");

    let holder = connection::open(&db_path).expect("open holder connection");
    let contender = connection::open(&db_path).expect("open contender connection");

    holder
        .execute("BEGIN IMMEDIATE", [])
        .expect("begin immediate on holder");
    contender
        .pragma_update(None, "busy_timeout", 0_i64)
        .expect("set busy_timeout=0 on contender");

    let write_err = contender
        .execute(
            "INSERT INTO schema_meta(key, value) VALUES ('probe', '1')",
            [],
        )
        .expect_err("write must fail while the holder's transaction is live");
    let locked = Error::Sqlite(write_err);

    assert_eq!(
        classify(&locked),
        ("store_locked", Class::Locked),
        "expected a SQLITE_BUSY/SQLITE_LOCKED classification, got {locked:?}"
    );

    holder.execute("ROLLBACK", []).expect("rollback holder");
}

/// A non-busy `Sqlite` failure is not `store_locked` — it stays `internal`.
#[test]
fn classify_reports_internal_for_a_non_busy_sqlite_failure() {
    let constraint = Error::Sqlite(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
        None,
    ));
    assert_eq!(classify(&constraint), ("internal", Class::Internal));
}
