//! The outbox's classification writers — why a pending row is not sent now,
//! which session key it belongs to, and the wire fields its first send fixed —
//! plus the per-state counts the status report shows.
//!
//! A hold never changes a row's `state`: a held row is still `pending`, which
//! is what keeps it protecting the local edit it describes and what keeps it
//! from ever reading as synchronized.

use rusqlite::Connection;
use toolu_orm::core::expr::Expr;
use toolu_orm::core::query_column::CommonOps;
use toolu_orm::core::value::Value;

use super::orm;
use super::schema_replica::{ReplicaOperation, replica_operation as col};
use super::sync_exchange::ExchangeKey;
use crate::prelude::*;

/// Why a pending row is not sent now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Hold {
    /// Its repository has no approved canonical name under the key's policy.
    Policy,
    /// Its body matches a secret rule without an override.
    Secret,
    /// Its label matches `[sync] skip_repos`.
    SkipRepos,
    /// It, or its entity, belongs to another session key.
    Workspace,
    /// The upstream does not advertise its kind at its schema version.
    Incompatible,
    /// An earlier row for the same entity is held, or a restore's tombstone
    /// position is not known yet.
    Order,
    /// It was made before this key selected `replica-v1` and waits for the
    /// pull to reach the upgrade horizon.
    Upgrade,
}

impl Hold {
    /// Every hold, in status order.
    pub const ALL: [Self; 7] = [
        Self::Policy,
        Self::Secret,
        Self::SkipRepos,
        Self::Workspace,
        Self::Incompatible,
        Self::Order,
        Self::Upgrade,
    ];

    /// The stored literal.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Policy => "policy",
            Self::Secret => "secret",
            Self::SkipRepos => "skip_repos",
            Self::Workspace => "workspace",
            Self::Incompatible => "incompatible",
            Self::Order => "order",
            Self::Upgrade => "upgrade",
        }
    }

    /// Read a stored literal back.
    ///
    /// # Errors
    /// [`Error::Other`] for a literal no writer produces.
    pub fn parse(raw: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|hold| hold.as_str() == raw)
            .ok_or_else(|| Error::Other(format!("unknown outbox hold: {raw}")))
    }
}

/// Which outbox rows an [`update`] touches.
#[derive(Debug, Clone, Copy)]
pub enum Target<'a> {
    /// One operation by id.
    Operation(&'a str),
    /// Every pending row no send has stamped yet, created at or before the
    /// given time (every one, with `None`) — what a logout, a login to another
    /// key, or a replaced credential does for the key being left.
    Unstamped(Option<&'a str>),
}

/// What an [`update`] writes.
#[derive(Debug, Clone, Copy)]
pub enum Change<'a> {
    /// Set, or clear with `None`, the hold and its detail.
    Hold(Option<(Hold, &'a str)>),
    /// Stamp the key the rows belong to.
    Stamp(&'a ExchangeKey),
    /// Persist the wire fields a first send resolved, so every retry carries
    /// the same values.
    Wire {
        /// Canonical repository.
        repository: Option<&'a str>,
        /// For a restore, the tombstone position it names.
        observed_sequence: Option<i64>,
    },
}

/// Apply `change` to the rows `target` names; returns how many changed.
///
/// One writer for every classification change: they differ only in which
/// columns they set and which rows they touch.
///
/// # Errors
/// Propagates SQLite failures.
pub fn update(
    conn: &Connection,
    target: Target<'_>,
    change: Change<'_>,
    at: &str,
) -> Result<usize> {
    let update = ReplicaOperation::update().set(&col::updated_at, at);
    let update = match change {
        Change::Hold(hold) => update
            .set(&col::hold_reason, hold.map(|(h, _)| h.as_str()))
            .set(&col::hold_detail, hold.map(|(_, detail)| detail)),
        Change::Stamp(key) => update
            .set(&col::api_url, key.api_url.as_str())
            .set(&col::workspace_id, key.workspace_id.as_str()),
        Change::Wire {
            repository,
            observed_sequence,
        } => update
            .set(&col::wire_repository, repository)
            .set(&col::observed_sequence, observed_sequence),
    };
    let update = match target {
        Target::Operation(operation_id) => update.filter(col::operation_id.eq(operation_id)),
        Target::Unstamped(through) => {
            let unstamped = update
                .filter(col::state.eq("pending"))
                .filter(col::api_url.is_null());
            match through {
                // RFC3339 strings order chronologically; the ORM has no `<=`
                // on text.
                Some(through) => {
                    unstamped.filter(Expr::raw("\"created_at\" <= ?", vec![Value::from(through)]))
                }
                None => unstamped,
            }
        }
    };
    orm::execute(conn, update.to_sql())
}

/// What the outbox holds, by state — the status report's `outbox` block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutboxCounts {
    /// Pending rows with no hold that no send has attempted yet.
    pub pending: i64,
    /// Pending rows with no hold that a send attempted without an answer.
    pub retryable: i64,
    /// Pending rows per hold, in [`Hold::ALL`] order.
    pub held: Vec<(Hold, i64)>,
    /// Rows the upstream refused.
    pub rejected: i64,
}

/// Count the outbox by state; every pending row lands in exactly one count.
///
/// # Errors
/// Propagates SQLite failures and a stored hold literal no writer produces.
pub fn counts(conn: &Connection) -> Result<OutboxCounts> {
    let rows: Vec<(String, Option<String>, i64)> = orm::query_all(
        conn,
        ReplicaOperation::select()
            .columns_typed(&[&col::state, &col::hold_reason, &col::attempts])
            .filter(col::state.ne("accepted"))
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let mut counts = OutboxCounts {
        held: Hold::ALL.iter().map(|h| (*h, 0)).collect(),
        ..OutboxCounts::default()
    };
    for (state, hold, attempts) in rows {
        match (state.as_str(), hold) {
            ("rejected", _) => counts.rejected += 1,
            (_, Some(raw)) => {
                let hold = Hold::parse(&raw)?;
                if let Some((_, n)) = counts.held.iter_mut().find(|(h, _)| *h == hold) {
                    *n += 1;
                }
            }
            (_, None) if attempts > 0 => counts.retryable += 1,
            (_, None) => counts.pending += 1,
        }
    }
    Ok(counts)
}

#[cfg(test)]
#[path = "tests/replica_outbox_hold.rs"]
mod tests;
