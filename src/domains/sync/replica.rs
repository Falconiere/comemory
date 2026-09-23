//! `replica-v1` — the versioned journal protocol every replicated entity
//! kind shares, and the engine halves that serve it.
//!
//! The legacy memory-only exchange ([`super::exchange`]) keeps its wire shape
//! and its routes; both write the same journal, so a legacy client and a
//! replica client converge on one state.

/// The shared test fixture — a real data directory, a real migrated database
/// and real saved memories — declared once here and used by every `tests/`
/// module in this folder, so the file is loaded as one module.
#[cfg(test)]
#[path = "replica/tests/support.rs"]
pub(crate) mod test_support;

/// The acceptance path: decide, materialize, journal, receipt.
pub mod accept;
/// Journal seeding for memories that predate the journal.
pub mod bootstrap;
/// `GET /sync/replica/changes` — the ordered page above a cursor.
pub mod changes;
/// The write half for a code generation: record, project, activate, journal
/// and receipt, all in one transaction.
pub mod code_accept;
/// The wire contract: envelopes, operations and dispositions.
pub mod contract;
/// The read and staging shapes: changes, manifest, stage and activate.
pub mod contract_views;
/// Acceptance for a pulled document revision: one transaction, no local row.
pub mod document_accept;
/// `GET /sync/replica/events` — notification-only frames.
pub mod events;
/// `GET /sync/replica/manifest` — holdings, capability and seeding progress.
pub mod manifest;
/// The write half of acceptance — markdown, mirror, journal and receipt.
pub mod materialize;
/// Staged parts of an oversized revision and their activation.
pub mod staging;
/// The acceptance decision, made before any state moves.
pub mod validate;
