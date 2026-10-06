//! The transfer bundle (#342): one project's carried rows as canonical JSON,
//! its digest, and the actor remap an import may apply. Pure — no store
//! access — so export, import and the network legs (#343–#345) agree on one
//! byte form.
//!
//! **Canonical form.** Every carried table once, parents first, each with its
//! declared columns in declaration order and its rows sorted by primary key
//! in Rust (never by SQLite `ORDER BY`), so two engines holding the same rows
//! produce the same bytes. The digest is `canonical_json` over the `tables`
//! array: "same tasks" means equal digests.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domains::projects::principal::Principal;
use crate::prelude::*;
use crate::store::project_table_shape::TableShape;
use crate::utilities::canonical_json;
use crate::utilities::project_error::{ProjectError, RequestEdge};

/// The bundle's `format` marker.
pub const FORMAT: &str = "comemory.project-bundle";
/// The one bundle version this engine reads and writes.
pub const VERSION: u64 = 1;
/// The refusal message for a document that is not a project bundle.
pub(crate) const NOT_A_BUNDLE: &str = "bundle is not a comemory project bundle";
/// The refusal message for a bundle version this engine cannot read.
pub(crate) const UNSUPPORTED_VERSION: &str =
    "bundle version is not supported by this engine (it reads version 1)";

/// One project, with every row it owns that travels.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Bundle {
    /// Always [`FORMAT`].
    pub format: String,
    /// Always [`VERSION`].
    pub version: u64,
    /// The project's UUID, lowercase.
    pub project_id: String,
    /// 64-hex SHA-256 of the canonical `tables`.
    pub digest: String,
    /// Every carried table, parents first.
    pub tables: Vec<TableRows>,
}

/// One table's rows.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TableRows {
    /// Table name.
    pub table: String,
    /// Declared columns, in declaration order.
    pub columns: Vec<String>,
    /// Each row's values, positional with `columns`.
    pub rows: Vec<Vec<Value>>,
}

/// Rewrite every principal pair equal to `from` as `to`: the hook #345 uses
/// to turn the local operator into the platform user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorRemap {
    /// The principal being replaced; a pair matches when both kind and id do.
    pub from: Principal,
    /// Its replacement.
    pub to: Principal,
}

/// A bundle of `tables`, put in canonical order and digested.
pub fn seal(project_id: &str, mut tables: Vec<TableRows>, shapes: &[TableShape]) -> Result<Bundle> {
    sort_rows(&mut tables, shapes);
    let digest = digest(&tables)?;
    Ok(Bundle {
        format: FORMAT.to_string(),
        version: VERSION,
        project_id: project_id.to_string(),
        digest,
        tables,
    })
}

/// The 64-hex digest of `tables`, which must already be canonical.
pub fn digest(tables: &[TableRows]) -> Result<String> {
    let value = serde_json::to_value(tables)?;
    Ok(canonical_json::bytes_and_digest(&value)?.1)
}

/// Sort each table's rows by its primary key, so the byte form does not
/// depend on the order rows were read or sent in.
pub fn sort_rows(tables: &mut [TableRows], shapes: &[TableShape]) {
    for (table, shape) in shaped(tables, shapes) {
        let key: Vec<usize> = shape
            .key
            .iter()
            .filter_map(|column| shape.position(column))
            .collect();
        table.rows.sort_by(|a, b| {
            key.iter()
                .map(|&i| compare(a.get(i), b.get(i)))
                .find(|o| o.is_ne())
                .unwrap_or(Ordering::Equal)
        });
    }
}

/// One scalar against another: `null` first, then integers by value, then
/// strings by their UTF-8 bytes (SQLite's `BINARY` order).
fn compare(a: Option<&Value>, b: Option<&Value>) -> Ordering {
    let rank = |v: Option<&Value>| match v {
        None | Some(Value::Null) => 0,
        Some(Value::Number(_)) => 1,
        Some(_) => 2,
    };
    match (a, b) {
        (Some(Value::Number(x)), Some(Value::Number(y))) => x
            .as_i64()
            .unwrap_or_default()
            .cmp(&y.as_i64().unwrap_or_default()),
        (Some(Value::String(x)), Some(Value::String(y))) => x.as_bytes().cmp(y.as_bytes()),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// Rewrite, in place, every `<role>_principal_type` / `<role>_principal_id`
/// pair equal to `remap.from` as `remap.to`. JSON bodies and free-text
/// columns (`verified_by`) are never rewritten. No key column is a principal,
/// so the canonical order holds.
pub fn remap_actors(tables: &mut [TableRows], shapes: &[TableShape], remap: &ActorRemap) {
    let from = (remap.from.principal_type.as_str(), remap.from.id.as_str());
    let to = (remap.to.principal_type.as_str(), remap.to.id.as_str());
    for (table, shape) in shaped(tables, shapes) {
        for (kind, id) in principal_pairs(shape) {
            let matching = table.rows.iter_mut().filter(|row| {
                row.get(kind).and_then(Value::as_str) == Some(from.0)
                    && row.get(id).and_then(Value::as_str) == Some(from.1)
            });
            for row in matching {
                set(row, kind, to.0);
                set(row, id, to.1);
            }
        }
    }
}

/// Each table of `tables` paired with its shape; a table no shape names is
/// skipped (the bundle check refuses one before this runs).
fn shaped<'a>(
    tables: &'a mut [TableRows],
    shapes: &'a [TableShape],
) -> impl Iterator<Item = (&'a mut TableRows, &'a TableShape)> {
    tables.iter_mut().filter_map(move |table| {
        let shape = shapes.iter().find(|s| s.name == table.table)?;
        Some((table, shape))
    })
}

/// Put `value` at `position` of `row`, when the row is that wide.
fn set(row: &mut [Value], position: usize, value: &str) {
    if let Some(cell) = row.get_mut(position) {
        *cell = Value::from(value);
    }
}

/// The `(kind, id)` column positions of every principal pair in `shape`.
fn principal_pairs(shape: &TableShape) -> Vec<(usize, usize)> {
    shape
        .columns
        .iter()
        .enumerate()
        .filter_map(|(kind, column)| {
            let role = column.name.strip_suffix("_principal_type")?;
            let id = shape.position(&format!("{role}_principal_id"))?;
            Some((kind, id))
        })
        .collect()
}

/// Decode `bytes` as a bundle: the header first, untyped, so a future version
/// is refused as `unsupported_version` rather than as an unknown field.
pub fn parse(bytes: &[u8]) -> Result<Bundle> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| refusal("invalid", "bundle is not JSON"))?;
    if value.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(refusal("invalid", NOT_A_BUNDLE));
    }
    if value.get("version").and_then(Value::as_u64) != Some(VERSION) {
        return Err(refusal("unsupported_version", UNSUPPORTED_VERSION));
    }
    serde_json::from_value(value)
        .map_err(|_| refusal("invalid", "bundle does not match format version 1"))
}

/// A schema-edge (`400`) `invalid_request` for the bundle, with `reason` and
/// a message naming only schema identifiers.
pub(crate) fn refusal(reason: &str, message: &str) -> Error {
    ProjectError::invalid_field("bundle", reason)
        .at(RequestEdge::Schema, Some(message))
        .into()
}

#[cfg(test)]
#[path = "tests/bundle.rs"]
mod tests;
