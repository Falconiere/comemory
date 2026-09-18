//! The live `content_hash` set behind `GET /sync/manifest`.
//!
//! One read, named for the concern that needs it, in the same shape as
//! `doctor_probes`'s live-memory probe: the sync protocol asks the store for
//! rows, and `domains::sync::exchange::manifest` does the bucketing and
//! hashing. Keeping the SQL here is Binding Rule 10 — the store is the single
//! home of every SQL string in the crate.

use rusqlite::Connection;

use super::{
    orm,
    schema_memory::{Memories, memories as col},
};
use crate::prelude::*;
use toolu_orm::core::query_column::CommonOps;

/// Every live memory's `content_hash`, ordered, oldest lexicographic first.
///
/// Soft-deleted rows are excluded: a tombstone has no content to digest, and
/// including one would make two peers that agree on the live corpus report
/// different bucket digests. The order is fixed in SQL so the caller's digest
/// is reproducible without sorting the whole set first.
pub fn live_content_hashes(conn: &Connection) -> Result<Vec<String>> {
    orm::query_all(
        conn,
        Memories::select()
            .columns_typed(&[&col::content_hash])
            .filter(col::deleted_at.is_null())
            .order_by(col::content_hash.asc())
            .to_sql(),
        |r| r.get(0),
    )
}
