//! `api::sync` — shared middle of `GET /sync/{changes,manifest}` and
//! `POST /sync/import` (memory-sync design spec, 2026-09-02).

pub mod changes;
pub mod import;
pub(crate) mod import_rules;
pub(crate) mod import_state;
pub(crate) mod import_write;
pub mod manifest;
pub mod types;

#[cfg(test)]
#[path = "sync/tests/changes.rs"]
mod tests_changes;

#[cfg(test)]
#[path = "sync/tests/import.rs"]
mod tests_import;

pub use types::{
    ChangesResponse, ImportEntry, ImportItemResult, ImportRequest, ImportResponse, ImportStatus,
    ManifestResponse, SyncEntry, SyncOp, SyncRecord, SyncVector, WireFrontmatter,
};
