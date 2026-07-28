//! Mirror for `comemory::store::simhash_scan` — the bulk `(id, simhash)`
//! scan shared by the save near-dup check and `comemory consolidate`.

use comemory::store::simhash_scan::live_simhashes;

/// Insert one memory row with an explicit repo, simhash and delete state.
fn insert(
    conn: &rusqlite::Connection,
    id: &str,
    repo: &str,
    simhash: i64,
    deleted_at: Option<&str>,
) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, simhash, deleted_at)
         VALUES (?1, ?1, 'note', ?2, 'f', 3, 1, ?1, 'body text',
                 '2026-07-28T00:00:00Z', '2026-07-28T00:00:00Z', ?1, ?3, ?4)",
        rusqlite::params![id, repo, simhash, deleted_at],
    )
    .expect("insert memory");
}

/// Open a real store and seed four rows: two in `alpha`, one in `beta`, and
/// one soft-deleted. Returns the tempdir guard and the connection.
fn seeded() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    insert(&conn, "aaaa0001", "alpha", 42, None);
    insert(&conn, "aaaa0002", "alpha", 43, None);
    insert(&conn, "bbbb0001", "beta", 44, None);
    insert(&conn, "cccc0001", "alpha", 45, Some("2026-07-28T01:00:00Z"));
    (dir, conn)
}

#[test]
fn scan_returns_live_rows_id_ordered_and_skips_soft_deleted() {
    let (_d, conn) = seeded();
    let rows = live_simhashes(&conn, None, None).expect("scan");
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["aaaa0001", "aaaa0002", "bbbb0001"],
        "soft-deleted rows are absent and the order is id-ascending"
    );
    assert_eq!(
        rows.iter().map(|r| r.simhash).collect::<Vec<_>>(),
        vec![42, 43, 44],
        "each row carries its stored fingerprint verbatim"
    );
}

#[test]
fn repo_filter_restricts_the_scan() {
    let (_d, conn) = seeded();
    let rows = live_simhashes(&conn, Some("beta"), None).expect("scan");
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["bbbb0001"],
        "only the beta row survives the repo predicate"
    );
}

#[test]
fn exclude_drops_exactly_one_id() {
    let (_d, conn) = seeded();
    let rows = live_simhashes(&conn, None, Some("aaaa0001")).expect("scan");
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["aaaa0002", "bbbb0001"],
        "the excluded id is gone and nothing else is"
    );
}

#[test]
fn repo_and_exclude_compose() {
    let (_d, conn) = seeded();
    let rows = live_simhashes(&conn, Some("alpha"), Some("aaaa0001")).expect("scan");
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["aaaa0002"],
        "both predicates apply together"
    );
}

#[test]
fn unhashed_rows_are_returned_for_the_caller_to_judge() {
    let (dir, conn) = seeded();
    insert(&conn, "dddd0001", "alpha", 0, None);
    let rows = live_simhashes(&conn, None, None).expect("scan");
    assert!(
        rows.iter().any(|r| r.id == "dddd0001" && r.simhash == 0),
        "a zero fingerprint is data, not a filter decision the store makes"
    );
    drop(dir);
}

#[test]
fn an_empty_corpus_scans_to_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    assert!(
        live_simhashes(&conn, None, None).expect("scan").is_empty(),
        "no live rows means no scan rows, not an error"
    );
}
