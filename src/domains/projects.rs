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
/// A project's transfer binding: recorded on a transfer, shown by `project show`.
pub mod binding;
/// The transfer bundle: canonical rows, digest, parse and actor remap.
pub mod bundle;
/// A parsed bundle checked against this engine's carried tables.
pub mod bundle_check;
/// `project changes`: the body-free change feed and its writers.
pub mod changes;
/// A create request checked against every charter rule.
pub mod charter;
/// `project create`: a draft charter and its `project.created` event.
pub mod create;
/// Typed evidence: the kind and trust vocabularies, the claim shape and its
/// initial trust, the stored metadata and the wire view (#346).
pub mod evidence;
/// `project evidence add`: one evidence row on a project or work item.
pub mod evidence_add;
/// The checks an evidence attach runs before any store access.
pub mod evidence_check;
/// `project evidence list`: a filtered keyset page of a project's evidence.
pub mod evidence_page;
/// `project export`: one project as a canonical transfer bundle.
pub mod export;
/// `project import`: a bundle written under the same ids, or compared.
pub mod import;
/// The epoch-millisecond `<ms>:<uuid>` keyset cursor every project page uses.
pub mod keyset;
/// `project archive|restore|pause|resume`: the four lifecycle commands.
pub mod lifecycle;
/// The platform's charter and paging caps and their `422` refusals.
pub mod limits;
/// `project list`: a keyset page of charters.
pub mod list;
/// The `local_only` warning a mutation of a transferred project carries.
pub mod local_only;
/// Criterion, milestone and charter shapes a plan operation carries.
pub mod operation_fields;
/// Request-level rules and normalization of a proposal's operations.
pub mod operation_rules;
/// The twelve typed plan operations a proposal is written in.
pub mod operations;
/// `project plan show`: the committed plan at the current version.
pub mod plan;
/// The principal kinds and ids an envelope carries.
pub mod principal;
/// A plan proposal's wire view.
pub mod proposal_view;
/// `project proposal list|show`: a project's proposals.
pub mod proposals;
/// `project proposal submit`: an immutable plan proposal.
pub mod propose;
/// The idempotent-command runner every mutation but hard deletion and
/// transfer import goes through.
pub mod receipt;
/// `project show`: one charter by id.
pub mod show;
/// Slug derivation from a charter's name.
pub mod slug;
/// ISO-8601 rendering and `targetDate` parsing for epoch-millisecond columns.
pub mod timestamp;
/// The charter view every command returns, and its batched relation load.
pub mod view;
/// A work item's shapes as a plan operation carries them.
pub mod work_item_fields;
/// A work item's request-level rules, shared by create and patch.
pub mod work_item_rules;
