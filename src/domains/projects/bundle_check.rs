//! A parsed [`Bundle`] checked against this engine's carried tables before any
//! store access (#342): a `400 invalid_request` on `bundle`, `schema_mismatch`
//! for another engine's table or column list, `malformed` for a bad row —
//! including any reference to a row the bundle does not hold, so an import
//! never points into another local project. Messages name schema identifiers
//! and row positions only.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::domains::projects::bundle::{
    Bundle, FORMAT, NOT_A_BUNDLE, TableRows, UNSUPPORTED_VERSION, VERSION, refusal,
};
use crate::prelude::*;
use crate::store::project_table_shape::TableShape;
use crate::utilities::uuid;

/// Check `bundle` against `shapes` (the carried tables, parents first) and
/// put its tables in that order.
pub fn check(bundle: &mut Bundle, shapes: &[TableShape]) -> Result<()> {
    // `parse` checks the header too; a caller that builds a `Bundle` directly
    // (#344, #345) must not skip it.
    if bundle.format != FORMAT {
        return Err(refusal("invalid", NOT_A_BUNDLE));
    }
    if bundle.version != VERSION {
        return Err(refusal("unsupported_version", UNSUPPORTED_VERSION));
    }
    if uuid::canonical(&bundle.project_id).as_deref() != Some(bundle.project_id.as_str()) {
        return Err(malformed("bundle projectId is not a lowercase UUID"));
    }
    bundle.tables = in_shape_order(std::mem::take(&mut bundle.tables), shapes)?;
    for (table, shape) in bundle.tables.iter().zip(shapes) {
        check_rows(table, shape, &bundle.project_id)?;
    }
    check_references(&bundle.tables, shapes)
}

/// `tables` reordered to `shapes`, refusing a missing, repeated or unknown
/// table and a column list unlike the declaration.
fn in_shape_order(mut tables: Vec<TableRows>, shapes: &[TableShape]) -> Result<Vec<TableRows>> {
    let mut ordered = Vec::with_capacity(shapes.len());
    for shape in shapes {
        let found: Vec<usize> = tables
            .iter()
            .enumerate()
            .filter(|(_, t)| t.table == shape.name)
            .map(|(i, _)| i)
            .collect();
        let [at] = found[..] else {
            return Err(mismatch(&format!(
                "bundle must list table {} exactly once",
                shape.name
            )));
        };
        let table = tables.swap_remove(at);
        let declared = shape.columns.iter().map(|c| c.name.as_str());
        if let Some((i, expected)) = declared
            .enumerate()
            .find(|(i, name)| table.columns.get(*i).map(String::as_str) != Some(name))
        {
            return Err(mismatch(&format!(
                "bundle table {} column {i} must be {expected}",
                shape.name
            )));
        }
        if table.columns.len() != shape.columns.len() {
            return Err(mismatch(&format!(
                "bundle table {} has columns this engine does not declare",
                shape.name
            )));
        }
        ordered.push(table);
    }
    if let Some(extra) = tables.first() {
        // The name came from the bundle: quoted and bounded, never echoed raw.
        let name: String = extra.table.chars().take(64).collect();
        return Err(mismatch(&format!(
            "bundle table {name:?} is not carried by this engine"
        )));
    }
    Ok(ordered)
}

/// Width, value types, nullability, ownership, key uniqueness and the
/// dependency self-edge of every row of `table`.
fn check_rows(table: &TableRows, shape: &TableShape, project_id: &str) -> Result<()> {
    let name = &shape.name;
    let owner = if name == "projects" {
        shape.position("id")
    } else {
        shape.position("project_id")
    };
    if name == "projects" && table.rows.len() != 1 {
        return Err(malformed("bundle table projects must hold exactly one row"));
    }
    let mut keys = HashSet::new();
    for (n, row) in table.rows.iter().enumerate() {
        if row.len() != shape.columns.len() {
            return Err(malformed(&format!(
                "bundle table {name} row {n} has the wrong width"
            )));
        }
        for (value, column) in row.iter().zip(&shape.columns) {
            let fits = match value {
                Value::Null => !column.not_null,
                Value::String(_) => !column.integer,
                Value::Number(v) => column.integer && v.is_i64(),
                _ => false,
            };
            if !fits {
                let column = &column.name;
                return Err(malformed(&format!(
                    "bundle table {name} row {n} column {column} has the wrong type or is null"
                )));
            }
        }
        if owner
            .and_then(|i| row.get(i))
            .is_some_and(|v| v.as_str() != Some(project_id))
        {
            return Err(malformed(&format!(
                "bundle table {name} row {n} belongs to another project"
            )));
        }
        if !keys.insert(tuple(row, shape, &shape.key)) {
            return Err(malformed(&format!(
                "bundle table {name} row {n} repeats a key"
            )));
        }
        let edge = (shape.position("blocker_id"), shape.position("blocked_id"));
        if let (Some(a), Some(b)) = edge
            && row.get(a) == row.get(b)
        {
            return Err(malformed(&format!(
                "bundle table {name} row {n} blocks itself"
            )));
        }
    }
    Ok(())
}

/// Every non-null reference names a row of the bundle.
fn check_references(tables: &[TableRows], shapes: &[TableShape]) -> Result<()> {
    let mut targets: HashMap<(String, Vec<String>), HashSet<String>> = HashMap::new();
    for (table, shape) in tables.iter().zip(shapes) {
        for reference in &shape.references {
            let wanted = (reference.table.clone(), reference.target.clone());
            let held = match targets.entry(wanted) {
                Entry::Occupied(held) => held.into_mut(),
                Entry::Vacant(slot) => slot.insert(held_keys(
                    tables,
                    shapes,
                    &reference.table,
                    &reference.target,
                )?),
            };
            for (n, row) in table.rows.iter().enumerate() {
                let set = reference.columns.iter().all(|c| {
                    shape
                        .position(c)
                        .and_then(|i| row.get(i))
                        .is_some_and(|v| !v.is_null())
                });
                if set && !held.contains(&tuple(row, shape, &reference.columns)) {
                    let (name, column) = (&shape.name, reference.columns.join(", "));
                    return Err(malformed(&format!(
                        "bundle table {name} row {n} ({column}) names a row the bundle does not hold"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// The `columns` tuples of every row of `table` in the bundle.
fn held_keys(
    tables: &[TableRows],
    shapes: &[TableShape],
    table: &str,
    columns: &[String],
) -> Result<HashSet<String>> {
    let found = tables.iter().zip(shapes).find(|(_, s)| s.name == table);
    let Some((rows, shape)) = found else {
        return Err(malformed(&format!(
            "bundle references table {table}, which it cannot carry"
        )));
    };
    Ok(rows
        .rows
        .iter()
        .map(|row| tuple(row, shape, columns))
        .collect())
}

/// `row`'s values at `columns` as one comparable string, each value in its
/// JSON form (a missing column as `null`).
fn tuple(row: &[Value], shape: &TableShape, columns: &[String]) -> String {
    let parts: Vec<String> = columns
        .iter()
        .map(|c| {
            shape
                .position(c)
                .and_then(|i| row.get(i))
                .map_or_else(|| "null".to_string(), Value::to_string)
        })
        .collect();
    parts.join("\u{1f}")
}

/// A `schema_mismatch` refusal.
fn mismatch(message: &str) -> Error {
    refusal("schema_mismatch", message)
}

/// A `malformed` refusal.
fn malformed(message: &str) -> Error {
    refusal("malformed", message)
}

#[cfg(test)]
#[path = "tests/bundle_check.rs"]
mod tests;
