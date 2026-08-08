#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The upgrade matrix: for every historical schema version `N` in
//! `2..CURRENT_VERSION_NUM`, a database seeded at `N` by
//! [`comemory::store::migrate::tests_2::build_legacy_db`] must migrate to
//! head via the real `connection::open` path with its seeded row intact
//! and a real (non-zero) backfilled `simhash`.
//!
//! Scoped from 2, not 1: `memories` does not exist until
//! `0002_v2_tables.sql`, so v1 is not seedable at all — that state is
//! covered by the existing fresh-create tests instead.
//!
//! The non-zero-simhash assertion is the specific regression this suite
//! exists to make impossible again. Before `build_legacy_db` was fixed to
//! seed the `0004_simhash_backfill`/`0005_simhash_rehash` markers only
//! when `through >= 4`/`>= 5`, it seeded both unconditionally — fabricating
//! a state no binary ever wrote, which suppressed the two Rust post-passes
//! and let a v2/v3-seeded row reach head still carrying the v4 `DEFAULT 0`
//! placeholder. This suite would have passed vacuously against that bug.
//!
//! The version range is derived from `CURRENT_VERSION_NUM` rather than
//! hand-listed, so a future migration cannot silently fall outside the
//! matrix the way `migration_chain()` fell a migration behind `run()`.
//!
//! All fns are prefixed `upgrade_matrix_`: the project-wide
//! `cargo nextest run -E 'test(upgrade_matrix)'` filter selects exactly
//! this suite, and a zero-match filter run exits 4 — a misnamed fn here
//! turns that check red rather than silently green.
//!
//! Real data throughout: every fixture is a genuine SQLite database built
//! by replaying the real historical migration SQL, migrated by the real
//! `connection::open` path — no mocks.

use comemory::store::connection;
use comemory::store::migrate::tests_2::build_legacy_db;
use comemory::store::migrate::{self};
use tempfile::tempdir;

/// Build a legacy database seeded at schema version `through`, migrate it
/// to head, and assert the one seeded `memories` row survives verbatim
/// with a real (non-zero) `simhash`. Shared body every version in the
/// matrix runs through.
fn assert_version_survives_to_head(through: u32) {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    let memory_id = format!("aaaa{through:04}");
    build_legacy_db(
        &db,
        through as usize,
        &through.to_string(),
        memory_id.as_str(),
    );

    let conn = connection::open(&db)
        .unwrap_or_else(|e| panic!("v{through}-seeded db must migrate to head: {e}"));

    let (slug, kind, content_hash, body, md_path, simhash): (
        String,
        String,
        String,
        String,
        String,
        i64,
    ) = conn
        .query_row(
            "SELECT slug, kind, content_hash, body, md_path, simhash FROM memories WHERE id = ?1",
            [memory_id.as_str()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .unwrap_or_else(|e| panic!("v{through}-seeded row must survive to head: {e}"));

    assert_eq!(slug, "note", "v{through}: slug changed across upgrade");
    assert_eq!(kind, "decision", "v{through}: kind changed across upgrade");
    assert_eq!(
        content_hash, "h",
        "v{through}: content_hash changed across upgrade"
    );
    assert_eq!(body, "body", "v{through}: body changed across upgrade");
    assert_eq!(
        md_path, "m/x.md",
        "v{through}: md_path changed across upgrade"
    );
    assert_ne!(
        simhash, 0,
        "v{through}-seeded row reached head with simhash still 0 — the v4 backfill or \
         v5 rehash post-pass did not run against it"
    );

    let version: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'version'",
            [],
            |r| r.get(0),
        )
        .expect("version row");
    assert_eq!(version, migrate::CURRENT_VERSION);
}

/// Runs [`assert_version_survives_to_head`] against every historical
/// version `2..CURRENT_VERSION_NUM`. Also proves the matrix itself is
/// non-empty, guarding against a vacuous pass if `CURRENT_VERSION_NUM`
/// ever regressed to or below the v1 floor.
#[test]
fn upgrade_matrix_every_historical_version_survives_to_head() {
    let versions: Vec<u32> = (2..migrate::CURRENT_VERSION_NUM).collect();
    assert!(
        !versions.is_empty(),
        "matrix requires at least one historical version above the v1 floor"
    );
    for through in versions {
        assert_version_survives_to_head(through);
    }
}
