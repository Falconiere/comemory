//! The `replica-v1` operation id: `op-<yyyymmdd>-<32 hex>`.
//!
//! An operation id is minted by the device that made the mutation and becomes
//! the key of the upstream feed row that accepts it, so it must be unique
//! across every device of a workspace, not just this one. 128 bits of digest
//! over the entity, the operation, the clock, this process and a per-process
//! counter makes a collision implausible at any realistic volume, where the
//! 32 bits a `dated_id` carries collide within a day at tens of thousands of
//! operations. The date prefix keeps ids day-sortable for an operator.

use std::sync::atomic::{AtomicU64, Ordering};

use time::OffsetDateTime;

use crate::utilities::digest::sha256_hex;

/// Mutations minted by this process so far — two mints in the same clock tick
/// still differ.
static MINTED: AtomicU64 = AtomicU64::new(0);

/// Mint an id for one mutation of `entity_kind`/`entity_key` doing `op`.
#[must_use]
pub(crate) fn mint(entity_kind: &str, entity_key: &str, op: &str) -> String {
    let now = OffsetDateTime::now_utc();
    let seed = format!(
        "{entity_kind}:{entity_key}:{op}:{}:{}:{}",
        now.unix_timestamp_nanos(),
        std::process::id(),
        MINTED.fetch_add(1, Ordering::Relaxed)
    );
    let hex = sha256_hex(seed.as_bytes());
    format!(
        "op-{:04}{:02}{:02}-{}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        &hex[..32]
    )
}

#[cfg(test)]
#[path = "tests/operation_id.rs"]
mod tests;
