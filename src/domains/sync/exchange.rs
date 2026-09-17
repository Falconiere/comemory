//! `domains::sync::exchange` — the server side of the platform protocol:
//! `GET /sync/{changes,manifest}` and `POST /sync/import` (memory-sync design
//! spec, 2026-09-02), plus the code-index half, `GET /sync/code/manifest` and
//! `POST /sync/code/import` (code-graph sync design, 2026-09-15).
//!
//! The wire models here are shared with the client half beside it
//! ([`client`](crate::domains::sync::client),
//! [`client_code`](crate::domains::sync::client_code) and
//! [`code_plan`](crate::domains::sync::code_plan)); the
//! `code_*` modules here are the *server's* code import, distinct from the
//! client's `code.rs`/`client_code.rs` push.

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
#[path = "exchange/tests/changes.rs"]
mod tests_changes;

#[cfg(test)]
#[path = "exchange/tests/import.rs"]
mod tests_import;

pub use code_types::{
    CoChangeWire, CodeFileRef, CodeFileWire, CodeImportRejection, CodeImportRequest,
    CodeImportResponse, CodeManifestResponse, CodeSymbolWire,
};
pub use types::{
    ChangesResponse, ImportEntry, ImportItemResult, ImportRequest, ImportResponse, ImportStatus,
    ManifestResponse, SyncEntry, SyncOp, SyncRecord, SyncVector, WireFrontmatter,
};
