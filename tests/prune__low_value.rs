//! Tests for [`comemory::prune::low_value::detect`] — signal-based
//! low-value detection over seeded `memories` / `feedback` / `edges`
//! rows (all columns from migration v4 present).
//!
//! Defaults in play: `prune.min_activation = -2.0`,
//! `prune.min_feedback = 0.25`, `prune.low_value_default_below_quality
//! = 2` (inclusive), `rank.decay = 0.5`.

use comemory::config::Config;
use comemory::prune::low_value::detect;

fn open_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

/// Insert one fully-populated (post-v4, 15-column) memory row.
fn seed_memory(
    conn: &rusqlite::Connection,
    id: &str,
    quality: u8,
    access_count: i64,
    last_accessed: &str,
) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, access_count, last_accessed, simhash)
         VALUES (?1, ?1, 'note', 'demo', 'f', ?2, 1, 'h-' || ?1, 'body ' || ?1,
                 '2024-12-01T00:00:00Z', '2024-12-01T00:00:00Z', 'm/' || ?1 || '.md', ?3, ?4, 0)",
        rusqlite::params![id, quality, access_count, last_accessed],
    )
    .expect("seed memory");
}

#[test]
fn flags_only_cold_unloved_low_quality_unreferenced() {
    let (_d, conn) = open_db();
    // Cold (≈17 months unaccessed), downvoted, quality 2, no incoming
    // edges → flagged.
    seed_memory(&conn, "aaaa0001", 2, 0, "2025-01-01T00:00:00Z");
    // Same signals but quality 4 (> below_quality 2) → survives.
    seed_memory(&conn, "aaaa0002", 4, 0, "2025-01-01T00:00:00Z");
    // Same quality but hot: 50 accesses, last access recent → activation
    // well above the −2.0 floor → survives.
    seed_memory(&conn, "aaaa0003", 2, 50, "2026-06-01T00:00:00Z");
    // Same as 0001 but referenced by an incoming edge → survives.
    seed_memory(&conn, "aaaa0004", 2, 0, "2025-01-01T00:00:00Z");
    conn.execute_batch(
        "INSERT INTO feedback(memory_id, used_count, irrelevant_count) VALUES
           ('aaaa0001', 0, 2), ('aaaa0002', 0, 2), ('aaaa0003', 0, 2), ('aaaa0004', 0, 2);
         INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa0002','memory','aaaa0004','derived_from','2026-01-01T00:00:00Z');",
    )
    .expect("seed feedback + edge");

    let ids = detect(&conn, &Config::defaults()).expect("detect");
    assert_eq!(ids, vec!["aaaa0001".to_string()]);
}

#[test]
fn superseded_and_untouched_since_is_flagged() {
    let (_d, conn) = open_db();
    // Good quality and not signal-stale enough on its own — but a live
    // memory superseded it on 2026-01-01 and it has not been accessed
    // since (last access 2025-06-01).
    seed_memory(&conn, "aaaa0001", 4, 10, "2025-06-01T00:00:00Z");
    seed_memory(&conn, "aaaa0002", 4, 1, "2026-06-01T00:00:00Z");
    conn.execute_batch(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa0002','memory','aaaa0001','supersedes','2026-01-01T00:00:00Z');",
    )
    .expect("seed supersede edge");

    let ids = detect(&conn, &Config::defaults()).expect("detect");
    assert_eq!(ids, vec!["aaaa0001".to_string()]);
}

#[test]
fn superseded_but_accessed_since_survives() {
    let (_d, conn) = open_db();
    // Superseded on 2026-01-01 but accessed afterwards (2026-05-01):
    // somebody still finds it useful, so the superseded rule must not
    // flag it.
    seed_memory(&conn, "aaaa0001", 4, 10, "2026-05-01T00:00:00Z");
    seed_memory(&conn, "aaaa0002", 4, 1, "2026-06-01T00:00:00Z");
    conn.execute_batch(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa0002','memory','aaaa0001','supersedes','2026-01-01T00:00:00Z');",
    )
    .expect("seed supersede edge");

    let ids = detect(&conn, &Config::defaults()).expect("detect");
    assert!(ids.is_empty(), "accessed-since memory flagged: {ids:?}");
}

#[test]
fn freshly_created_supersede_edge_is_not_flagged() {
    let (_d, conn) = open_db();
    // Same shape as the flagged case, but the edge was written just now —
    // inside the 7-day grace window that protects freshly-rebuilt DBs
    // (rebuild resets every edge timestamp to rebuild time).
    seed_memory(&conn, "aaaa0001", 4, 10, "2025-06-01T00:00:00Z");
    seed_memory(&conn, "aaaa0002", 4, 1, "2026-06-01T00:00:00Z");
    conn.execute_batch(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa0002','memory','aaaa0001','supersedes',
                 strftime('%Y-%m-%dT%H:%M:%fZ','now'));",
    )
    .expect("seed fresh supersede edge");

    let ids = detect(&conn, &Config::defaults()).expect("detect");
    assert!(ids.is_empty(), "grace-window edge flagged: {ids:?}");
}

#[test]
fn superseded_grace_days_config_is_honored() {
    let (_d, conn) = open_db();
    // Edge written ~14 days ago, memory untouched since: outside the
    // default 7-day grace (flagged), inside a widened 30-day grace
    // (cfg.prune.superseded_grace_days = 30 → not flagged).
    seed_memory(&conn, "aaaa0001", 4, 10, "2025-06-01T00:00:00Z");
    seed_memory(&conn, "aaaa0002", 4, 1, "2026-06-01T00:00:00Z");
    conn.execute_batch(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa0002','memory','aaaa0001','supersedes',
                 strftime('%Y-%m-%dT%H:%M:%fZ','now','-14 days'));",
    )
    .expect("seed aged supersede edge");

    let ids = detect(&conn, &Config::defaults()).expect("detect default grace");
    assert_eq!(
        ids,
        vec!["aaaa0001".to_string()],
        "14-day-old edge is past the default 7-day grace"
    );

    let mut cfg = Config::defaults();
    cfg.prune.superseded_grace_days = 30;
    let ids = detect(&conn, &cfg).expect("detect widened grace");
    assert!(ids.is_empty(), "30-day grace must shield the edge: {ids:?}");
}

#[test]
fn self_supersede_edge_does_not_flag() {
    let (_d, conn) = open_db();
    // Defense-in-depth: a hand-seeded self-edge (the writers refuse to
    // create one) must not make the memory "superseded and forgotten" —
    // quality 4 keeps it clear of the signal rule.
    seed_memory(&conn, "aaaa0001", 4, 10, "2025-06-01T00:00:00Z");
    conn.execute_batch(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa0001','memory','aaaa0001','supersedes','2026-01-01T00:00:00Z');",
    )
    .expect("seed self edge");

    let ids = detect(&conn, &Config::defaults()).expect("detect");
    assert!(ids.is_empty(), "self-superseded memory flagged: {ids:?}");
}

#[test]
fn soft_deleted_superseder_does_not_flag() {
    let (_d, conn) = open_db();
    seed_memory(&conn, "aaaa0001", 4, 10, "2025-06-01T00:00:00Z");
    seed_memory(&conn, "aaaa0002", 4, 1, "2026-06-01T00:00:00Z");
    conn.execute_batch(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa0002','memory','aaaa0001','supersedes','2026-01-01T00:00:00Z');
         UPDATE memories SET deleted_at = '2026-06-02T00:00:00Z' WHERE id = 'aaaa0002';",
    )
    .expect("seed + soft-delete superseder");

    let ids = detect(&conn, &Config::defaults()).expect("detect");
    assert!(ids.is_empty(), "dead superseder must not flag: {ids:?}");
}

#[test]
fn fresh_memory_is_not_low_value() {
    let (_d, conn) = open_db();
    // Quality 1 and zero feedback, but accessed today: activation 0 is
    // above the −2.0 floor, so the signal rule must not fire.
    let today = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .expect("format now");
    seed_memory(&conn, "aaaa0001", 1, 0, &today);

    let ids = detect(&conn, &Config::defaults()).expect("detect");
    assert!(ids.is_empty(), "fresh memory flagged: {ids:?}");
}

#[test]
fn positive_feedback_protects_cold_memory() {
    let (_d, conn) = open_db();
    // Cold and quality 2, but used 5×/0 irrelevant → Beta posterior
    // (5+1)/(5+4) ≈ 0.67 > 0.25 ceiling → survives.
    seed_memory(&conn, "aaaa0001", 2, 0, "2025-01-01T00:00:00Z");
    conn.execute(
        "INSERT INTO feedback(memory_id, used_count, irrelevant_count) VALUES ('aaaa0001', 5, 0)",
        [],
    )
    .expect("seed feedback");

    let ids = detect(&conn, &Config::defaults()).expect("detect");
    assert!(ids.is_empty(), "well-used memory flagged: {ids:?}");
}

#[test]
fn soft_deleted_memories_are_ignored() {
    let (_d, conn) = open_db();
    seed_memory(&conn, "aaaa0001", 2, 0, "2025-01-01T00:00:00Z");
    conn.execute(
        "UPDATE memories SET deleted_at = '2026-06-01T00:00:00Z' WHERE id = 'aaaa0001'",
        [],
    )
    .expect("soft delete");

    let ids = detect(&conn, &Config::defaults()).expect("detect");
    assert!(ids.is_empty(), "already-deleted memory flagged: {ids:?}");
}

#[test]
fn ids_are_sorted_and_deduplicated_across_rules() {
    let (_d, conn) = open_db();
    // aaaa0002 matches the signal rule; aaaa0001 matches BOTH rules
    // (cold/unloved/low-quality — its only incoming edge is the
    // supersede edge, so the signal rule skips it, but the superseded
    // rule catches it). Output must be sorted with no duplicates.
    seed_memory(&conn, "aaaa0002", 2, 0, "2025-01-01T00:00:00Z");
    seed_memory(&conn, "aaaa0001", 2, 0, "2025-01-01T00:00:00Z");
    seed_memory(&conn, "aaaa0003", 5, 1, "2026-06-01T00:00:00Z");
    conn.execute_batch(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa0003','memory','aaaa0001','supersedes','2026-01-01T00:00:00Z');",
    )
    .expect("seed supersede edge");

    let ids = detect(&conn, &Config::defaults()).expect("detect");
    assert_eq!(ids, vec!["aaaa0001".to_string(), "aaaa0002".to_string()]);
}
