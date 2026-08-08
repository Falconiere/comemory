#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integrity tests for [`crate::store::migrate::list`] — the checks that
//! stop the classes of drift this whole feature exists to prevent: a
//! `MIGRATIONS` entry silently missing its SQL file, a hand-labeled
//! `Class` rotting out of sync with what the SQL actually does, a declared
//! `markers` set that no longer matches what a real migrated database
//! holds (see `docs/architecture.md`'s schema-migration section). All fns
//! are prefixed `migration_integrity_`: the
//! project-wide `cargo nextest run -E 'test(migration_integrity)'` filter
//! selects exactly this suite plus `api::rebuild::tests::coverage`, and a
//! zero-match filter run exits 4 — a misnamed fn here turns that check red
//! rather than silently green.
//!
//! Real data throughout: the markers test builds an actual SQLite database
//! by replaying the real migration chain via `store::connection::open`, not
//! a hand-assembled fixture.

use std::collections::BTreeSet;
use std::fs;

use comemory::store::connection;
use comemory::store::migrate::list::{self, Class};
use comemory::store::migrate::preflight;
use comemory::store::migrate::{self};

/// `src/store/sql/*.sql` basenames (minus the `.sql` extension), read from
/// disk so this test cannot itself drift from the directory it checks.
fn sql_basenames() -> BTreeSet<String> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/store/sql");
    fs::read_dir(dir)
        .expect("read src/store/sql")
        .map(|entry| entry.expect("dir entry"))
        .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("sql"))
        .map(|entry| {
            entry
                .path()
                .file_stem()
                .expect("sql file has a stem")
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

/// The leading `NNNN` version number parsed off every `src/store/sql/*.sql`
/// basename, e.g. `13` for `0013_v13_documents`.
fn highest_sql_file_number() -> u32 {
    sql_basenames()
        .iter()
        .map(|name| {
            name.split('_')
                .next()
                .and_then(|prefix| prefix.parse::<u32>().ok())
                .unwrap_or_else(|| panic!("{name} does not start with a numeric prefix"))
        })
        .max()
        .expect("at least one migration file exists")
}

/// `MIGRATIONS[].key` must be exactly the set of `src/store/sql/*.sql`
/// basenames — no file left unwired, no dangling key naming a file that
/// does not exist.
#[test]
fn migration_integrity_migrations_keys_match_the_sql_directory() {
    let migration_keys: BTreeSet<String> =
        list::MIGRATIONS.iter().map(|m| m.key.to_string()).collect();
    assert_eq!(
        migration_keys,
        sql_basenames(),
        "MIGRATIONS[].key must equal the set of src/store/sql/*.sql basenames"
    );
}

/// `CURRENT_VERSION` (the hand-written literal), `CURRENT_VERSION_NUM`
/// (`MIGRATIONS.len()`), and the highest numbered `.sql` file on disk must
/// never disagree. `CURRENT_VERSION` cannot be derived from the other two
/// on stable Rust (E0015 — see its own doc comment), so this is the
/// machine check standing in for that derivation.
#[test]
fn migration_integrity_current_version_agrees_with_migrations_len() {
    assert_eq!(
        migrate::CURRENT_VERSION.parse::<u32>(),
        Ok(migrate::CURRENT_VERSION_NUM),
        "CURRENT_VERSION and CURRENT_VERSION_NUM have drifted apart"
    );
    assert_eq!(
        migrate::CURRENT_VERSION_NUM,
        list::MIGRATIONS.len() as u32,
        "CURRENT_VERSION_NUM must equal MIGRATIONS.len()"
    );
    assert_eq!(
        migrate::CURRENT_VERSION_NUM,
        highest_sql_file_number(),
        "CURRENT_VERSION_NUM must equal the highest numbered .sql file in src/store/sql"
    );
}

/// Every SQL shape that can destroy or silently overwrite existing rows —
/// checked against every migration's SQL text so a hand-maintained `Class`
/// label cannot rot the way the old hand-written migration list did.
/// `DELETE FROM` and `ALTER TABLE … DROP COLUMN` destroy data outright;
/// `INSERT OR REPLACE` silently overwrites an existing row rather than
/// erroring or skipping it — all three are as data-destroying as `DROP
/// TABLE`/`UPDATE ` and must gate the same mandatory-snapshot behavior, or a
/// failed snapshot downgrades from "refuse" to "warn" on exactly the
/// migration that most needs "refuse".
const DESTRUCTIVE_PATTERNS: &[&str] = &[
    "DROP TABLE",
    "UPDATE ",
    "DELETE FROM",
    "DROP COLUMN",
    "INSERT OR REPLACE",
];

/// True when `sql` contains any [`DESTRUCTIVE_PATTERNS`] shape.
fn sql_demands_destructive(sql: &str) -> bool {
    DESTRUCTIVE_PATTERNS
        .iter()
        .any(|pattern| sql.contains(pattern))
}

/// Any migration whose SQL text matches [`DESTRUCTIVE_PATTERNS`] must be
/// classed `Destructive`. Expected membership per the migration SQL itself:
/// 0004 (drops `memory_fts`/`code_fts`, `UPDATE`s `memories`/`code_symbols`),
/// 0005 (drops `search_stats`), 0006/0008/0013 (each rebuild `edges` via
/// create-copy-drop-rename).
#[test]
fn migration_integrity_destructive_sql_is_classed_destructive() {
    let destructive_keys: BTreeSet<&str> = list::MIGRATIONS
        .iter()
        .filter(|m| sql_demands_destructive(m.sql))
        .map(|m| m.key)
        .collect();
    let expected: BTreeSet<&str> = [
        "0004_v4_rank",
        "0005_v5_learning",
        "0006_v6_code_graph",
        "0008_v8_reinforcement",
        "0013_v13_documents",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        destructive_keys, expected,
        "the set of migrations whose SQL matches a destructive pattern has changed; \
         update this list only alongside a deliberate schema change"
    );
    for migration in list::MIGRATIONS {
        if sql_demands_destructive(migration.sql) {
            assert_eq!(
                migration.class,
                Class::Destructive,
                "{}'s SQL matches a destructive pattern but is classed Additive",
                migration.key
            );
        }
    }
}

/// The declared `Migration::markers` — unioned across `MIGRATIONS` — must
/// equal the real `schema_meta` marker keys a fully-migrated database
/// actually holds. This is the test that would have caught the "refuses
/// every existing database" bug: a naive "applied == MIGRATIONS keys"
/// check is wrong on a real database (`0001` writes no marker; `0004`/
/// `0005` each write a second, non-migration marker for their simhash
/// post-pass).
#[test]
fn migration_integrity_declared_markers_match_a_real_migrated_db() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    let conn = connection::open(&db).expect("build a real, fully-migrated db");

    let mut stmt = conn
        .prepare("SELECT key FROM schema_meta WHERE key LIKE '0%'")
        .expect("prepare");
    let applied: BTreeSet<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");

    let expected: BTreeSet<String> = preflight::expected_markers()
        .into_iter()
        .map(String::from)
        .collect();

    assert_eq!(
        applied, expected,
        "Migration::markers (unioned) must equal the real schema_meta marker keys"
    );
}
