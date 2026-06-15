//! Real-data tests for the DB-worker.
//!
//! Seeds real `memories`/`memory_fts` + `code_symbols` rows (production
//! writers, no mocks) into a temp `comemory.db`, then drives `run_query` and
//! the spawned worker thread. Proves the memory leg matches `pipeline::search`,
//! the code leg returns the seeded symbols ranked, and that querying never
//! mutates `retrieval_log` or `access_count` (read-only).

use comemory::config::Config;
use comemory::retrieval::pipeline::{self, PageWindow, SearchOptions};
use comemory::store::{connection, fts};
use comemory::tui::app::Tab;
use comemory::tui::worker::{self, Hits, Request};

#[path = "common/code_seed.rs"]
mod code_seed;

/// Insert two searchable memories (row + FTS) into a fresh db.
fn seed_memories(conn: &rusqlite::Connection) {
    conn.execute_batch(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema,
             content_hash, body, created_at, updated_at, md_path, simhash) VALUES
          ('aaaa0001','a','note','d','f',3,1,'h1',
           'sqlite busy timeout fix for the pool',
           '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/1.md',1),
          ('bbbb0002','b','note','d','f',3,1,'h2',
           'postgres connection pool exhausted under load',
           '2026-06-09T00:00:00Z','2026-06-09T00:00:00Z','m/2.md',2);
         INSERT INTO memory_fts(memory_id, body, tags) VALUES
          ('aaaa0001','sqlite busy timeout fix for the pool',''),
          ('bbbb0002','postgres connection pool exhausted under load','');",
    )
    .expect("seed memories");
}

/// Fresh db with seeded memories.
fn mem_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_memories(&conn);
    (dir, conn)
}

/// A first-page request.
fn request(query: &str, tab: Tab) -> Request {
    Request {
        seq: 1,
        query: query.to_string(),
        vec: None,
        repo: None,
        tab,
        window: PageWindow {
            offset: 0,
            limit: 12,
        },
        embed: None,
    }
}

fn count_log(conn: &rusqlite::Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM retrieval_log", [], |r| r.get(0))
        .expect("count retrieval_log")
}

fn sum_access(conn: &rusqlite::Connection) -> i64 {
    conn.query_row(
        "SELECT COALESCE(SUM(access_count), 0) FROM memories",
        [],
        |r| r.get(0),
    )
    .expect("sum access_count")
}

#[test]
fn memory_leg_matches_pipeline_search_order() {
    let (_dir, conn) = mem_db();
    let cfg = Config::defaults();

    let hits =
        match worker::run_query(&cfg, &conn, &request("pool", Tab::Memory)).expect("run_query") {
            Hits::Memory { hits, .. } => hits,
            Hits::Code { .. } => panic!("expected memory hits"),
        };
    assert!(!hits.is_empty(), "expected memory hits for 'pool'");

    let reference = pipeline::search(
        &cfg,
        &conn,
        "pool",
        None,
        None,
        None,
        SearchOptions {
            track: false,
            source: "search",
            window: PageWindow {
                offset: 0,
                limit: 12,
            },
        },
    )
    .expect("reference search");

    let got: Vec<&str> = hits.iter().map(|h| h.memory_id.as_str()).collect();
    let want: Vec<&str> = reference
        .hits
        .iter()
        .map(|h| h.memory_id.as_str())
        .collect();
    assert_eq!(
        got, want,
        "worker memory leg must match pipeline::search order"
    );
}

#[test]
fn memory_leg_honors_page_offset() {
    let (_dir, conn) = mem_db();
    let cfg = Config::defaults();
    let window = PageWindow {
        offset: 1,
        limit: 1,
    };
    let req = Request {
        seq: 1,
        query: "pool".to_string(),
        vec: None,
        repo: None,
        tab: Tab::Memory,
        window,
        embed: None,
    };
    let hits = match worker::run_query(&cfg, &conn, &req).expect("run_query") {
        Hits::Memory { hits, .. } => hits,
        Hits::Code { .. } => panic!("expected memory hits"),
    };

    // The worker forwards the window verbatim, so an offset page must equal the
    // same offset page from `pipeline::search` (criterion C4, deep paging).
    let reference = pipeline::search(
        &cfg,
        &conn,
        "pool",
        None,
        None,
        None,
        SearchOptions {
            track: false,
            source: "search",
            window,
        },
    )
    .expect("reference search");

    let got: Vec<&str> = hits.iter().map(|h| h.memory_id.as_str()).collect();
    let want: Vec<&str> = reference
        .hits
        .iter()
        .map(|h| h.memory_id.as_str())
        .collect();
    assert_eq!(got, want, "worker must honor the page-window offset");
}

#[test]
fn querying_is_read_only() {
    let (_dir, conn) = mem_db();
    let cfg = Config::defaults();
    let before_log = count_log(&conn);
    let before_access = sum_access(&conn);

    worker::run_query(&cfg, &conn, &request("pool", Tab::Memory)).expect("run_query");

    assert_eq!(
        count_log(&conn),
        before_log,
        "worker must not write retrieval_log"
    );
    assert_eq!(
        sum_access(&conn),
        before_access,
        "worker must not bump access_count"
    );

    // Contrast: a tracked search WOULD mutate, so the snapshot above is not
    // vacuous — it genuinely catches a write.
    pipeline::search(
        &cfg,
        &conn,
        "pool",
        None,
        None,
        None,
        SearchOptions {
            track: true,
            source: "search",
            window: PageWindow {
                offset: 0,
                limit: 12,
            },
        },
    )
    .expect("tracked search");
    assert!(
        sum_access(&conn) > before_access || count_log(&conn) > before_log,
        "a tracked search should have mutated telemetry"
    );
}

#[test]
fn code_leg_returns_seeded_symbols_ranked() {
    let (_dir, conn) = code_seed::open_db();
    // `seed_symbol` writes only `code_symbols`; pair it with `fts::index_code`
    // so the lexical leg can find the symbols (the real index-code path does
    // both).
    let a = code_seed::seed_symbol(&conn, "r", "src/a.rs", "alpha_handler");
    fts::index_code(&conn, a, "alpha_handler", "fn body() {}", "src/a.rs").expect("fts a");
    let b = code_seed::seed_symbol(&conn, "r", "src/b.rs", "beta_handler");
    fts::index_code(&conn, b, "beta_handler", "fn body() {}", "src/b.rs").expect("fts b");
    let cfg = Config::defaults();

    let hits =
        match worker::run_query(&cfg, &conn, &request("handler", Tab::Code)).expect("run_query") {
            Hits::Code { hits, .. } => hits,
            Hits::Memory { .. } => panic!("expected code hits"),
        };
    assert!(!hits.is_empty(), "expected code hits for 'handler'");

    let seeded = [a, b];
    for h in &hits {
        assert!(
            seeded.contains(&h.symbol_id),
            "unexpected symbol id {}",
            h.symbol_id
        );
    }
    for w in hits.windows(2) {
        assert!(
            w[0].parts.final_score >= w[1].parts.final_score,
            "code hits must be final_score-descending"
        );
    }
}

#[tokio::test]
async fn worker_thread_echoes_seq_and_serves() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_memories(&conn);
    let cfg = Config::defaults();

    let (req_tx, req_rx) = std::sync::mpsc::channel::<Request>();
    let (resp_tx, mut resp_rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = worker::spawn(cfg, conn, req_rx, resp_tx);

    req_tx
        .send(request("pool", Tab::Memory))
        .expect("send request");
    // The request carries seq=1; the response must echo it.
    let resp = resp_rx.recv().await.expect("recv response");
    assert_eq!(resp.seq, 1);
    match resp.result {
        Ok(Hits::Memory { hits, .. }) => assert!(!hits.is_empty(), "expected hits"),
        Ok(Hits::Code { .. }) => panic!("expected memory hits"),
        Err(e) => panic!("worker error: {e}"),
    }

    drop(req_tx); // close the channel so the worker thread exits
    handle.join().expect("join worker thread");
}
