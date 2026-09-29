#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/export.rs`, against a real data
//! directory loaded with the every-table seed: the bundle carries the
//! thirteen carried tables and never a receipt or a binding (AC-5), writes
//! only its telemetry row, and a malformed or unknown id is refused as `show`
//! refuses it (AC-11).

use comemory::config::{Config, Paths};
use comemory::domains::projects::authority::{self, Envelope};
use comemory::domains::projects::export::Request;
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use comemory::utilities::error_code::{Class, classify};

const SEED: &str = include_str!("../../../../tests/fixtures/projects/every_table_seed.sql");
const A: &str = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f";

#[test]
fn export_carries_every_carried_table_and_never_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().unwrap();
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).unwrap();
    conn.execute_batch(SEED).unwrap();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let bundle = authority::run(
        &mut ctx,
        &Envelope::local_operator(),
        Request {
            id: A.to_uppercase(),
        },
    )
    .unwrap();
    assert_eq!(bundle.project_id, A, "the id is stored lowercase");
    let names: Vec<&str> = bundle.tables.iter().map(|t| t.table.as_str()).collect();
    assert_eq!(names.len(), 13);
    assert_eq!(names[0], "projects");
    for kept in ["project_command_receipts", "project_transfer_bindings"] {
        assert!(!names.contains(&kept), "{kept} stays in its data directory");
    }
    assert!(bundle.tables.iter().all(|t| !t.rows.is_empty()));
    let again = authority::run(
        &mut ctx,
        &Envelope::local_operator(),
        Request { id: A.into() },
    )
    .unwrap();
    assert_eq!(again, bundle, "an export is repeatable byte for byte");

    let logged: i64 = conn
        .query_row(
            "SELECT count(*) FROM activity_log WHERE command = 'project.export'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        logged, 2,
        "each export leaves its telemetry row and nothing else"
    );
}

#[test]
fn export_refuses_malformed_and_unknown_ids() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().unwrap();
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let e = authority::run(
        &mut ctx,
        &Envelope::local_operator(),
        Request { id: "nope".into() },
    )
    .unwrap_err();
    assert_eq!(classify(&e), ("invalid_request", Class::BadRequest));
    assert_eq!(e.to_string(), "projectId is invalid");
    let unknown = "00000000-0000-4000-8000-000000000000";
    let e = authority::run(
        &mut ctx,
        &Envelope::local_operator(),
        Request { id: unknown.into() },
    )
    .unwrap_err();
    assert_eq!(classify(&e), ("project_not_found", Class::NotFound));
}
