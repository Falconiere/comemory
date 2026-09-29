//! The idempotent-command runner every project mutation goes through (#327),
//! ported from the platform's `project-command-receipt-service.ts`: the state
//! change, its activity event and its `project_command_receipts` row commit
//! in one immediate transaction. Scoped to principal and key; an exact
//! replay returns the stored response and writes nothing, any other reuse is
//! `409 idempotency_conflict`, a failure writes no receipt, and receipts live
//! until their project is hard-deleted. The receipt is read inside the
//! transaction, so SQLite's writer lock serializes two processes racing a key.

use std::time::Instant;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::domains::projects::authority::Actor;
use crate::domains::projects::limits;
use crate::domains::projects::timestamp::now_ms;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::connection::write_transaction;
use crate::store::project_receipts::{self, NewReceipt, ReceiptRow};
use crate::utilities::activity::{Outcome, record_in};
use crate::utilities::canonical_json;
use crate::utilities::context::Ctx;
use crate::utilities::project_error::ProjectError;
use crate::utilities::uuid;

/// The platform's `PROJECT_IDEMPOTENCY_KEY_MAX`, in UTF-16 code units.
pub const IDEMPOTENCY_KEY_MAX: usize = 200;

/// One command's idempotency identity: its checked key, its type and the
/// digest of its body. Only [`Keyed::new`] builds one, so every key reaching
/// the store has passed the length check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keyed {
    key: String,
    command_type: &'static str,
    digest: String,
}

impl Keyed {
    /// Check `key` (1–200 UTF-16 units, else `422 invalid_request` on
    /// `idempotencyKey`) and digest `body` under `command_type`. `body` is the
    /// validated request minus the key and other bookkeeping.
    pub fn new(key: &str, command_type: &'static str, body: &Value) -> Result<Self> {
        limits::text("idempotencyKey", key, 1, IDEMPOTENCY_KEY_MAX)?;
        Ok(Self {
            key: key.to_string(),
            command_type,
            digest: digest(command_type, body)?,
        })
    }
}

/// What a first run wrote: its response and the project it touched.
#[derive(Debug)]
pub struct Applied<T> {
    /// The adapter-independent core response, stored for replay.
    pub response: T,
    /// The receipt's owning project.
    pub project_id: String,
}

/// How [`run`] answered.
#[derive(Debug, PartialEq, Eq)]
pub enum Ran<T> {
    /// The command ran and its receipt was written.
    Applied(T),
    /// A receipt already answered it; nothing ran.
    Replayed(T),
}

/// Hex SHA-256 of `{"commandType": command_type, "body": body}` with every
/// object key sorted, the platform's `digestCommandBody`.
pub fn digest(command_type: &str, body: &Value) -> Result<String> {
    let value = json!({"commandType": command_type, "body": body});
    Ok(canonical_json::bytes_and_digest(&value)?.1)
}

/// Run `apply` once per `(actor, key)`. A first run commits `apply`'s writes
/// and the receipt together; a replay returns the stored response and
/// writes nothing; a reuse for another command type or body is refused
/// before `apply` is called. An error from `apply` rolls everything back.
pub fn run<T, F>(conn: &mut Connection, actor: &Actor, keyed: &Keyed, apply: F) -> Result<Ran<T>>
where
    T: Serialize + DeserializeOwned,
    F: FnOnce(&Connection) -> Result<Applied<T>>,
{
    let principal = actor.principal();
    let principal_type = principal.principal_type.as_str();
    let tx = write_transaction(conn)?;
    if let Some(existing) = project_receipts::find(&tx, principal_type, &principal.id, &keyed.key)?
    {
        // Dropping `tx` rolls back a transaction that wrote nothing.
        return replay(&existing, keyed).map(Ran::Replayed);
    }
    let applied = apply(&tx)?;
    let response = serde_json::to_string(&applied.response)?;
    project_receipts::insert(
        &tx,
        &NewReceipt {
            id: &uuid::new_v4()?,
            project_id: &applied.project_id,
            principal_type,
            principal_id: &principal.id,
            idempotency_key: &keyed.key,
            command_type: keyed.command_type,
            request_digest: &keyed.digest,
            response: &response,
            at_ms: now_ms(),
        },
    )?;
    tx.commit()?;
    Ok(Ran::Applied(applied.response))
}

/// The stored response when `existing` answers exactly `keyed`, else
/// `idempotency_conflict`. A stored response that no longer parses is a
/// server-side data problem, never a caller error.
fn replay<T: DeserializeOwned>(existing: &ReceiptRow, keyed: &Keyed) -> Result<T> {
    if existing.command_type != keyed.command_type || existing.request_digest != keyed.digest {
        return Err(ProjectError::IdempotencyConflict.into());
    }
    serde_json::from_str(&existing.response).map_err(|e| {
        ProjectError::Invariant {
            invariant: "project_command_receipt".to_string(),
            message: format!(
                "project command receipt {} does not parse: {e}",
                existing.id
            ),
        }
        .into()
    })
}

/// Record the `activity_log` telemetry row for an applied or failed command
/// under `command`, summarized by `summary`, and return its response. A
/// replay records nothing: it ran nothing.
pub fn record<T>(
    ctx: &mut Ctx<'_>,
    command: &'static str,
    started: Instant,
    ran: Result<Ran<T>>,
    summary: impl FnOnce(&T) -> Value,
) -> Result<T> {
    let result = match ran {
        Ok(Ran::Replayed(response)) => return Ok(response),
        Ok(Ran::Applied(response)) => Ok(response),
        Err(e) => Err(e),
    };
    let summarized = result.as_ref().map(summary);
    let outcome = match &summarized {
        Ok(value) => Outcome::Ok(value),
        Err(e) => Outcome::Failed(e),
    };
    record_in(ctx, command, started, &outcome, None);
    result
}

#[cfg(test)]
#[path = "tests/receipt.rs"]
mod tests;
