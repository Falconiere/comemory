#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/show.rs`: a created project reads
//! back by id in either case; a malformed id is a `400`, an unknown one
//! `404 project_not_found`.

use comemory::config::{Config, Paths};
use comemory::domains::projects::authority::{self, Envelope};
use comemory::domains::projects::{create, show};
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use comemory::utilities::error_code::{Class, classify};

#[test]
fn show_reads_back_refuses_malformed_and_answers_unknown_with_404() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().unwrap();
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let req: create::Request = serde_json::from_value(serde_json::json!({
        "idempotencyKey": "k1", "name": "Read me", "keyPrefix": "READ", "outcome": "Read back"
    }))
    .unwrap();
    let created = authority::run(&mut ctx, &Envelope::local_operator(), req)
        .unwrap()
        .project;

    let shown = authority::run(
        &mut ctx,
        &Envelope::local_operator(),
        show::Request {
            id: created.id.to_uppercase(),
        },
    )
    .unwrap();
    assert_eq!(shown.project, created);

    let e = authority::run(
        &mut ctx,
        &Envelope::local_operator(),
        show::Request {
            id: "not-a-uuid".into(),
        },
    )
    .unwrap_err();
    assert_eq!(classify(&e), ("invalid_request", Class::BadRequest));
    let unknown = "00000000-0000-4000-8000-000000000000";
    let e = authority::run(
        &mut ctx,
        &Envelope::local_operator(),
        show::Request { id: unknown.into() },
    )
    .unwrap_err();
    assert_eq!(classify(&e), ("project_not_found", Class::NotFound));
}

/// AC-6: a bound project's show carries its binding; an unbound one has no
/// `transfer` key, so its JSON is the platform's shape unchanged.
#[test]
fn show_carries_transfer_only_when_bound() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().unwrap();
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).unwrap();
    conn.execute_batch(include_str!(
        "../../../../tests/fixtures/projects/every_table_seed.sql"
    ))
    .unwrap();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let bound = authority::run(
        &mut ctx,
        &Envelope::local_operator(),
        show::Request {
            id: "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f".into(),
        },
    )
    .unwrap();
    let json = serde_json::to_value(&bound).unwrap();
    assert_eq!(
        json["transfer"],
        serde_json::json!({
            "direction": "imported",
            "remote": "ws-origin",
            "digest": "d".repeat(64),
            "transferredAt": "2025-09-27T19:06:50.000Z",
            "remappedFrom": "user:u-before",
        })
    );

    let req: create::Request = serde_json::from_value(serde_json::json!({
        "name": "Local only", "keyPrefix": "LOCL", "outcome": "Never moved",
        "idempotencyKey": "show-transfer-local"
    }))
    .unwrap();
    let local = authority::run(&mut ctx, &Envelope::local_operator(), req)
        .unwrap()
        .project;
    let unbound = authority::run(
        &mut ctx,
        &Envelope::local_operator(),
        show::Request { id: local.id },
    )
    .unwrap();
    let json = serde_json::to_value(&unbound).unwrap();
    assert!(json.get("transfer").is_none(), "{json}");
}
