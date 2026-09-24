//! `repository_approval` row CRUD — which repository labels are approved, and
//! what each is called upstream, readable without the network.
//!
//! The platform's allowlist is only visible inside a policy fetch, and a local
//! `comemory index` run has neither auth nor a connection. A shared document's
//! identity is a digest over the CANONICAL repository, so a capture that
//! guessed the name would mint an id no other machine computes. This table
//! carries whatever the last policy load resolved.
//!
//! A label with no row is withheld, not assumed: before the first policy load
//! nothing is approved, which is the right answer for a machine that has not
//! joined a workspace.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_sync::{RepositoryApproval, repository_approval as col};
use crate::prelude::*;
use crate::utilities::repo_label;

/// Replace the whole map with what one policy load resolved.
///
/// Wholesale rather than merged: a repository whose approval was REVOKED has
/// no row in the new set, and a merge would leave its stale row behind saying
/// it is still shareable. The caller owns the transaction, so a failure
/// half-way leaves the previous map intact rather than an empty one.
///
/// `resolved` is `(label, canonical)`. Labels are normalized here so a caller
/// cannot key a row by something a reader will not find, and a label that
/// normalizes to nothing is dropped rather than stored under an empty key
/// [`canonical_for`] would never ask for.
///
/// # Errors
/// Propagates SQLite failures.
pub fn replace_all(tx: &Connection, resolved: &[(String, String)], at: &str) -> Result<()> {
    orm::execute(tx, RepositoryApproval::delete().to_sql())?;
    for (label, canonical) in resolved {
        let label = repo_label::normalize(label);
        if label.is_empty() {
            continue;
        }
        orm::execute(
            tx,
            RepositoryApproval::insert()
                .set(&col::label, label.as_str())
                .set(&col::canonical, canonical.as_str())
                .set(&col::updated_at, at)
                .to_sql(),
        )?;
    }
    Ok(())
}

/// How many labels the map holds — what a caller needs to tell "no policy has
/// loaded" from "this one label is not approved".
///
/// # Errors
/// Propagates SQLite failures.
pub fn len(conn: &Connection) -> Result<i64> {
    let mut statement = conn.prepare("SELECT COUNT(*) FROM repository_approval")?;
    let count = statement.query_row([], |r| r.get(0))?;
    Ok(count)
}

/// The canonical repository `label` resolves to, or `None` when it is not
/// approved — which is also the answer before any policy has been loaded.
///
/// # Errors
/// Propagates SQLite failures.
pub fn canonical_for(conn: &Connection, label: &str) -> Result<Option<String>> {
    let normalized = repo_label::normalize(label);
    if normalized.is_empty() {
        return Ok(None);
    }
    orm::query_optional(
        conn,
        RepositoryApproval::select()
            .columns_typed(&[&col::canonical])
            .filter(col::label.eq(normalized.as_str()))
            .to_sql(),
        |r| r.get(0),
    )
}

/// The label this machine files `canonical` under, or `None` when no approved
/// label resolves to it. When several do, the smallest wins, so every call
/// answers the same one.
///
/// # Errors
/// Propagates SQLite failures.
pub fn label_for(conn: &Connection, canonical: &str) -> Result<Option<String>> {
    let labels: Vec<String> = orm::query_all(
        conn,
        RepositoryApproval::select()
            .columns_typed(&[&col::label])
            .filter(col::canonical.eq(canonical))
            .to_sql(),
        |r| r.get(0),
    )?;
    Ok(labels.into_iter().min())
}

#[cfg(test)]
#[path = "tests/repository_approval.rs"]
mod tests;
