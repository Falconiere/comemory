//! The project quarter of [`super::rebuild_copy`]'s preservation copy: the
//! fourteen tables of [`super::schema_projects::PROJECT_TABLES`], which hold
//! operator-authored charters, plans, evidence and receipts that exist nowhere
//! but this database.
//!
//! Each table's column list comes from its declaration, so a column added
//! later cannot be silently left behind, narrowed to the columns the attached
//! `old` table actually has, so an older source copies what it holds and the
//! rest take their declared defaults.

use toolu_orm::core::alias::TableRef;
use toolu_orm::query::insert::InsertBuilder;
use toolu_orm::query::select::SelectBuilder;

use crate::prelude::*;
use crate::store::rebuild_copy::{old_column_exists, old_table_exists};
use crate::store::{Connection, orm, schema_projects};

/// Copy every project table from `old` into `main`, parents first — the
/// registry reversed — because the connection enforces foreign keys and
/// `OR IGNORE` does not cover a violation. A self-referencing table
/// (`parent_work_item_id`, `superseded_by_execution_id`) is one statement, and
/// SQLite checks an immediate key at the end of the statement, so row order
/// inside it does not matter. A source without project tables copies nothing.
pub(crate) fn copy_project_tables(conn: &Connection) -> Result<()> {
    for table in schema_projects::table_defs().iter().rev() {
        if !old_table_exists(conn, &table.name)? {
            continue;
        }
        let mut columns: Vec<&str> = Vec::with_capacity(table.columns.len());
        for column in &table.columns {
            if old_column_exists(conn, &table.name, &column.name)? {
                columns.push(&column.name);
            }
        }
        let source = SelectBuilder::from_table(TableRef::new(&table.name).in_database("old"))
            .columns_raw(&columns);
        orm::execute(
            conn,
            InsertBuilder::into_table(TableRef::new(&table.name).in_database("main"))
                .or_ignore()
                .select_raw(&columns, source)
                .to_sql(),
        )?;
    }
    Ok(())
}
