//! Declared schema — `replica_device`, the single-row identity of the machine
//! this database belongs to (#254).
//!
//! Kept apart from `replica_stream` on purpose: a stream's epoch changes when
//! its state is replaced or restored, while the device that recorded an event
//! does not. Every shared verdict and activity event carries this id, so an
//! imported event is attributed to the machine that recorded it.

use toolu_orm::core::column::{Integer, Text};
use toolu_orm::table;

/// `replica_device`: this database's device id, minted once by migration
/// 0026.
#[table(name = "replica_device")]
pub struct ReplicaDevice {
    /// Always `1` — one device per database.
    #[column(primary_key, check = "id = 1")]
    pub id: Integer,
    /// 32 lowercase hex characters.
    #[column(not_null)]
    pub device_id: Text,
    /// RFC3339 time the id was minted.
    #[column(not_null)]
    pub created_at: Text,
}
