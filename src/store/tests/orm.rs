#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Generated statements retain bindings, errors and transaction ownership.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::*;
use crate::store::schema_core::{SchemaMeta, schema_meta};

fn connection() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    conn.execute_batch("CREATE TABLE schema_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL)")
        .expect("schema");
    conn
}

fn insert(key: &str, value: &str) -> (String, Vec<Value>) {
    SchemaMeta::insert()
        .set(&schema_meta::key, key)
        .set(&schema_meta::value, value)
        .to_sql()
}

fn select(key: &str) -> (String, Vec<Value>) {
    SchemaMeta::select()
        .columns_typed(&[&schema_meta::value])
        .filter(schema_meta::key.eq(key))
        .to_sql()
}

#[test]
fn values_are_bound_and_updates_keep_filter_parameter_order() {
    let conn = connection();
    let key = "quote' OR 1=1 --";
    assert_eq!(execute(&conn, insert(key, "original")).unwrap(), 1);
    execute(&conn, insert("other", "untouched")).unwrap();
    let update = SchemaMeta::update()
        .set(&schema_meta::value, "updated ?2 ' value")
        .filter(schema_meta::key.eq(key));
    assert_eq!(execute(&conn, update.to_sql()).unwrap(), 1);
    let values = query_all(
        &conn,
        SchemaMeta::select()
            .columns_typed(&[&schema_meta::value])
            .order_by(schema_meta::key.asc())
            .to_sql(),
        |row| row.get::<_, String>(0),
    )
    .unwrap();
    assert_eq!(values, ["untouched", "updated ?2 ' value"]);
}

#[test]
fn absent_and_invalid_rows_preserve_native_error_semantics() {
    let conn = connection();
    assert_eq!(
        query_optional(&conn, select("missing"), |row| row.get::<_, String>(0)).unwrap(),
        None
    );
    assert!(matches!(
        query_one(&conn, select("missing"), |row| row.get::<_, String>(0)),
        Err(Error::Sqlite(rusqlite::Error::QueryReturnedNoRows))
    ));
    execute(&conn, insert("key", "not an integer")).unwrap();
    assert!(matches!(
        query_optional(&conn, select("key"), |row| row.get::<_, i64>(0)),
        Err(Error::Sqlite(rusqlite::Error::InvalidColumnType(..)))
    ));
    assert!(matches!(
        execute(&conn, insert("key", "duplicate")),
        Err(Error::Sqlite(rusqlite::Error::SqliteFailure(..)))
    ));
}

#[test]
fn caller_transaction_rolls_back_generated_writes() {
    let mut conn = connection();
    {
        let tx = conn.transaction().unwrap();
        execute(&tx, insert("key", "temporary")).unwrap();
        assert_eq!(
            query_one(&tx, select("key"), |row| row.get::<_, String>(0)).unwrap(),
            "temporary"
        );
    }
    assert_eq!(
        query_one(&conn, SchemaMeta::select().to_count_sql(), |row| row
            .get::<_, i64>(0))
        .unwrap(),
        0
    );
}
