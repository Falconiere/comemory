//! `project export` (#342): one project, with every carried row it owns, as a
//! canonical [`Bundle`]. It reads inside one transaction so the bundle is a
//! consistent snapshot, and writes nothing but its `activity_log` telemetry.
//! Command receipts and the transfer binding stay behind (see
//! `store::schema_projects::transfer_class`).

use std::time::Instant;

use crate::domains::projects::authority::{Actor, Command, Verb, sealed};
use crate::domains::projects::bundle::{self, Bundle, TableRows};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::project_table_shape::carried;
use crate::store::{project_read, project_transfer};
use crate::utilities::activity::{Outcome, command, record_in};
use crate::utilities::context::Ctx;
use crate::utilities::project_error::ProjectError;
use crate::utilities::uuid;

/// `project export` request.
#[derive(Debug, Clone)]
pub struct Request {
    /// The project's UUID.
    pub id: String,
}

impl sealed::Sealed for Request {}

impl Command for Request {
    type Response = Bundle;

    fn verb(&self) -> Verb {
        Verb::ProjectExport
    }

    /// Export the project `self.id` names, then record the telemetry row.
    fn execute(self, ctx: &mut Ctx<'_>, _actor: &Actor) -> Result<Bundle> {
        let started = Instant::now();
        let result = export(ctx, &self);
        let summary = result
            .as_ref()
            .map(|b| serde_json::json!({"id": b.project_id, "digest": b.digest}));
        let outcome = match &summary {
            Ok(value) => Outcome::Ok(value),
            Err(e) => Outcome::Failed(e),
        };
        record_in(ctx, command::PROJECT_EXPORT, started, &outcome, None);
        result
    }
}

/// Resolve the id as `project show` does, then snapshot it.
fn export(ctx: &mut Ctx<'_>, req: &Request) -> Result<Bundle> {
    let id = uuid::canonical(&req.id)
        .ok_or_else(|| Error::from(ProjectError::invalid_field("projectId", "invalid")))?;
    // A deferred transaction: one snapshot across every table, no writer lock.
    let tx = ctx.conn()?.transaction()?;
    if project_read::project(&tx, &id)?.is_none() {
        return Err(ProjectError::ProjectNotFound {
            project_id: req.id.clone(),
        }
        .into());
    }
    let bundle = snapshot(&tx, &id)?;
    tx.commit()?;
    Ok(bundle)
}

/// The canonical bundle of project `id` as `conn` holds it now. Import calls
/// it inside its own write transaction to compare an existing copy.
pub fn snapshot(conn: &Connection, id: &str) -> Result<Bundle> {
    let shapes = carried();
    let tables = shapes
        .iter()
        .map(|shape| {
            Ok(TableRows {
                table: shape.name.clone(),
                columns: shape.columns.iter().map(|c| c.name.clone()).collect(),
                rows: project_transfer::rows(conn, shape, id)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    bundle::seal(id, tables, &shapes)
}

#[cfg(test)]
#[path = "tests/export.rs"]
mod tests;
