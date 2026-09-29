//! A project's transfer binding (#342): the one transfer it took part in —
//! direction, the other side, the effective digest, when, and the actor a
//! remap replaced. `project show` displays it; the warning a later local
//! mutation carries is [`super::local_only`].

use serde::Serialize;

use crate::domains::projects::principal::Principal;
use crate::domains::projects::timestamp::iso;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::project_binding::{self, BindingRow};
use crate::utilities::project_error::ProjectError;

/// Which way the copy went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// This data directory received the copy from the remote.
    Imported,
    /// This data directory sent the copy to the remote (#345).
    Exported,
}

impl Direction {
    /// The stored column value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Imported => "imported",
            Self::Exported => "exported",
        }
    }
}

/// One transfer to record.
pub struct NewBinding<'a> {
    /// The bound project.
    pub project_id: &'a str,
    /// Which way the copy went.
    pub direction: Direction,
    /// The other side: a workspace id, or a local label.
    pub remote: &'a str,
    /// The effective digest — the transferred rows after any actor remap:
    /// on an import, what the local copy hashes to; on an export (#345), what
    /// the remote stored (its `Response.digest`). A repeat compares against it.
    pub digest: &'a str,
    /// The principal an actor remap replaced, when one ran.
    pub remapped_from: Option<&'a Principal>,
    /// Epoch milliseconds.
    pub at_ms: i64,
}

/// A binding as `project show` renders it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TransferView {
    /// `imported` or `exported`.
    pub direction: String,
    /// The other side.
    pub remote: String,
    /// The effective digest: the transferred rows after any actor remap.
    pub digest: String,
    /// ISO-8601.
    pub transferred_at: String,
    /// `<type>:<id>` of the actor a remap replaced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remapped_from: Option<String>,
}

/// Record `binding`, replacing any earlier one of the same project.
pub fn record(conn: &Connection, binding: &NewBinding<'_>) -> Result<()> {
    let remapped_from = binding
        .remapped_from
        .map(|p| (p.principal_type.as_str().to_string(), p.id.clone()));
    project_binding::upsert(
        conn,
        &BindingRow {
            project_id: binding.project_id.to_string(),
            direction: binding.direction.as_str().to_string(),
            remote: binding.remote.to_string(),
            digest: binding.digest.to_string(),
            remapped_from,
            transferred_at: binding.at_ms,
        },
    )
}

/// The binding of `project_id` as its view, when it has one.
pub fn find(conn: &Connection, project_id: &str) -> Result<Option<TransferView>> {
    let Some(row) = project_binding::find(conn, project_id)? else {
        return Ok(None);
    };
    let transferred_at = iso(row.transferred_at).ok_or_else(|| ProjectError::Invariant {
        invariant: "project_timestamp_range".to_string(),
        message: format!(
            "project_transfer_bindings.transferred_at of {project_id} is outside the representable range"
        ),
    })?;
    Ok(Some(TransferView {
        direction: row.direction,
        remote: row.remote,
        digest: row.digest,
        transferred_at,
        remapped_from: row.remapped_from.map(|(kind, id)| format!("{kind}:{id}")),
    }))
}
