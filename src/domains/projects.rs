//! The projects capability: engine-owned project management (epic #261).
//! A project is a charter — outcome, success criteria, constraints,
//! non-goals and the repositories it may use — with a lifecycle status, a
//! health and an append-only activity log, ported from the comemory.io
//! platform's `/v1/projects` contract.
//!
//! Every command core here is the one implementation the CLI, the loopback
//! HTTP server and the MCP catalog call. All SQL stays in `store`; this
//! capability never imports a delivery adapter.

/// The one `project_activity_events` writer every mutation shares.
pub mod activity;
/// `project activity`: a keyset page of one project's activity events.
pub mod activity_page;
/// The capability envelope every core runs under, and the verb table.
pub mod authority;
/// `project changes`: the body-free change feed and its writers.
pub mod changes;
/// A create request checked against every charter rule.
pub mod charter;
/// `project create`: a draft charter and its `project.created` event.
pub mod create;
/// The epoch-millisecond `<ms>:<uuid>` keyset cursor every project page uses.
pub mod keyset;
/// `project archive|restore|pause|resume`: the four lifecycle commands.
pub mod lifecycle;
/// The platform's charter and paging caps and their `422` refusals.
pub mod limits;
/// `project list`: a keyset page of charters.
pub mod list;
/// `project plan show`: the committed plan at the current version.
pub mod plan;
/// The principal kinds and ids an envelope carries.
pub mod principal;
/// The idempotent-command runner every mutation goes through.
pub mod receipt;
/// `project show`: one charter by id.
pub mod show;
/// Slug derivation from a charter's name.
pub mod slug;
/// ISO-8601 rendering and `targetDate` parsing for epoch-millisecond columns.
pub mod timestamp;
/// The charter view every command returns, and its batched relation load.
pub mod view;
