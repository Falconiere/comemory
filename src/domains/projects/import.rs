//! `project import` (#342): a [`Bundle`] written into this data directory
//! under the same ids, in one immediate transaction.
//!
//! The bundle is checked and its declared digest verified before any store
//! access; the optional actor remap is applied next, and the **effective
//! digest** — what the stored copy will hash to — decides the outcome. The
//! same id with the same digest is `unchanged`, with a different one
//! `skipped`, and neither writes a project row. Otherwise a key prefix or slug
//! held by another project is refused, the rows go in parents first under
//! deferred keys, and the binding and one `changed` feed frame (#324, the
//! project's own id as `event_id`) are recorded. No `project_activity_events`
//! row is written: activity is transferred content, and an import event would
//! make the next identical import diverge.

use std::time::Instant;

use serde::Serialize;
use serde_json::Value;

use crate::domains::projects::authority::{Actor, Command, Verb, sealed};
use crate::domains::projects::binding::{self, Direction, NewBinding};
use crate::domains::projects::bundle::{self, ActorRemap, Bundle};
use crate::domains::projects::bundle_check;
use crate::domains::projects::changes;
use crate::domains::projects::export;
use crate::domains::projects::timestamp::now_ms;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::connection::write_transaction;
use crate::store::project_read;
use crate::store::project_table_shape::{TableShape, carried};
use crate::store::project_transfer::{self, Identity};
use crate::store::project_transfer_write::{self, RowsInsert};
use crate::utilities::activity::{Outcome as Logged, command, record_in};
use crate::utilities::context::Ctx;
use crate::utilities::project_error::{ProjectError, RequestEdge};

/// One import: the bundle, the side it came from, and an optional remap.
pub struct Import {
    /// The parsed bundle.
    pub bundle: Bundle,
    /// The other side, recorded on the binding: a workspace id, or a local
    /// label such as `file:<path>`.
    pub remote: String,
    /// Rewrite one principal as another before anything is compared.
    pub remap: Option<ActorRemap>,
}

/// What the import did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The project was written and bound.
    Imported,
    /// This data directory already holds an identical copy; nothing written.
    Unchanged,
    /// This data directory holds a different copy; nothing written, both kept.
    Skipped,
}

/// The import's answer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    /// What happened.
    pub outcome: Outcome,
    /// The project's id.
    pub project_id: String,
    /// The bundle's effective digest, after any remap.
    pub digest: String,
    /// The digest of the copy already here, when there was one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_digest: Option<String>,
    /// Rows written; `0` unless imported.
    pub rows: usize,
}

impl sealed::Sealed for Import {}

impl Command for Import {
    type Response = Response;

    fn verb(&self) -> Verb {
        Verb::ProjectImport
    }

    /// Import into this data directory, then record the telemetry row. The
    /// actor writes nothing: an import records no activity event, and the
    /// rows keep the principals the bundle carries (after any remap).
    fn execute(self, ctx: &mut Ctx<'_>, _actor: &Actor) -> Result<Response> {
        let started = Instant::now();
        let result = apply(ctx, self);
        let summary = result.as_ref().map(
            |r| serde_json::json!({"id": r.project_id, "outcome": r.outcome, "digest": r.digest}),
        );
        let outcome = match &summary {
            Ok(value) => Logged::Ok(value),
            Err(e) => Logged::Failed(e),
        };
        record_in(ctx, command::PROJECT_IMPORT, started, &outcome, None);
        result
    }
}

/// Check, remap, then compare or write in one immediate transaction.
fn apply(ctx: &mut Ctx<'_>, import: Import) -> Result<Response> {
    let shapes = carried();
    let Import {
        mut bundle,
        remote,
        remap,
    } = import;
    let digest = effective_digest(&mut bundle, &shapes, remap.as_ref())?;
    let id = bundle.project_id.clone();
    let tx = write_transaction(ctx.conn()?)?;
    if project_read::project(&tx, &id)?.is_some() {
        let local = export::snapshot(&tx, &id)?.digest;
        let outcome = if local == digest {
            Outcome::Unchanged
        } else {
            Outcome::Skipped
        };
        return Ok(Response {
            outcome,
            project_id: id,
            digest,
            local_digest: Some(local),
            rows: 0,
        });
    }
    refuse_taken_identity(&tx, &bundle, &shapes)?;
    let rows = write_rows(&tx, &bundle, &shapes)?;
    let at_ms = now_ms();
    let binding = NewBinding {
        project_id: &id,
        direction: Direction::Imported,
        remote: &remote,
        digest: &digest,
        remapped_from: remap.as_ref().map(|r| &r.from),
        at_ms,
    };
    binding::record(&tx, &binding)?;
    // One `changed` feed frame (#324) so a connected console refetches; the
    // feed is outside the transferred tables, so the digest is untouched.
    changes::record_change(&tx, &id, &id, "project", at_ms)?;
    tx.commit()?;
    Ok(Response {
        outcome: Outcome::Imported,
        project_id: id,
        digest,
        local_digest: None,
        rows,
    })
}

/// Check the bundle, verify its declared digest, apply the remap, and return
/// the digest of what will be stored.
fn effective_digest(
    bundle: &mut Bundle,
    shapes: &[TableShape],
    remap: Option<&ActorRemap>,
) -> Result<String> {
    bundle_check::check(bundle, shapes)?;
    bundle::sort_rows(&mut bundle.tables, shapes);
    if bundle::digest(&bundle.tables)? != bundle.digest {
        let refusal = ProjectError::invalid_field("digest", "mismatch").at(
            RequestEdge::Schema,
            Some("bundle digest does not match its rows"),
        );
        return Err(refusal.into());
    }
    if let Some(remap) = remap {
        bundle::remap_actors(&mut bundle.tables, shapes, remap);
    }
    bundle::digest(&bundle.tables)
}

/// Refuse a key prefix or slug another project already holds.
fn refuse_taken_identity(conn: &Connection, bundle: &Bundle, shapes: &[TableShape]) -> Result<()> {
    let (Some(project), Some(shape)) = (bundle.tables.first(), shapes.first()) else {
        return Ok(());
    };
    let row = project.rows.first();
    for (identity, column, field) in [
        (Identity::KeyPrefix, "key_prefix", "keyPrefix"),
        (Identity::Slug, "slug", "slug"),
    ] {
        let value = shape
            .position(column)
            .and_then(|i| row?.get(i))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if project_transfer::identity_holder(conn, identity, value)?.is_some() {
            let message = format!("{field} is already used in this workspace");
            let mut refusal = ProjectError::invalid_field(field, "duplicate")
                .at(RequestEdge::Invariant, Some(&message));
            push_detail(&mut refusal, "value", value);
            return Err(refusal.into());
        }
    }
    Ok(())
}

/// Insert every table's rows parents first under deferred keys, then check
/// the keys of the tables written; returns the rows written.
fn write_rows(conn: &Connection, bundle: &Bundle, shapes: &[TableShape]) -> Result<usize> {
    project_transfer_write::defer_foreign_keys(conn)?;
    let mut written = 0;
    for (table, shape) in bundle.tables.iter().zip(shapes) {
        if project_transfer_write::insert_rows(conn, shape, &table.rows)? == RowsInsert::Conflict {
            return Err(conflict(&shape.name));
        }
        written += table.rows.len();
    }
    let names: Vec<&str> = shapes.iter().map(|s| s.name.as_str()).collect();
    if let Some(table) = project_transfer_write::first_key_violation(conn, &names)? {
        return Err(conflict(&table));
    }
    Ok(written)
}

/// A `422` refusal: a row of `table` collides with this data directory.
fn conflict(table: &str) -> Error {
    let message = format!("bundle rows of {table} collide with rows this data directory holds");
    let mut refusal = ProjectError::invalid_field("bundle", "conflict")
        .at(RequestEdge::Invariant, Some(&message));
    push_detail(&mut refusal, "table", table);
    refusal.into()
}

/// Add `key: value` to an `invalid_request`'s details.
fn push_detail(refusal: &mut ProjectError, key: &'static str, value: &str) {
    if let ProjectError::InvalidRequest { details, .. } = refusal {
        details.push(key, Value::from(value));
    }
}

#[cfg(test)]
#[path = "tests/import.rs"]
mod tests;
