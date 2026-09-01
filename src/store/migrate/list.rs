//! The single source of truth for every schema migration: key, SQL,
//! destructiveness class, optional Rust post-pass, and the `schema_meta`
//! markers it writes. [`run`](super::run) iterates [`MIGRATIONS`] instead of
//! hand-listing each `apply` call, and the replay test helpers slice it too
//! — one list, so it cannot fall behind either consumer the way the old
//! hand-written pair did.

use rusqlite::Connection;

use super::{
    M_BOOTSTRAP, M_V2, M_V3, M_V4, M_V5, M_V6, M_V7, M_V8, M_V9, M_V10, M_V11, M_V12, M_V13, M_V14,
    M_V15, backfill_memory_simhash, rehash_simhashes,
};
use crate::prelude::*;

/// Whether a migration can destroy data it does not also rewrite.
///
/// Drives one decision: whether a failed pre-migration snapshot degrades to
/// a warning ([`Additive`](Class::Additive)) or refuses the upgrade
/// ([`Destructive`](Class::Destructive)). Two classes, not three — the
/// decision is binary, and biasing toward the snapshot is the safer error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Loses nothing on a failed snapshot: every column it adds carries a
    /// default, and it never drops or overwrites existing rows.
    Additive,
    /// Drops a table, rebuilds one via create-copy-drop-rename, or
    /// `UPDATE`s existing rows in place — a failed snapshot before this
    /// class must refuse the upgrade rather than warn.
    Destructive,
}

/// One versioned schema migration and its post-apply Rust pass, if any.
pub struct Migration {
    /// The `schema_meta` marker key this migration is applied under, e.g.
    /// `"0004_v4_rank"`. Matches the historical `apply(conn, key, sql)`
    /// call it replaces.
    pub key: &'static str,
    /// The migration's DDL/DML text, applied inside one transaction (the
    /// bootstrap migration is the one exception — see [`super::apply`]).
    pub class: Class,
    /// The migration's SQL text.
    pub sql: &'static str,
    /// A Rust pass that runs immediately after this migration applies,
    /// inside the same `run` call but its own transaction. `Some` only for
    /// `0004` (simhash backfill) and `0005` (simhash rehash) — SQLite has
    /// no `simhash()` function, so both post-passes recompute in Rust.
    pub post: Option<fn(&mut Connection) -> Result<()>>,
    /// Every `schema_meta` key this migration writes. Empty for `0001`,
    /// whose bootstrap `apply` special-case returns before the marker
    /// insert; two entries for `0004`/`0005`, whose post-pass each writes
    /// its own run-once marker; one entry for the rest. The union across
    /// [`MIGRATIONS`] is the expected marker set a fully-migrated database
    /// holds.
    pub markers: &'static [&'static str],
}

/// Every migration in apply order. Single source of truth: `run` iterates
/// it, and the replay test helpers slice it to reconstruct a historical
/// schema state — neither consumer can hand-list a set that drifts from the
/// other.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        key: "0001_schema_meta",
        sql: M_BOOTSTRAP,
        class: Class::Additive,
        post: None,
        markers: &[],
    },
    Migration {
        key: "0002_v2_tables",
        sql: M_V2,
        class: Class::Additive,
        post: None,
        markers: &["0002_v2_tables"],
    },
    Migration {
        key: "0003_stats_tables",
        sql: M_V3,
        class: Class::Additive,
        post: None,
        markers: &["0003_stats_tables"],
    },
    Migration {
        key: "0004_v4_rank",
        sql: M_V4,
        class: Class::Destructive,
        post: Some(backfill_memory_simhash),
        markers: &["0004_v4_rank", "0004_simhash_backfill"],
    },
    Migration {
        key: "0005_v5_learning",
        sql: M_V5,
        class: Class::Destructive,
        post: Some(rehash_simhashes),
        markers: &["0005_v5_learning", "0005_simhash_rehash"],
    },
    Migration {
        key: "0006_v6_code_graph",
        sql: M_V6,
        class: Class::Destructive,
        post: None,
        markers: &["0006_v6_code_graph"],
    },
    Migration {
        key: "0007_v7_repo_root",
        sql: M_V7,
        class: Class::Additive,
        post: None,
        markers: &["0007_v7_repo_root"],
    },
    Migration {
        key: "0008_v8_reinforcement",
        sql: M_V8,
        class: Class::Destructive,
        post: None,
        markers: &["0008_v8_reinforcement"],
    },
    Migration {
        key: "0009_v9_code_refs",
        sql: M_V9,
        class: Class::Additive,
        post: None,
        markers: &["0009_v9_code_refs"],
    },
    Migration {
        key: "0010_v10_bandit",
        sql: M_V10,
        class: Class::Additive,
        post: None,
        markers: &["0010_v10_bandit"],
    },
    Migration {
        key: "0011_v11_memory_rank",
        sql: M_V11,
        class: Class::Additive,
        post: None,
        markers: &["0011_v11_memory_rank"],
    },
    Migration {
        key: "0012_v12_edge_fts",
        sql: M_V12,
        class: Class::Additive,
        post: None,
        markers: &["0012_v12_edge_fts"],
    },
    Migration {
        key: "0013_v13_documents",
        sql: M_V13,
        class: Class::Destructive,
        post: None,
        markers: &["0013_v13_documents"],
    },
    Migration {
        key: "0014_v14_console",
        sql: M_V14,
        class: Class::Additive,
        post: None,
        markers: &["0014_v14_console"],
    },
    Migration {
        key: "0015_v15_console_api",
        sql: M_V15,
        class: Class::Additive,
        post: None,
        markers: &["0015_v15_console_api"],
    },
];

#[cfg(test)]
#[path = "tests/list.rs"]
mod tests;
