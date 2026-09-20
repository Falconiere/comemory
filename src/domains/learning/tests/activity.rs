#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! What `feedback` and `sync.import` record in `activity_log`, against real
//! data in a temp data dir: a verdict on a genuinely tracked query, and an
//! import batch applied locally.

use time::OffsetDateTime;

use comemory::config::{Config, Paths};
use comemory::domains::learning::feedback;
use comemory::domains::memories::MemoryStore;
use comemory::domains::memories::frontmatter::{References, Relations};
use comemory::domains::memories::id::memory_id;
use comemory::domains::memories::{self, Kind};
use comemory::domains::retrieval::search;
use comemory::domains::sync::exchange::{self, import};
use comemory::store::activity::{ActivityFilter, ActivityRow, list};
use comemory::store::connection;
use comemory::store::sync_log::SyncOp;
use comemory::utilities::context::Ctx;
use comemory::utilities::digest::sha256_hex;
use serde_json::Value;

fn rows_for(conn: &comemory::store::Connection, command: &str) -> Vec<ActivityRow> {
    let filter = ActivityFilter {
        command: Some(command),
        ..ActivityFilter::default()
    };
    list(conn, &filter, 0, 0).unwrap().0
}

fn summary_of(row: &ActivityRow) -> Value {
    serde_json::from_str(row.summary.as_deref().expect("a summary")).unwrap()
}

fn import_entry(body: &str) -> exchange::ImportEntry {
    let id = memory_id(body);
    let content_hash = sha256_hex(body.trim_end().as_bytes());
    exchange::ImportEntry {
        op: SyncOp::Upsert,
        id: id.clone(),
        content_hash: content_hash.clone(),
        at: "2026-09-20T12:00:00Z".into(),
        record: Some(exchange::SyncRecord {
            frontmatter: exchange::WireFrontmatter {
                id,
                kind: Kind::Note,
                repo: "demo".into(),
                tags: vec!["sync".into()],
                created: OffsetDateTime::now_utc(),
                quality: 3,
                schema: 1,
                content_hash,
                references: References::default(),
                relations: Relations::default(),
            },
            body: body.to_string(),
            vector: None,
        }),
    }
}

#[test]
fn a_verdict_on_a_tracked_query_records_its_target_and_provenance() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let cfg = Config::defaults();

    let saved = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        memories::save::run(
            &mut ctx,
            memories::save::Request {
                body: "pgbouncer in transaction mode fixes pool exhaustion".to_string(),
                title: None,
                kind: Kind::Decision,
                repo: "demo".to_string(),
                tags: Vec::new(),
                author: String::new(),
                quality: 3,
                supersedes: Vec::new(),
                vector: None,
                ref_file: Vec::new(),
                ref_symbol: Vec::new(),
            },
            false,
            None,
        )
        .unwrap()
    };

    // A real tracked search, so the verdict below names a query_id that
    // actually exists in `retrieval_log`.
    let query_id = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        search::run(
            &mut ctx,
            search::Request {
                query: "pgbouncer".to_string(),
                k: None,
                offset: 0,
                repo: None,
                kind: None,
                vector: None,
                since: None,
                until: None,
                as_of: None,
            },
            true,
        )
        .unwrap()
        .query_id
        .expect("a tracked search has a query id")
    };

    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        feedback::run(
            &mut ctx,
            feedback::Request {
                query_id: query_id.clone(),
                used: vec![saved.id.clone()],
                irrelevant: Vec::new(),
                used_code: Vec::new(),
                irrelevant_code: Vec::new(),
                source: None,
            },
        )
        .unwrap();
    }

    let rows = rows_for(&conn, "feedback");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].ok);
    let summary = summary_of(&rows[0]);
    assert_eq!(summary["query_id"], query_id);
    assert_eq!(summary["targets"], serde_json::json!([saved.id]));
    assert_eq!(summary["used"], 1);
    assert_eq!(summary["irrelevant"], 0);
    assert_eq!(summary["provenance"], "manual");
    assert_eq!(summary["known_query"], true);
}

#[test]
fn an_import_batch_records_how_many_entries_it_applied() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let cfg = Config::defaults();

    let req = exchange::ImportRequest {
        cursor: 0,
        entries: vec![
            import_entry("the first imported lesson"),
            import_entry("the second imported lesson"),
        ],
    };
    let applied = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        import::run(&mut ctx, req, None).unwrap()
    };
    assert_eq!(applied.results.len(), 2);

    let rows = rows_for(&conn, "sync.import");
    assert_eq!(rows.len(), 1, "one row for one batch, not one per entry");
    let summary = summary_of(&rows[0]);
    assert_eq!(summary["entries"], 2);
    assert_eq!(
        summary["applied"].as_u64().unwrap() + summary["skipped"].as_u64().unwrap(),
        2,
        "every entry is either applied or skipped: {summary}"
    );
    assert_eq!(summary["head_seq"], applied.head_seq);
}

#[test]
fn an_oversized_import_batch_records_a_failed_row() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let cfg = Config::defaults();

    let entries = (0..501)
        .map(|i| import_entry(&format!("lesson number {i}")))
        .collect();
    let refused = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        import::run(
            &mut ctx,
            exchange::ImportRequest { cursor: 0, entries },
            None,
        )
    };
    assert!(refused.is_err(), "the batch is over the 500-entry cap");

    let rows = rows_for(&conn, "sync.import");
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].ok);
    assert_eq!(rows[0].error_code.as_deref(), Some("bad_request"));
}

#[test]
fn an_import_that_restores_a_trashed_memory_records_only_the_batch() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let cfg = Config::defaults();
    let body = "a lesson that will be trashed and then restored by an import";

    // Save it, then trash it, so the import below has a restore to apply.
    let saved = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        memories::save::run(
            &mut ctx,
            memories::save::Request {
                body: body.to_string(),
                title: None,
                kind: Kind::Note,
                repo: "demo".to_string(),
                tags: Vec::new(),
                author: String::new(),
                quality: 3,
                supersedes: Vec::new(),
                vector: None,
                ref_file: Vec::new(),
                ref_symbol: Vec::new(),
            },
            false,
            None,
        )
        .unwrap()
    };
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        memories::delete::run(&mut ctx, &saved.id).unwrap();
    }
    let before_restores = rows_for(&conn, "restore").len();
    // The local delete advanced the log, so the pusher's cursor has to name
    // that head — otherwise the entry is refused as stale before it restores.
    let cursor = comemory::store::sync_log::head_seq(&conn).unwrap();

    let mut entry = import_entry(body);
    entry.op = SyncOp::Restore;
    let applied = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        import::run(
            &mut ctx,
            exchange::ImportRequest {
                cursor,
                entries: vec![entry],
            },
            None,
        )
        .unwrap()
    };
    assert_eq!(
        applied.results[0].status,
        exchange::ImportStatus::Accepted,
        "the entry must actually restore: {:?}",
        applied.results[0]
    );

    // The memory really came back — the restore ran, it just did not report
    // itself a second time.
    assert!(
        MemoryStore::new(paths.clone()).load(&saved.id).is_ok(),
        "the import restored the trashed memory"
    );
    assert_eq!(
        rows_for(&conn, "restore").len(),
        before_restores,
        "the batch already records itself as one sync.import run; a per-entry \
         restore row would report the same work twice"
    );
    assert_eq!(rows_for(&conn, "sync.import").len(), 1);
}
