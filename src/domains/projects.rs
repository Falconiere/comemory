//! The projects capability: engine-owned project management (epic #261).
//! A project is a charter — outcome, success criteria, constraints,
//! non-goals and the repositories it may use — with a lifecycle status, a
//! health and an append-only activity log, ported from the comemory.io
//! platform's `/v1/projects` contract.
//!
//! Every command core here is the one implementation the CLI, the loopback
//! HTTP server and the MCP catalog call. All SQL stays in `store`; this
//! capability never imports a delivery adapter.

/// The epoch-millisecond `<ms>:<uuid>` keyset cursor every project page uses.
pub mod keyset;
/// The platform's charter and paging caps and their `422` refusals.
pub mod limits;
/// The actor a project command runs as.
pub mod principal;
/// Slug derivation from a charter's name.
pub mod slug;
/// ISO-8601 rendering and `targetDate` parsing for epoch-millisecond columns.
pub mod timestamp;
