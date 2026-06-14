//! Mirror test for `store::memory_list::list_memories`: pagination window,
//! total count, stable `created_at DESC, id ASC` order, `repo`/`kind` filters,
//! soft-delete exclusion, and the `limit == 0` "all" sentinel. Writes real
//! `memories` rows through the shared `memory_row::insert` path (no mocks).

use comemory::memory::{Frontmatter, Kind, References, Relations};
use comemory::store::memory_list::{self, ListRow};
use comemory::store::{connection, memory_row};
use rusqlite::Connection;
use time::OffsetDateTime;

/// Insert a memory row with an explicit id, kind, repo, and `created`
/// timestamp so order and filters are deterministic. `created` is an offset in
/// whole days from a fixed epoch; a larger offset is more recent. The
/// `md_path` is `{id}-{slug}.md` so `file_stem` recovers `{id}-{slug}`.
fn seed(conn: &mut Connection, id: &str, kind: Kind, repo: &str, day: i64) {
    let created =
        OffsetDateTime::from_unix_timestamp(1_700_000_000 + day * 86_400).expect("valid timestamp");
    let fm = Frontmatter {
        id: id.to_string(),
        kind,
        repo: repo.to_string(),
        tags: Vec::new(),
        author: "alice".to_string(),
        created,
        quality: 3,
        schema: 1,
        content_hash: format!("hash-{id}"),
        references: References::default(),
        relations: Relations::default(),
    };
    let slug = format!("note-{id}");
    let md_path = format!("/data/.comemory/memories/{id}-{slug}.md");
    let body = format!("body for {id}");
    let tx = conn.transaction().expect("tx");
    memory_row::insert(&tx, &fm, &body, &slug, &md_path, &fm.tags).expect("insert");
    tx.commit().expect("commit");
}

/// Soft-delete a row the same way `comemory delete` stamps the mirror.
fn soft_delete(conn: &Connection, id: &str) {
    conn.execute(
        "UPDATE memories SET deleted_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?1",
        [id],
    )
    .expect("soft delete");
}

fn ids(rows: &[ListRow]) -> Vec<&str> {
    rows.iter().map(|r| r.id.as_str()).collect()
}

/// Eight rows across two repos and two kinds, with distinct created days so
/// the expected `created_at DESC, id ASC` order is unambiguous. Day grows with
/// the id suffix, so newest-first is `…8, …7, …6, …`.
fn seeded_db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    seed(&mut conn, "aaaa0001", Kind::Decision, "alpha", 1);
    seed(&mut conn, "aaaa0002", Kind::Bug, "alpha", 2);
    seed(&mut conn, "aaaa0003", Kind::Decision, "beta", 3);
    seed(&mut conn, "aaaa0004", Kind::Bug, "beta", 4);
    seed(&mut conn, "aaaa0005", Kind::Decision, "alpha", 5);
    seed(&mut conn, "aaaa0006", Kind::Bug, "alpha", 6);
    seed(&mut conn, "aaaa0007", Kind::Decision, "beta", 7);
    seed(&mut conn, "aaaa0008", Kind::Bug, "beta", 8);
    (dir, conn)
}

#[test]
fn lists_all_in_stable_created_desc_id_asc_order() {
    let (_dir, conn) = seeded_db();
    let page = memory_list::list_memories(&conn, None, None, 0, 0).expect("list");
    assert_eq!(page.total, 8);
    assert_eq!(page.rows.len(), 8);
    // Newest day first (day 8 == aaaa0008), oldest last.
    assert_eq!(
        ids(&page.rows),
        vec![
            "aaaa0008", "aaaa0007", "aaaa0006", "aaaa0005", "aaaa0004", "aaaa0003", "aaaa0002",
            "aaaa0001",
        ]
    );
}

#[test]
fn id_tiebreak_orders_equal_timestamps_ascending() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    // Same created day for all three: the id tiebreak must order them ASC so
    // pagination windows are stable across calls.
    seed(&mut conn, "bbbb0003", Kind::Note, "alpha", 9);
    seed(&mut conn, "bbbb0001", Kind::Note, "alpha", 9);
    seed(&mut conn, "bbbb0002", Kind::Note, "alpha", 9);
    let page = memory_list::list_memories(&conn, None, None, 0, 0).expect("list");
    assert_eq!(ids(&page.rows), vec!["bbbb0001", "bbbb0002", "bbbb0003"]);
}

#[test]
fn limit_and_offset_window_the_middle_page() {
    let (_dir, conn) = seeded_db();
    let page = memory_list::list_memories(&conn, None, None, 3, 2).expect("list");
    // Total is the full filtered set, independent of the window.
    assert_eq!(page.total, 8);
    // Skip the two newest (0008, 0007), take the next three.
    assert_eq!(ids(&page.rows), vec!["aaaa0006", "aaaa0005", "aaaa0004"]);
}

#[test]
fn offset_past_end_returns_empty_with_full_total() {
    let (_dir, conn) = seeded_db();
    let page = memory_list::list_memories(&conn, None, None, 5, 100).expect("list");
    assert_eq!(page.total, 8);
    assert!(page.rows.is_empty());
}

#[test]
fn repo_filter_restricts_total_and_rows() {
    let (_dir, conn) = seeded_db();
    let page = memory_list::list_memories(&conn, Some("beta"), None, 0, 0).expect("list");
    assert_eq!(page.total, 4);
    assert_eq!(
        ids(&page.rows),
        vec!["aaaa0008", "aaaa0007", "aaaa0004", "aaaa0003"]
    );
}

#[test]
fn kind_filter_restricts_total_and_rows() {
    let (_dir, conn) = seeded_db();
    let page = memory_list::list_memories(&conn, None, Some("decision"), 0, 0).expect("list");
    assert_eq!(page.total, 4);
    assert_eq!(
        ids(&page.rows),
        vec!["aaaa0007", "aaaa0005", "aaaa0003", "aaaa0001"]
    );
}

#[test]
fn repo_and_kind_filters_combine() {
    let (_dir, conn) = seeded_db();
    let page = memory_list::list_memories(&conn, Some("alpha"), Some("bug"), 0, 0).expect("list");
    assert_eq!(page.total, 2);
    assert_eq!(ids(&page.rows), vec!["aaaa0006", "aaaa0002"]);
}

#[test]
fn soft_deleted_rows_are_excluded() {
    let (_dir, conn) = seeded_db();
    soft_delete(&conn, "aaaa0008");
    soft_delete(&conn, "aaaa0001");
    let page = memory_list::list_memories(&conn, None, None, 0, 0).expect("list");
    assert_eq!(page.total, 6);
    let listed = ids(&page.rows);
    assert!(!listed.contains(&"aaaa0008"));
    assert!(!listed.contains(&"aaaa0001"));
}

#[test]
fn limit_zero_returns_all_rows() {
    let (_dir, conn) = seeded_db();
    let all = memory_list::list_memories(&conn, None, None, 0, 0).expect("list");
    assert_eq!(all.rows.len(), 8);
    assert_eq!(all.total, 8);
}

#[test]
fn slug_is_file_stem_id_dash_slug() {
    let (_dir, conn) = seeded_db();
    let page = memory_list::list_memories(&conn, None, None, 1, 0).expect("list");
    let row = &page.rows[0];
    // `md_path` was `…/aaaa0008-note-aaaa0008.md`; the slug field is the file
    // stem `{id}-{slug}`, matching the legacy markdown-scan output.
    assert_eq!(row.slug, "aaaa0008-note-aaaa0008");
    assert_eq!(row.repo, "beta");
    assert_eq!(row.kind, "bug");
}
