#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Live-table coverage test for [`super::COPIED_TABLES`] /
//! [`super::RECONSTRUCTABLE_TABLES`] — the half of the migration-integrity
//! suite that lives beside `rebuild::copy` rather than `store::migrate::list`
//! (see `src/store/migrate/tests/list.rs`'s module doc for the shared naming
//! contract). Prefixed `migration_integrity_` for the same reason: the
//! `cargo nextest run -E 'test(migration_integrity)'` filter must select it,
//! and a zero-match filter run exits 4.
//!
//! The live table set is derived from the real migration SQL — CREATEs
//! minus DROPs, with `ALTER TABLE … RENAME TO …` applied as a rewrite, in
//! chain order — never hand-listed, so it cannot itself rot the way the
//! allowlists it checks against could. `search_stats` (created 0002,
//! dropped 0005, never recreated) is excluded by construction; `edges`
//! (dropped and recreated via a renamed scratch table by 0006/0008/0013) is
//! retained by construction, because the RENAME step reclaims its name
//! before the set is read.

use std::collections::BTreeSet;

use comemory::api::rebuild::copy::{COPIED_TABLES, RECONSTRUCTABLE_TABLES};
use comemory::store::connection;
use comemory::store::migrate::list::MIGRATIONS;
use regex::Regex;

/// One `CREATE [VIRTUAL] TABLE [IF NOT EXISTS] <name>`, `DROP TABLE [IF
/// EXISTS] <name>`, or `ALTER TABLE <a> RENAME TO <b>` statement found in a
/// migration's SQL text, in the order it appears.
enum Stmt {
    Create(String),
    Drop(String),
    Rename(String, String),
}

/// The three statement shapes, as one alternation so `captures_iter` yields
/// them in source order — the order [`derive_live_tables`] must replay them
/// in, since e.g. 0006's `edges` rebuild is a CREATE, a DROP, and a RENAME
/// on the very same table name within one migration's SQL. Built via
/// `concat!` of separate raw-string fragments rather than one
/// backslash-continued raw string: a raw string never processes `\`, so a
/// trailing `\` before a newline stays a literal backslash character in the
/// pattern instead of continuing the line.
const STATEMENT_PATTERN: &str = concat!(
    r"(?i)CREATE\s+(?:VIRTUAL\s+)?TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?(?P<create>[A-Za-z_][A-Za-z0-9_]*)",
    r"|DROP\s+TABLE\s+(?:IF\s+EXISTS\s+)?(?P<drop>[A-Za-z_][A-Za-z0-9_]*)",
    r"|ALTER\s+TABLE\s+(?P<rename_from>[A-Za-z_][A-Za-z0-9_]*)\s+RENAME\s+TO\s+(?P<rename_to>[A-Za-z_][A-Za-z0-9_]*)",
);

/// Compile [`STATEMENT_PATTERN`].
fn statement_pattern() -> Regex {
    Regex::new(STATEMENT_PATTERN).expect("statement pattern compiles")
}

/// Every CREATE/DROP/RENAME statement across every migration's SQL, in
/// chain order (migration order, then source order within each
/// migration's text).
fn statements() -> Vec<Stmt> {
    let pattern = statement_pattern();
    MIGRATIONS
        .iter()
        .flat_map(|m| pattern.captures_iter(m.sql).collect::<Vec<_>>())
        .map(|caps| {
            if let Some(name) = caps.name("create") {
                Stmt::Create(name.as_str().to_string())
            } else if let Some(name) = caps.name("drop") {
                Stmt::Drop(name.as_str().to_string())
            } else {
                let from = caps.name("rename_from").expect("rename_from present");
                let to = caps.name("rename_to").expect("rename_to present");
                Stmt::Rename(from.as_str().to_string(), to.as_str().to_string())
            }
        })
        .collect()
}

/// Replay every [`Stmt`] against an initially empty set: CREATE inserts,
/// DROP removes, RENAME removes the old name and inserts the new one. This
/// is the live-table derivation itself — CREATEs minus DROPs with RENAME
/// applied, in chain order.
fn derive_live_tables() -> BTreeSet<String> {
    let mut live = BTreeSet::new();
    for stmt in statements() {
        match stmt {
            Stmt::Create(name) => {
                live.insert(name);
            }
            Stmt::Drop(name) => {
                live.remove(&name);
            }
            Stmt::Rename(from, to) => {
                live.remove(&from);
                live.insert(to);
            }
        }
    }
    live
}

/// The literal `27` is a deliberate canary, not an incidental constant: a
/// migration that adds or drops a table is *expected* to fail this test, and
/// the failure is the prompt to decide whether the new table belongs in
/// `COPIED_TABLES` or `RECONSTRUCTABLE_TABLES`. Bump the number in the same
/// change that answers that question — never to make the test pass again.
///
/// Bumped 25 -> 27 by the v14 console migration, which added `eval_runs`
/// and `gc_runs`, and 27 -> 28 by the v15 console-API migration, which
/// added `index_runs`. All three went into `COPIED_TABLES`: they are run
/// history, and markdown cannot reconstruct history. Without that answer a
/// rebuild would have silently discarded every recorded eval, gc, and
/// index run — which is exactly what this canary exists to prevent.
#[test]
fn migration_integrity_derived_live_set_has_exactly_twenty_eight_tables() {
    let live = derive_live_tables();
    assert_eq!(
        live.len(),
        28,
        "expected exactly 28 live tables, got {}: {live:?}",
        live.len()
    );
    // The count alone would still pass if a history table were added to
    // the schema and then filed under the wrong allowlist, so name the
    // three the bumps above were about.
    for table in ["eval_runs", "gc_runs", "index_runs"] {
        assert!(
            COPIED_TABLES.contains(&table),
            "{table} is run history markdown cannot reconstruct — it must be COPIED"
        );
        assert!(
            !RECONSTRUCTABLE_TABLES
                .iter()
                .any(|(name, _)| *name == table),
            "{table} must not also be listed as reconstructable"
        );
    }
    assert!(
        live.contains("edges"),
        "edges must survive the RENAME rewrite (0006/0008/0013 each drop and \
         rename a scratch table over it)"
    );
    assert!(
        !live.contains("search_stats"),
        "search_stats is dropped by 0005 and never recreated"
    );
    for scratch in ["edges_v6", "edges_v8", "edges_v13"] {
        assert!(
            !live.contains(scratch),
            "{scratch} is a rebuild scratch table renamed away within its own migration"
        );
    }
}

#[test]
fn migration_integrity_every_live_table_is_covered_exactly_once() {
    let live = derive_live_tables();
    let copied: BTreeSet<&str> = COPIED_TABLES.iter().copied().collect();
    let reconstructable: BTreeSet<&str> = RECONSTRUCTABLE_TABLES.iter().map(|(t, _)| *t).collect();

    assert_eq!(
        copied.len(),
        COPIED_TABLES.len(),
        "COPIED_TABLES must not repeat a table name"
    );
    assert_eq!(
        reconstructable.len(),
        RECONSTRUCTABLE_TABLES.len(),
        "RECONSTRUCTABLE_TABLES must not repeat a table name"
    );

    let overlap: Vec<&&str> = copied.intersection(&reconstructable).collect();
    assert!(
        overlap.is_empty(),
        "a table cannot appear in both COPIED_TABLES and RECONSTRUCTABLE_TABLES: {overlap:?}"
    );

    for table in &live {
        assert!(
            copied.contains(table.as_str()) || reconstructable.contains(table.as_str()),
            "{table} is a live table but appears in neither COPIED_TABLES nor \
             RECONSTRUCTABLE_TABLES"
        );
    }
    for table in copied.iter().chain(reconstructable.iter()) {
        assert!(
            live.contains(*table),
            "{table} is declared in COPIED_TABLES/RECONSTRUCTABLE_TABLES but is not a live table"
        );
    }
}

/// SQLite creates internal shadow tables backing every FTS5 and `vec0`
/// virtual table (verified against a real migrated database, not assumed):
/// `<name>_data`/`_idx`/`_content`/`_docsize`/`_config` for FTS5,
/// `<name>_chunks`/`_info`/`_rowids`/`_vector_chunks00` for `vec0`. None of
/// these are literal `CREATE TABLE` statements in our migration SQL, so
/// [`derive_live_tables`] never produces them — they must be filtered out
/// of the real `sqlite_master` listing before the two sets can be compared.
const SHADOW_SUFFIXES: &[&str] = &[
    "_data",
    "_idx",
    "_content",
    "_docsize",
    "_config",
    "_chunks",
    "_info",
    "_rowids",
    "_vector_chunks00",
];

/// True when `name` is a shadow companion of some other table in `bases` —
/// i.e. `name` is exactly `<base><suffix>` for a base that is itself a
/// distinct entry in `bases`.
fn is_shadow_of_another(name: &str, bases: &BTreeSet<String>) -> bool {
    SHADOW_SUFFIXES
        .iter()
        .filter_map(|suffix| name.strip_suffix(suffix))
        .any(|base| bases.contains(base))
}

#[test]
fn migration_integrity_derived_live_set_matches_a_real_migrated_db() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    let conn = connection::open(&db).expect("build a real, fully-migrated db");

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .expect("prepare");
    let raw: BTreeSet<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");

    let real: BTreeSet<String> = raw
        .iter()
        .filter(|name| !is_shadow_of_another(name, &raw))
        .cloned()
        .collect();

    assert_eq!(
        derive_live_tables(),
        real,
        "the SQL-derived live table set must match a real migrated database's sqlite_master, \
         once FTS5/vec0 shadow companions are filtered out"
    );
}
