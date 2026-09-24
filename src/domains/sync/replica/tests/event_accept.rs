#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Accepting feedback and activity events: one contribution per event, typed
//! answers for everything else, and the receiver's own display settings.

use crate::domains::learning::code_feedback::record_code_with_provenance;
use crate::domains::learning::feedback_tracking::record_with_provenance;
use crate::domains::learning::telemetry::StatsDb;
use crate::domains::sync::replica::accept;
use crate::domains::sync::replica::contract::Disposition;
use crate::store::code_row::{self, CodeSymbolRow};
use crate::store::replica_journal::ReplicaOp;
use crate::store::{replica_read, replica_redaction};
use crate::utilities::telemetry::PROV_MANUAL;
use rusqlite::Connection;

use crate::domains::sync::replica::test_support as support;

use support::{BODY, Home, approve, envelope, journalled_ops};

/// An author with one approved memory and `verdicts` journalled `used`
/// verdicts on it.
fn author(verdicts: usize) -> (Home, String) {
    let mut home = Home::new();
    approve(&home.conn, "Falconiere/comemory");
    let id = home.save(BODY, &["sync"]);
    let mut db = StatsDb::open(home.paths.stats_db()).expect("stats");
    record_with_provenance(
        &mut db,
        "q-20260924-1a2b3c4d",
        &vec![id.clone(); verdicts],
        &[],
        PROV_MANUAL,
    )
    .expect("verdicts");
    (home, id)
}

fn dispositions(
    peer: &mut Home,
    ops: Vec<crate::domains::sync::replica::contract::Operation>,
) -> Vec<Disposition> {
    let mut ctx = peer.ctx();
    accept::run(&mut ctx, envelope(ops))
        .expect("import")
        .results
        .into_iter()
        .map(|r| r.disposition)
        .collect()
}

fn scalar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("scalar")
}

#[test]
fn an_imported_verdict_lands_once_attributed_to_its_origin() {
    let (author, id) = author(1);
    let ops = journalled_ops(&author.conn, "feedback_event");
    let mut peer = Home::new();

    assert_eq!(
        dispositions(&mut peer, ops.clone()),
        vec![Disposition::Accepted]
    );

    let device = crate::store::replica_device::id(&author.conn).expect("device");
    let (row_device, query): (String, String) = peer
        .conn
        .query_row("SELECT device, query_id FROM feedback_events", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("row");
    assert_eq!(row_device, device);
    assert_eq!(query, format!("{device}:q-20260924-1a2b3c4d"));
    assert_eq!(
        scalar(
            &peer.conn,
            &format!("SELECT used_count FROM feedback WHERE memory_id = '{id}'")
        ),
        1
    );
    let page = replica_read::page(&peer.conn, 0, 10, Some("feedback_event")).expect("page");
    assert_eq!(
        page[0].origin,
        crate::store::replica_journal::ReplicaOrigin::Sync
    );
    assert_eq!(
        scalar(&peer.conn, "SELECT count(*) FROM replica_operation"),
        0,
        "never re-offered"
    );
}

#[test]
fn the_same_event_under_any_operation_id_counts_once() {
    let (author, id) = author(1);
    let ops = journalled_ops(&author.conn, "feedback_event");
    let mut peer = Home::new();
    let mut ctx = peer.ctx();
    let first = accept::run(&mut ctx, envelope(ops.clone()))
        .expect("first")
        .results;

    let mut renamed = ops[0].clone();
    renamed.operation_id = "op-20260924-relayed1".to_string();
    let mut ctx = peer.ctx();
    let again = accept::run(&mut ctx, envelope(vec![ops[0].clone(), renamed.clone()]))
        .expect("again")
        .results;
    assert_eq!(again[0].disposition, Disposition::Duplicate, "a replay");
    assert_eq!(
        again[1].disposition,
        Disposition::Duplicate,
        "an echo under a new id"
    );
    assert_eq!(
        again[1].sequence, first[0].sequence,
        "answered with the first position"
    );
    let mut ctx = peer.ctx();
    let replayed = accept::run(&mut ctx, envelope(vec![renamed]))
        .expect("replay")
        .results;
    assert_eq!(
        replayed[0].sequence, first[0].sequence,
        "the receipt keeps it"
    );
    assert_eq!(
        scalar(
            &peer.conn,
            &format!("SELECT used_count FROM feedback WHERE memory_id = '{id}'")
        ),
        1
    );
}

#[test]
fn one_event_id_cannot_name_two_events() {
    let (author, _) = author(1);
    let ops = journalled_ops(&author.conn, "feedback_event");
    let mut peer = Home::new();
    dispositions(&mut peer, ops.clone());

    let mut forged = ops[0].clone();
    forged.operation_id = "op-20260924-forged01".to_string();
    let mut payload = forged.payload.clone().expect("payload");
    payload["verdict"] = serde_json::json!("irrelevant");
    let (_, digest) = crate::utilities::canonical_json::bytes_and_digest(&payload).expect("digest");
    forged.payload = Some(payload);
    forged.payload_digest = Some(digest);
    assert_eq!(
        dispositions(&mut peer, vec![forged]),
        vec![Disposition::RejectedConflict]
    );
    assert_eq!(
        scalar(&peer.conn, "SELECT count(*) FROM feedback_events"),
        1
    );
}

#[test]
fn history_without_its_bytes_is_answered_expired_and_counts_nothing() {
    let (author, _) = author(1);
    let mut digest_only = journalled_ops(&author.conn, "feedback_event").remove(0);
    digest_only.payload = None;
    let mut peer = Home::new();
    assert_eq!(
        dispositions(&mut peer, vec![digest_only.clone()]),
        vec![Disposition::PayloadExpired]
    );
    assert_eq!(
        dispositions(&mut peer, vec![digest_only]),
        vec![Disposition::PayloadExpired]
    );
    assert_eq!(scalar(&peer.conn, "SELECT count(*) FROM feedback"), 0);
    assert!(
        replica_read::page(&peer.conn, 0, 10, Some("feedback_event"))
            .expect("page")
            .is_empty()
    );
}

#[test]
fn an_expired_or_erased_copy_answers_its_own_disposition() {
    let (author, _) = author(2);
    let ops = journalled_ops(&author.conn, "feedback_event");
    let mut peer = Home::new();
    dispositions(&mut peer, ops.clone());
    replica_redaction::expire_events_before(
        &peer.conn,
        "2999-01-01T00:00:00Z",
        "2026-09-24T10:00:00Z",
    )
    .expect("expire");
    let event = ops[1].entity_key.clone();
    replica_redaction::erase_verdicts(&peer.conn, &[event], "2026-09-24T10:00:00Z").expect("erase");

    let mut renamed: Vec<_> = ops.clone();
    for (n, op) in renamed.iter_mut().enumerate() {
        op.operation_id = format!("op-20260924-echo000{n}");
    }
    assert_eq!(
        dispositions(&mut peer, renamed),
        vec![Disposition::PayloadExpired, Disposition::PayloadErased]
    );
}

#[test]
fn an_event_is_never_tombstoned_or_restored() {
    let (author, _) = author(1);
    let mut op = journalled_ops(&author.conn, "feedback_event").remove(0);
    op.op = ReplicaOp::Tombstone;
    op.payload = None;
    op.payload_digest = None;
    let mut peer = Home::new();
    assert_eq!(
        dispositions(&mut peer, vec![op]),
        vec![Disposition::RejectedInvalid]
    );
}

#[test]
fn a_code_verdict_is_keyed_by_the_receivers_label_or_the_canonical_name() {
    let author = Home::new();
    approve(&author.conn, "checkout-a");
    let symbol = code_row::insert(
        &author.conn,
        &CodeSymbolRow {
            repo: "checkout-a",
            path: "src/store/feedback.rs",
            blob_oid: "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
            symbol: "upsert_used",
            kind: "function",
            lang: "rust",
            line_start: 23,
            line_end: 31,
            snippet: "pub(crate) fn upsert_used() {}",
            simhash: 0,
            parent_id: None,
        },
    )
    .expect("symbol");
    let mut db = StatsDb::open(author.paths.stats_db()).expect("stats");
    record_code_with_provenance(&mut db, "q-20260924-1a2b3c4d", &[symbol], &[], PROV_MANUAL)
        .expect("verdict");
    let ops = journalled_ops(&author.conn, "feedback_event");

    let mut labelled = Home::new();
    approve(&labelled.conn, "checkout-b");
    dispositions(&mut labelled, ops.clone());
    let mut hosted = Home::new();
    dispositions(&mut hosted, ops);

    let key = |home: &Home| -> String {
        home.conn
            .query_row("SELECT repo FROM code_feedback", [], |r| r.get(0))
            .expect("counter")
    };
    assert_eq!(key(&labelled), "checkout-b");
    assert_eq!(key(&hosted), "Falconiere/comemory");
}

#[test]
fn the_receivers_display_settings_govern_the_activity_row() {
    let mut author = Home::new();
    approve(&author.conn, "Falconiere/comemory");
    author.save(BODY, &["sync"]);
    let mut ctx = author.ctx();
    crate::domains::sync::replica::event_capture::advance(&mut ctx).expect("capture");
    let ops = journalled_ops(&author.conn, "activity_event");
    assert_eq!(ops.len(), 1);

    let mut hidden = Home::new();
    hidden.cfg.activity.enabled = false;
    assert_eq!(
        dispositions(&mut hidden, ops.clone()),
        vec![Disposition::Accepted]
    );
    assert_eq!(scalar(&hidden.conn, "SELECT count(*) FROM activity_log"), 0);
    assert_eq!(
        replica_read::page(&hidden.conn, 0, 10, Some("activity_event"))
            .expect("page")
            .len(),
        1,
        "the position is kept all the same"
    );

    let mut terse = Home::new();
    terse.cfg.activity.summaries = false;
    dispositions(&mut terse, ops);
    assert_eq!(
        scalar(
            &terse.conn,
            "SELECT count(*) FROM activity_log WHERE summary IS NULL AND device IS NOT NULL"
        ),
        1
    );
}
