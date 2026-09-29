#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory rebuild` — part 5: every project-owned table (#325, #342).
//!
//! Listing a table in `COPIED_TABLES` satisfies the coverage test and copies
//! nothing on its own (`activity_log` once shipped that way), so this drives
//! the real binary over rows in every project table — every nullable column
//! set in at least one row, both self-references present — and compares each
//! table column for column, by the columns its declaration names.

use assert_cmd::Command;
use comemory::store::schema_projects::{PROJECT_TABLES, table_defs};
use rusqlite::Connection;
use rusqlite::types::Value;
use tempfile::tempdir;

fn run(home: &std::path::Path, args: &[&str]) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home)
        .env("HOME", home)
        .args(args)
        .assert()
        .success();
}

fn db(home: &std::path::Path) -> Connection {
    let conn = Connection::open(home.join("comemory.db")).expect("open db");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enforce keys");
    conn
}

/// One project with a row in every project table, written through real
/// SQLite with foreign keys enforced; both self-references travel through the
/// copy. Shared with the transfer tests (#342).
const SEED: &str = include_str!("fixtures/projects/every_table_seed.sql");

type Rows = Vec<Vec<Value>>;

/// Every row of every project table, by the columns its declaration names,
/// in a stable order.
fn project_rows(home: &std::path::Path) -> Vec<(String, Vec<String>, Rows)> {
    let conn = db(home);
    table_defs()
        .into_iter()
        .map(|def| {
            let columns: Vec<String> = def.columns.iter().map(|c| c.name.clone()).collect();
            let list = columns.join(", ");
            let mut statement = conn
                .prepare(&format!(
                    "SELECT {list} FROM \"{}\" ORDER BY {list}",
                    def.name
                ))
                .expect("prepare");
            let rows = statement
                .query_map([], |r| {
                    (0..columns.len())
                        .map(|i| r.get::<_, Value>(i))
                        .collect::<Result<Vec<_>, _>>()
                })
                .expect("query")
                .collect::<Result<Rows, _>>()
                .expect("collect");
            (def.name, columns, rows)
        })
        .collect()
}

#[test]
fn a_rebuild_keeps_every_project_row_column_for_column() {
    let home = tempdir().expect("home");
    run(
        home.path(),
        &[
            "save",
            "A project's charter and evidence live only in comemory.db.",
            "--kind",
            "decision",
        ],
    );
    db(home.path())
        .execute_batch(SEED)
        .expect("seed every project table");

    let before = project_rows(home.path());
    assert_eq!(before.len(), PROJECT_TABLES.len());
    for (table, columns, rows) in &before {
        assert!(!rows.is_empty(), "{table} was seeded");
        for (i, column) in columns.iter().enumerate() {
            assert!(
                rows.iter()
                    .any(|row| row.get(i).is_some_and(|v| *v != Value::Null)),
                "{table}.{column} is set in at least one row, so its copy is proven"
            );
        }
    }

    run(home.path(), &["rebuild"]);

    let after = project_rows(home.path());
    for ((table, _, rows_before), (_, _, rows_after)) in before.iter().zip(&after) {
        assert_eq!(rows_after, rows_before, "{table} after rebuild");
    }
}
