#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/bundle_check.rs`, over a bundle
//! exported from a real database loaded with the every-table seed and then
//! edited: each schema difference is `schema_mismatch` and each bad row
//! `malformed`, both `400` and naming the table, and a well-formed bundle in
//! any table order is accepted and put back in canonical order.

use comemory::domains::projects::bundle::{Bundle, TableRows};
use comemory::domains::projects::bundle_check::check;
use comemory::domains::projects::export::snapshot;
use comemory::errors::Error;
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

fn table<'a>(bundle: &'a mut Bundle, name: &str) -> &'a mut TableRows {
    bundle.tables.iter_mut().find(|t| t.table == name).unwrap()
}

/// Set `column` of row `row` of `name`.
fn set(bundle: &mut Bundle, name: &str, row: usize, column: &str, value: Value) {
    let t = table(bundle, name);
    let at = t.columns.iter().position(|c| c == column).unwrap();
    t.rows[row][at] = value;
}

/// Check `bundle`, expecting a `400` with `reason`; returns the message.
fn refused(mut bundle: Bundle, reason: &str) -> String {
    let e = check(&mut bundle, &carried()).expect_err("expected a refusal");
    let Error::Project(project) = &e else {
        panic!("not a project refusal: {e:?}")
    };
    assert_eq!(classify(&e).1, Class::BadRequest, "{e}");
    let details = serde_json::to_value(project.details()).unwrap();
    assert_eq!(details, json!({"field": "bundle", "reason": reason}), "{e}");
    e.to_string()
}

#[test]
fn check_accepts_any_table_order_and_restores_parents_first() {
    let mut shuffled = seeded_bundle();
    shuffled.tables.reverse();
    check(&mut shuffled, &carried()).unwrap();
    assert_eq!(shuffled, seeded_bundle());
}

#[test]
fn check_refuses_schema_mismatch_naming_table_and_column() {
    let mut receipts = seeded_bundle();
    receipts.tables.push(TableRows {
        table: "project_command_receipts".to_string(),
        columns: vec![],
        rows: vec![],
    });
    assert_eq!(
        refused(receipts, "schema_mismatch"),
        "bundle table \"project_command_receipts\" is not carried by this engine"
    );

    let mut missing = seeded_bundle();
    missing.tables.retain(|t| t.table != "project_milestones");
    assert_eq!(
        refused(missing, "schema_mismatch"),
        "bundle must list table project_milestones exactly once"
    );

    let mut extra = seeded_bundle();
    let t = table(&mut extra, "project_approvals");
    t.columns.push("approved_by_robot".to_string());
    for row in &mut t.rows {
        row.push(Value::Null);
    }
    assert_eq!(
        refused(extra, "schema_mismatch"),
        "bundle table project_approvals has columns this engine does not declare"
    );

    let mut reordered = seeded_bundle();
    table(&mut reordered, "project_milestones")
        .columns
        .swap(2, 3);
    assert_eq!(
        refused(reordered, "schema_mismatch"),
        "bundle table project_milestones column 2 must be name"
    );
}

#[test]
fn check_refuses_malformed_rows_naming_the_table() {
    // One cell edited per case, each caught by its own check.
    let edits = [
        (
            ("project_work_items", 0, "title", Value::Null),
            "bundle table project_work_items row 0 column title has the wrong type or is null",
        ),
        (
            ("project_work_items", 0, "number", json!("1")),
            "bundle table project_work_items row 0 column number has the wrong type or is null",
        ),
        (
            ("project_milestones", 0, "position", json!(1.5)),
            "bundle table project_milestones row 0 column position has the wrong type or is null",
        ),
        (
            (
                "project_milestones",
                0,
                "project_id",
                json!("another-project"),
            ),
            "bundle table project_milestones row 0 belongs to another project",
        ),
        (
            (
                "project_work_items",
                1,
                "parent_work_item_id",
                json!("w-elsewhere"),
            ),
            "bundle table project_work_items row 1 (parent_work_item_id, project_id) names a row \
             the bundle does not hold",
        ),
        (
            (
                "project_evidence_criteria",
                0,
                "criterion_id",
                json!("c-elsewhere"),
            ),
            "bundle table project_evidence_criteria row 0 (criterion_id) names a row the bundle \
             does not hold",
        ),
        (
            (
                "project_executions",
                0,
                "superseded_by_execution_id",
                json!("e-x"),
            ),
            "bundle table project_executions row 0 (superseded_by_execution_id) names a row the \
             bundle does not hold",
        ),
        (
            (
                "project_work_item_dependencies",
                0,
                "blocked_id",
                json!("w-1"),
            ),
            "bundle table project_work_item_dependencies row 0 blocks itself",
        ),
    ];
    for ((name, row, column, value), expected) in edits {
        let mut bundle = seeded_bundle();
        set(&mut bundle, name, row, column, value);
        assert_eq!(refused(bundle, "malformed"), expected, "{name}.{column}");
    }

    let mut repeated = seeded_bundle();
    let repositories = table(&mut repeated, "project_repositories");
    let copy = repositories.rows[0].clone();
    repositories.rows.push(copy);
    assert_eq!(
        refused(repeated, "malformed"),
        "bundle table project_repositories row 1 repeats a key"
    );

    let mut narrow = seeded_bundle();
    table(&mut narrow, "project_criteria").rows[0].pop();
    assert_eq!(
        refused(narrow, "malformed"),
        "bundle table project_criteria row 0 has the wrong width"
    );

    let mut empty = seeded_bundle();
    table(&mut empty, "projects").rows.clear();
    assert_eq!(
        refused(empty, "malformed"),
        "bundle table projects must hold exactly one row"
    );

    let mut upper = seeded_bundle();
    upper.project_id = A.to_uppercase();
    assert_eq!(
        refused(upper, "malformed"),
        "bundle projectId is not a lowercase UUID"
    );
}

/// A `Bundle` built in code (as #344/#345 may) is held to the header rules
/// `parse` enforces.
#[test]
fn check_refuses_a_built_bundle_with_another_format_or_version() {
    let mut foreign = seeded_bundle();
    foreign.format = "someone-else.bundle".to_string();
    assert_eq!(
        refused(foreign, "invalid"),
        "bundle is not a comemory project bundle"
    );
    let mut future = seeded_bundle();
    future.version = 2;
    assert_eq!(
        refused(future, "unsupported_version"),
        "bundle version is not supported by this engine (it reads version 1)"
    );
}
