//! The warning a local mutation of a transferred project carries (#342): the
//! epic rejects a mesh, so a change to a bound project stays in this data
//! directory and will not reach the other side. The shared activity writer
//! looks it up for every mutation; it never refuses one.

use serde::{Deserialize, Serialize};

use crate::prelude::*;
use crate::store::Connection;
use crate::store::project_binding;

/// The warning a mutation of a bound project carries: it never refuses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalOnly {
    /// Always `local_only`.
    pub code: String,
    /// The bound project.
    pub project_id: String,
    /// `imported` or `exported`.
    pub direction: String,
    /// The side the change will not reach.
    pub remote: String,
    /// One sentence for a person.
    pub message: String,
}

/// The warning for a mutation of `project_id`, when the project is bound.
/// It reads the stored row directly — never rendering `transferred_at` — so
/// nothing about the binding can fail the mutation it annotates.
pub fn local_only(conn: &Connection, project_id: &str) -> Result<Option<LocalOnly>> {
    Ok(project_binding::find(conn, project_id)?.map(|row| {
        let how = if row.direction == "exported" {
            "was exported to"
        } else {
            "was imported from"
        };
        let remote = row.remote;
        LocalOnly {
            code: "local_only".to_string(),
            project_id: project_id.to_string(),
            direction: row.direction,
            message: format!(
                "This project {how} {remote}; this change stays in this data directory \
                 and will not reach {remote}"
            ),
            remote,
        }
    }))
}

#[cfg(test)]
#[path = "tests/local_only.rs"]
mod tests;
