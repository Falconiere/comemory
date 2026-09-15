//! `api::sync` — shared middle of `GET /sync/{changes,manifest}` and
//! `POST /sync/import` (memory-sync design spec, 2026-09-02), plus the
//! code-index half, `GET /sync/code/manifest` and `POST /sync/code/import`
//! (code-graph sync design, 2026-09-15).

pub mod changes;
pub mod code_import;
pub(crate) mod code_import_rules;
pub(crate) mod code_import_write;
pub mod code_manifest;
pub mod code_types;
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

pub use code_types::{
    CoChangeWire, CodeFileRef, CodeFileWire, CodeImportRejection, CodeImportRequest,
    CodeImportResponse, CodeManifestResponse, CodeSymbolWire,
};
pub use types::{
    ChangesResponse, ImportEntry, ImportItemResult, ImportRequest, ImportResponse, ImportStatus,
    ManifestResponse, SyncEntry, SyncOp, SyncRecord, SyncVector, WireFrontmatter,
};
