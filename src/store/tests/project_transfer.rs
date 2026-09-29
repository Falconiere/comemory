#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! The transfer row walk against real databases opened through
//! `store::connection::open` (foreign keys on), loaded with the every-table
//! seed: rows scoped to one project in every table (evidence criteria through
//! their evidence), a named-column copy into a second database that only
//! succeeds with deferred keys, the per-table key check, and the identity
//! lookup a collision refusal names.

use comemory::store::connection::{self, write_transaction};
use comemory::store::project_table_shape::{TableShape, carried};
use comemory::store::project_transfer::{Identity, identity_holder, rows};
use comemory::store::project_transfer_write::{
    RowsInsert, defer_foreign_keys, first_key_violation, insert_rows,
};
use rusqlite::Connection;
use serde_json::Value;

const SEED: &str = include_str!("../../../tests/fixtures/projects/every_table_seed.sql");
const A: &str = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f";
const B: &str = "5d3b9a41-6c2e-4f7a-8b1d-2e9c0a7f4b36";

fn fresh() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

/// The seed's project `A`, plus a second project `B` with its own evidence
/// linked to its own criterion, so a scope leak shows up in every table.
fn seeded() -> (tempfile::TempDir, Connection) {
    let (dir, conn) = fresh();
    conn.execute_batch(SEED).expect("seed");
    conn.execute_batch(&format!(
        "INSERT INTO projects (id, slug, key_prefix, name, outcome, lead_principal_type,
             lead_principal_id, creator_principal_type, creator_principal_id)
         VALUES ('{B}', 'beta', 'BET', 'Beta', 'ship beta', 'user', 'u1', 'user', 'u1');
         INSERT INTO project_criteria (id, project_id, description) VALUES ('c-b', '{B}', 'b done');
         INSERT INTO project_evidence (id, project_id, kind, source, creator_principal_type,
             creator_principal_id) VALUES ('v-b', '{B}', 'commit', 'git', 'user', 'u1');
         INSERT INTO project_evidence_criteria (evidence_id, criterion_id) VALUES ('v-b', 'c-b');"
    ))
    .expect("second project");
    (dir, conn)
}

/// `table`'s rows for `project`, sorted, as the walk reads them.
fn shape(table: &str) -> TableShape {
    carried()
        .into_iter()
        .find(|s| s.name == table)
        .expect("a carried table")
}

fn sorted_rows(conn: &Connection, table: &str, project: &str) -> Vec<Vec<Value>> {
    let mut out = rows(conn, &shape(table), project).expect("rows");
    out.sort_by_key(|row| serde_json::to_string(row).expect("json"));
    out
}

#[test]
fn rows_scope_every_carried_table_to_one_project() {
    let (_dir, conn) = seeded();
    for table in carried() {
        let a = sorted_rows(&conn, &table.name, A);
        assert!(!a.is_empty(), "{} has the seed's rows", table.name);
        assert_eq!(a[0].len(), table.columns.len(), "{} width", table.name);
    }
    let criteria = sorted_rows(&conn, "project_evidence_criteria", A);
    assert_eq!(
        criteria,
        vec![vec![Value::from("v-1"), Value::from("c-1")]],
        "evidence criteria reach their project through the evidence"
    );
    assert_eq!(
        sorted_rows(&conn, "project_evidence_criteria", B),
        vec![vec![Value::from("v-b"), Value::from("c-b")]]
    );
    assert_eq!(sorted_rows(&conn, "projects", B).len(), 1);
    assert!(sorted_rows(&conn, "project_work_items", B).is_empty());
    let project = sorted_rows(&conn, "projects", A);
    assert_eq!(project[0][0], Value::from(A));
    assert!(
        project[0].contains(&Value::from(1_767_225_600_000_i64)),
        "INTEGER columns read as JSON numbers"
    );
}

/// Copy every table of `A` from `source` into `target` parents first, each
/// table's rows in primary-key order — which puts the superseded `e-1` before
/// the `e-2` it names — and return the last insert outcome.
fn copy_all(source: &Connection, target: &mut Connection, defer: bool) -> RowsInsert {
    let tx = write_transaction(target).expect("tx");
    if defer {
        defer_foreign_keys(&tx).expect("defer");
    }
    for table in carried() {
        let rows = sorted_rows(source, &table.name, A);
        let outcome = insert_rows(&tx, &table, &rows).expect("insert");
        if outcome != RowsInsert::Inserted {
            return outcome;
        }
    }
    let names: Vec<String> = carried().into_iter().map(|s| s.name).collect();
    let tables: Vec<&str> = names.iter().map(String::as_str).collect();
    assert_eq!(first_key_violation(&tx, &tables).expect("check"), None);
    tx.commit().expect("commit");
    RowsInsert::Inserted
}

#[test]
fn insert_rows_round_trip_named_columns_under_deferred_keys() {
    let (_src, source) = seeded();
    let (_dst, mut target) = fresh();
    assert_eq!(
        copy_all(&source, &mut target, false),
        RowsInsert::Conflict,
        "row by row, e-1 before the e-2 it names trips the immediate key"
    );
    assert_eq!(copy_all(&source, &mut target, true), RowsInsert::Inserted);
    for table in carried() {
        assert_eq!(
            sorted_rows(&target, &table.name, A),
            sorted_rows(&source, &table.name, A),
            "{} column for column",
            table.name
        );
    }
}

#[test]
fn insert_rows_reports_a_row_another_project_holds_as_a_conflict() {
    let (_dir, mut conn) = seeded();
    let milestones = shape("project_milestones");
    let mut taken = sorted_rows(&conn, "project_milestones", A);
    let at = milestones.position("project_id").expect("project_id");
    taken[0][at] = Value::from(B);
    let tx = write_transaction(&mut conn).expect("tx");
    assert_eq!(
        insert_rows(&tx, &milestones, &taken).expect("insert"),
        RowsInsert::Conflict,
        "milestone m-1 already belongs to A"
    );
}

#[test]
fn first_key_violation_names_only_the_tables_it_is_given() {
    let (_dir, mut conn) = seeded();
    let tx = write_transaction(&mut conn).expect("tx");
    defer_foreign_keys(&tx).expect("defer");
    tx.execute_batch(&format!(
        "INSERT INTO project_work_items (id, project_id, number, parent_work_item_id, kind,
             title, description)
         VALUES ('w-orphan', '{A}', 9, 'w-missing', 'task', 'orphan', 'no parent');"
    ))
    .expect("a deferred dangling parent is accepted until commit");
    assert_eq!(
        first_key_violation(&tx, &["project_milestones", "project_work_items"]).expect("check"),
        Some("project_work_items".to_string())
    );
    assert_eq!(
        first_key_violation(&tx, &["project_milestones", "projects"]).expect("check"),
        None,
        "a table outside the list is never scanned"
    );
}

#[test]
fn identity_holder_names_the_project_holding_a_prefix_or_slug() {
    let (_dir, conn) = seeded();
    let holder = |identity, value| identity_holder(&conn, identity, value).expect("lookup");
    assert_eq!(holder(Identity::KeyPrefix, "ALP"), Some(A.to_string()));
    assert_eq!(holder(Identity::Slug, "beta"), Some(B.to_string()));
    assert_eq!(holder(Identity::KeyPrefix, "ZZZ"), None);
    assert_eq!(holder(Identity::Slug, "ALP"), None, "columns are not mixed");
}
