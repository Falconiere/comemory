//! `replica_device` read — the id of the machine this database belongs to.
//!
//! Minted once by migration 0026 itself; every shared verdict and activity
//! event carries it as its origin.

use rusqlite::Connection;

use super::orm;
use super::schema_replica_device::{ReplicaDevice, replica_device as col};
use crate::prelude::*;

/// This database's device id.
///
/// # Errors
/// Returns [`Error::Other`] when the row migration 0026 mints is missing —
/// an event recorded without an origin could be attributed to any machine
/// that later received it.
pub fn id(conn: &Connection) -> Result<String> {
    let device: Option<String> = orm::query_optional(
        conn,
        ReplicaDevice::select()
            .columns_typed(&[&col::device_id])
            .to_sql(),
        |r| r.get(0),
    )?;
    device.ok_or_else(|| Error::Other("replica_device has no device row".to_string()))
}

#[cfg(test)]
#[path = "tests/replica_device.rs"]
mod tests;
