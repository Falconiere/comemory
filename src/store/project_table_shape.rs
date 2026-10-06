//! The transfer view of the project tables (#342): for each table a bundle
//! carries, its columns with their storage kind and nullability, its primary
//! key and every reference it makes, read off the declarations in
//! [`super::schema_projects`]. The bundle validates and canonicalizes against
//! this shape, so it never touches an ORM type and a column added to a
//! declaration is carried, keyed and checked with no new code.

use toolu_orm::core::column::ColumnType;
use toolu_orm::core::table::TableDef;

use super::schema_projects::{Transfer, table_defs, transfer_class};

/// One carried table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableShape {
    /// Table name.
    pub name: String,
    /// Declared columns, in declaration order.
    pub columns: Vec<ColumnShape>,
    /// Primary-key column names, in key order.
    pub key: Vec<String>,
    /// Every reference: column-level and table-level (composite) keys.
    pub references: Vec<Reference>,
}

/// One declared column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnShape {
    /// Column name.
    pub name: String,
    /// `INTEGER` storage (a JSON integer); every other column is `TEXT`.
    pub integer: bool,
    /// Declared `NOT NULL` or part of the primary key.
    pub not_null: bool,
}

/// A foreign key: a row's `columns` hold the `target` columns of one row in
/// `table`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// The referencing columns, in key order.
    pub columns: Vec<String>,
    /// The referenced table.
    pub table: String,
    /// The referenced columns, paired with `columns`.
    pub target: Vec<String>,
}

impl TableShape {
    /// The position of `column` in this table's rows.
    #[must_use]
    pub fn position(&self, column: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == column)
    }

    /// The declared column names, in declaration order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.columns.iter().map(|c| c.name.as_str()).collect()
    }
}

/// Every carried table, parents first — the registry reversed, so importing
/// in this order inserts a row's parents before it.
#[must_use]
pub fn carried() -> Vec<TableShape> {
    table_defs()
        .iter()
        .rev()
        .filter(|def| transfer_class(&def.name) == Some(Transfer::Carried))
        .map(shape)
        .collect()
}

/// The shape of one declaration.
fn shape(def: &TableDef) -> TableShape {
    let key: Vec<String> = if def.primary_key.is_empty() {
        def.columns
            .iter()
            .filter(|c| c.primary_key)
            .map(|c| c.name.clone())
            .collect()
    } else {
        def.primary_key.clone()
    };
    let columns = def
        .columns
        .iter()
        .map(|c| ColumnShape {
            name: c.name.clone(),
            integer: c.column_type == ColumnType::Integer,
            not_null: c.not_null || key.contains(&c.name),
        })
        .collect();
    let column_refs = def.columns.iter().filter_map(|c| {
        let (table, target) = split_reference(c.references.as_deref()?)?;
        Some(Reference {
            columns: vec![c.name.clone()],
            table: table.to_string(),
            target: vec![target.to_string()],
        })
    });
    let composite = def.foreign_keys.iter().map(|fk| Reference {
        columns: fk.columns.clone(),
        table: fk.references_table.clone(),
        target: fk.references_columns.clone(),
    });
    TableShape {
        name: def.name.clone(),
        columns,
        key,
        references: column_refs.chain(composite).collect(),
    }
}

/// `"project_evidence(id)"` as `("project_evidence", "id")`.
fn split_reference(reference: &str) -> Option<(&str, &str)> {
    let (table, rest) = reference.split_once('(')?;
    Some((table.trim(), rest.strip_suffix(')')?.trim()))
}

#[cfg(test)]
#[path = "tests/project_table_shape.rs"]
mod tests;
