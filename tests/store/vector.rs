//! Test mirror for `src/store/vector.rs`.
//!
//! Exercises `insert_memory` / `knn_memory` end-to-end against a real
//! `sqlite-vec`-backed connection plus the dim guard surfaced through
//! the schema_meta lookup.

use comemory::store::{connection, vector};
use tempfile::tempdir;

use crate::vectors;

#[test]
fn insert_and_knn_returns_nearest_neighbor() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    // Seed a row in memories so the FK in memory_vec.memory_id is satisfiable
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES('aaaa1111','a','note','hash1','body','2026-06-08T00:00:00Z','2026-06-08T00:00:00Z','a.md')",
        [],
    ).expect("seed memories");
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES('bbbb2222','b','note','hash2','body','2026-06-08T00:00:00Z','2026-06-08T00:00:00Z','b.md')",
        [],
    ).expect("seed memories");

    let v_a = vectors::vector("alpha", 1024);
    let v_b = vectors::vector("beta", 1024);
    let v_q = v_a.clone();

    vector::insert_memory(&conn, "aaaa1111", &v_a).expect("insert a");
    vector::insert_memory(&conn, "bbbb2222", &v_b).expect("insert b");

    let hits = vector::knn_memory(&conn, &v_q, 1, None).expect("knn");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory_id, "aaaa1111");
}

#[test]
fn knn_memory_with_repo_filter_returns_full_k_under_cross_repo_corpus() {
    // Regression for PR #3 review thread: vec0 returns the global nearest k
    // and the `m.repo = ?` JOIN runs *after*. If we don't oversample, a
    // corpus where the target repo is a minority returns far fewer than k
    // hits to the caller.
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    // 5 memories in repo "other", 3 in repo "target". Query vector is a
    // copy of the closest "other" vector, so the global top-5 by cosine
    // distance are all in repo "other". Without oversampling, asking for
    // k=3 under --repo=target would return 0 hits.
    for i in 0..5 {
        let id = format!("oth{i:05}");
        conn.execute(
            "INSERT INTO memories(id,slug,kind,repo,content_hash,body,created_at,updated_at,md_path) \
             VALUES(?1,?1,'note','other','h','b','2026-06-08T00:00:00Z','2026-06-08T00:00:00Z','x.md')",
            [&id],
        ).expect("seed other");
        let v = vectors::vector(&format!("other-{i}"), 1024);
        vector::insert_memory(&conn, &id, &v).expect("insert other");
    }
    for i in 0..3 {
        let id = format!("tgt{i:05}");
        conn.execute(
            "INSERT INTO memories(id,slug,kind,repo,content_hash,body,created_at,updated_at,md_path) \
             VALUES(?1,?1,'note','target','h','b','2026-06-08T00:00:00Z','2026-06-08T00:00:00Z','x.md')",
            [&id],
        ).expect("seed target");
        let v = vectors::vector(&format!("target-{i}"), 1024);
        vector::insert_memory(&conn, &id, &v).expect("insert target");
    }
    // Query vector identical to "other-0" — the global nearest neighbours
    // are all in repo "other", forcing the JOIN to do real filtering.
    let q = vectors::vector("other-0", 1024);

    let hits = vector::knn_memory(&conn, &q, 3, Some("target")).expect("knn target");
    assert_eq!(
        hits.len(),
        3,
        "expected full k=3 hits from repo 'target', got {}",
        hits.len()
    );
    assert!(
        hits.iter().all(|h| h.memory_id.starts_with("tgt")),
        "all hits must be from repo 'target': {:?}",
        hits.iter().map(|h| &h.memory_id).collect::<Vec<_>>()
    );
}

/// A real 768-dim (the `code_vec` dim) unit vector with a single `1.0`
/// component; distinct indices are exactly orthogonal.
fn code_basis(idx: usize) -> Vec<f32> {
    let mut v = vec![0.0f32; 768];
    v[idx] = 1.0;
    v
}

/// Insert one `code_symbols` row so the knn_code repo/lang post-filter has
/// scope columns to consult.
fn seed_code_symbol(conn: &rusqlite::Connection, id: i64, repo: &str, lang: &str) {
    conn.execute(
        "INSERT INTO code_symbols\
            (id,repo,path,blob_oid,symbol,kind,lang,line_start,line_end,snippet,simhash,indexed_at) \
         VALUES(?1,?2,'src/f.rs','oid','sym'||?1,'function',?3,1,10,'snippet',0,'t')",
        rusqlite::params![id, repo, lang],
    )
    .expect("seed code symbol");
}

#[test]
fn insert_and_knn_code_returns_nearest_neighbor() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    seed_code_symbol(&conn, 1, "webapp", "rust");
    seed_code_symbol(&conn, 2, "webapp", "rust");
    vector::insert_code(&conn, 1, &code_basis(0)).expect("insert 1");
    vector::insert_code(&conn, 2, &code_basis(1)).expect("insert 2");

    let hits = vector::knn_code(&conn, &code_basis(0), 1, None, None).expect("knn");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].symbol_id, 1);
    assert!(
        hits[0].distance < 1e-5,
        "identical vector must be distance 0"
    );
}

#[test]
fn knn_code_repo_and_lang_filters_restrict() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    seed_code_symbol(&conn, 1, "frontend", "rust");
    seed_code_symbol(&conn, 2, "backend", "python");
    // Both vectors are equally near the query so only the post-filter can
    // separate them.
    vector::insert_code(&conn, 1, &code_basis(0)).expect("insert 1");
    vector::insert_code(&conn, 2, &code_basis(0)).expect("insert 2");
    let q = code_basis(0);

    let by_repo = vector::knn_code(&conn, &q, 10, Some("backend"), None).expect("repo filter");
    assert_eq!(by_repo.len(), 1, "repo filter must drop the frontend row");
    assert_eq!(by_repo[0].symbol_id, 2);

    let by_lang = vector::knn_code(&conn, &q, 10, None, Some("rust")).expect("lang filter");
    assert_eq!(by_lang.len(), 1, "lang filter must drop the python row");
    assert_eq!(by_lang[0].symbol_id, 1);

    let both = vector::knn_code(&conn, &q, 10, Some("frontend"), Some("python"))
        .expect("conjunctive filter");
    assert!(both.is_empty(), "repo and lang filters are conjunctive");

    let unfiltered = vector::knn_code(&conn, &q, 10, None, None).expect("unfiltered");
    assert_eq!(unfiltered.len(), 2, "no filter must keep both rows");
}

#[test]
fn insert_rejects_mismatched_dim() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES('cccc3333','c','note','hash3','body','2026-06-08T00:00:00Z','2026-06-08T00:00:00Z','c.md')",
        [],
    ).expect("seed memories");

    let bad = vectors::vector("c", 16);
    let err = vector::insert_memory(&conn, "cccc3333", &bad).expect_err("dim mismatch");
    assert!(format!("{err}").contains("dim mismatch"));
}
