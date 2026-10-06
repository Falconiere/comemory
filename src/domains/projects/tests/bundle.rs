#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/bundle.rs`, over a bundle exported
//! from a real database loaded with the every-table seed: the digest ignores
//! row order and tracks every value, `parse` classifies a bad header before
//! the body, and the actor remap rewrites only matching principal pairs.

use comemory::domains::projects::bundle::{self, ActorRemap, Bundle, parse, remap_actors};
use comemory::domains::projects::export::snapshot;
use comemory::domains::projects::principal::{LOCAL_OPERATOR_ID, Principal, PrincipalType};
use comemory::errors::{Error, Result};
use comemory::store::connection;
use comemory::store::project_table_shape::carried;
use comemory::utilities::error_code::{Class, classify};
use serde_json::{Value, json};

const SEED: &str = include_str!("../../../../tests/fixtures/projects/every_table_seed.sql");
const A: &str = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f";

fn seeded_bundle() -> Bundle {
    let dir = tempfile::tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    conn.execute_batch(SEED).unwrap();
    snapshot(&conn, A).unwrap()
}

/// The refusal's class and serialized details.
fn refusal<T: std::fmt::Debug>(result: Result<T>) -> (Class, Value) {
    let e = result.expect_err("expected a refusal");
    let Error::Project(project) = &e else {
        panic!("not a project refusal: {e:?}")
    };
    (
        classify(&e).1,
        serde_json::to_value(project.details()).unwrap(),
    )
}

/// The value of `column` in the first row of `table`.
fn cell<'a>(bundle: &'a Bundle, table: &str, row: usize, column: &str) -> &'a Value {
    let t = bundle.tables.iter().find(|t| t.table == table).unwrap();
    let at = t.columns.iter().position(|c| c == column).unwrap();
    &t.rows[row][at]
}

#[test]
fn digest_ignores_row_order_and_changes_with_any_value() {
    let sealed = seeded_bundle();
    assert_eq!(sealed.digest.len(), 64);
    assert_eq!(bundle::digest(&sealed.tables).unwrap(), sealed.digest);

    let mut permuted = sealed.clone();
    for table in &mut permuted.tables {
        table.rows.reverse();
    }
    bundle::sort_rows(&mut permuted.tables, &carried());
    assert_eq!(permuted, sealed, "sorting restores the canonical order");

    let mut edited = sealed.clone();
    let items = edited
        .tables
        .iter_mut()
        .find(|t| t.table == "project_work_items")
        .unwrap();
    items.rows[0][6] = json!("renamed");
    assert_ne!(bundle::digest(&edited.tables).unwrap(), sealed.digest);
}

#[test]
fn parse_refuses_non_json_wrong_format_and_future_version() {
    let sealed = seeded_bundle();
    let bytes = serde_json::to_vec(&sealed).unwrap();
    assert_eq!(parse(&bytes).unwrap(), sealed, "a bundle round-trips");

    let (class, details) = refusal(parse(b"not json"));
    assert_eq!(class, Class::BadRequest);
    assert_eq!(details, json!({"field": "bundle", "reason": "invalid"}));

    let mut value = serde_json::to_value(&sealed).unwrap();
    value["format"] = json!("someone-else.bundle");
    let (class, details) = refusal(parse(&serde_json::to_vec(&value).unwrap()));
    assert_eq!(
        (class, details["reason"].clone()),
        (Class::BadRequest, json!("invalid"))
    );

    value["format"] = json!(bundle::FORMAT);
    value["version"] = json!(2);
    value["futureField"] = json!(true);
    let (class, details) = refusal(parse(&serde_json::to_vec(&value).unwrap()));
    assert_eq!(class, Class::BadRequest);
    assert_eq!(
        details,
        json!({"field": "bundle", "reason": "unsupported_version"}),
        "a future version is named before its unknown fields"
    );

    value["version"] = json!(1);
    let (_, details) = refusal(parse(&serde_json::to_vec(&value).unwrap()));
    assert_eq!(details["reason"], "invalid", "v1 has no futureField");
}

#[test]
fn remap_rewrites_only_matching_principal_pairs() {
    let mut remapped = seeded_bundle();
    let remap = ActorRemap {
        from: Principal::new(PrincipalType::User, LOCAL_OPERATOR_ID),
        to: Principal::new(PrincipalType::User, "u-platform"),
    };
    remap_actors(&mut remapped.tables, &carried(), &remap);

    for (table, column) in [
        ("projects", "creator_principal_id"),
        ("project_repositories", "creator_principal_id"),
        ("project_work_items", "assignee_principal_id"),
        ("project_approvals", "reviewer_principal_id"),
        ("project_activity_events", "actor_principal_id"),
    ] {
        assert_eq!(
            cell(&remapped, table, 0, column),
            "u-platform",
            "{table}.{column}"
        );
    }
    assert_eq!(
        cell(&remapped, "projects", 0, "lead_principal_id"),
        "u-lead"
    );
    assert_eq!(
        cell(&remapped, "project_work_items", 1, "assignee_principal_id"),
        "g-1",
        "a project agent is never a user"
    );
    assert_eq!(
        cell(&remapped, "project_evidence", 0, "verified_by"),
        "engine"
    );

    let agent = ActorRemap {
        from: Principal::new(PrincipalType::User, "g-1"),
        to: Principal::new(PrincipalType::User, "u-other"),
    };
    let mut untouched = seeded_bundle();
    remap_actors(&mut untouched.tables, &carried(), &agent);
    assert_eq!(
        untouched,
        seeded_bundle(),
        "g-1 is a project_agent, so a user remap of g-1 matches nothing"
    );
}
