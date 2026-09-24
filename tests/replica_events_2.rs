#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Activity events over the real surface (#254): real command runs recorded by
//! the real cores, captured into the journal by the real sweep, and relayed
//! between spawned `comemory serve` engines over real HTTP.
//!
//! Covers origin and canonical scope (AC-2), the exchange reaching a fixed
//! point (AC-8), display settings on either side (AC-9), what text may leave
//! (AC-10), retention (AC-11) and acceptance order under tied timestamps
//! (AC-13). Verdicts are `replica_events.rs` and `replica_events_3.rs`.

#[path = "common/replica_support.rs"]
mod replica_support;

#[path = "common/replica_events_support.rs"]
mod replica_events_support;

use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};

use replica_events_support::{
    CANONICAL, MEMORY_QUERY, activity_rows, age, approve, changes, count, device_of, dispositions,
    feedback, import, journal, memory_counters, operation_of, pair, reissued, transfer,
};
use replica_support::{BODY, Engine, cli_with_env};
use serde_json::{Value, json};

const KIND: &str = "activity_event";
const EVENT_KINDS: [&str; 2] = ["feedback_event", KIND];

/// What a shared summary may carry for each command, plus the withheld marker.
fn allowed(command: &str) -> &'static [&'static str] {
    match command {
        "save" => &["id", "kind", "tags", "supersedes"],
        "update" => &["id", "fields"],
        "delete" | "restore" => &["id"],
        "search" | "context" => &["query", "hits", "query_withheld"],
        "find" => &["query", "hits", "total", "query_withheld"],
        "search-code" => &["query", "hits", "lang", "query_withheld"],
        "feedback" => &[
            "used",
            "irrelevant",
            "used_code",
            "irrelevant_code",
            "provenance",
        ],
        "index-code" => &["files", "mode"],
        other => panic!("command {other} is not shareable"),
    }
}

fn find_as(engine: &Engine, actor: Option<&str>, query: &str, repo: Option<&str>) -> Value {
    let mut args = vec!["find", query];
    if let Some(repo) = repo {
        args.extend(["--repo", repo]);
    }
    let env: Vec<(&str, &str)> = actor.map(|a| ("COMEMORY_ACTOR", a)).into_iter().collect();
    let (code, body) = cli_with_env(&engine.data_dir(), &env, &args);
    assert_eq!(code, 0, "find failed: {body}");
    body
}

/// A fresh data directory holding `config`, and an engine serving it.
fn configured(config: &str) -> (tempfile::TempDir, Engine) {
    let home = tempfile::tempdir().expect("home");
    let data_dir = home.path().join(".comemory");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    std::fs::write(data_dir.join("config.toml"), config).expect("config");
    let engine = Engine::spawn_at(&data_dir, &[]);
    (home, engine)
}

#[test]
fn an_activity_row_travels_with_its_origin_and_a_canonical_repo() {
    let p = pair();
    find_as(
        &p.a,
        Some("replica-test/1.0"),
        MEMORY_QUERY,
        Some("checkout-a"),
    );

    let entries = changes(&p.a, &[KIND]);
    let rows = activity_rows(&p.a.data_dir());
    let device = device_of(&p.a.data_dir());
    assert_eq!(
        entries.len(),
        rows.len(),
        "every scoped, approved run is shared: {rows:?}"
    );
    for entry in &entries {
        let payload = &entry["payload"];
        let row = rows
            .iter()
            .find(|r| r["event_id"] == payload["event_id"])
            .unwrap_or_else(|| panic!("no row for {payload}"));
        assert_eq!(payload["device"], json!(device));
        assert_eq!(payload["at"], row["at"]);
        assert_eq!(payload["source"], row["source"]);
        assert_eq!(payload["command"], row["command"]);
        assert_eq!(payload["repo"], json!(CANONICAL), "the canonical scope");
        let command = payload["command"].as_str().expect("command");
        for key in payload["summary"].as_object().expect("summary").keys() {
            assert!(
                allowed(command).contains(&key.as_str()),
                "{command} shared {key}"
            );
        }
    }
    let mine = entries
        .iter()
        .find(|e| e["payload"]["actor"] == json!("replica-test/1.0"))
        .expect("the actor's find was shared");
    assert_eq!(mine["payload"]["command"], json!("find"));
    assert_eq!(mine["payload"]["summary"]["query"], json!(MEMORY_QUERY));

    let results = transfer(&p.a, &p.b, &[KIND]);
    assert!(dispositions(&results).iter().all(|d| d == "accepted"));
    let imported = activity_rows(&p.b.data_dir());
    assert_eq!(imported.len(), rows.len());
    for row in &imported {
        assert_eq!(row["device"], json!(device), "the sender's run, not ours");
        assert_eq!(
            row["repo"],
            json!("checkout-b"),
            "under the receiver's label"
        );
    }
    find_as(&p.b, None, MEMORY_QUERY, Some("checkout-b"));
    let (status, feed) = p.b.get("/api/v1/activity?limit=50");
    assert_eq!(status, 200, "{feed}");
    let items = feed["data"]["items"].as_array().expect("items");
    assert_eq!(items.len(), rows.len() + 1);
    assert_eq!(
        items[0]["device"],
        Value::Null,
        "the receiver's own newest run"
    );
    assert!(items[1..].iter().all(|i| i["device"] == json!(device)));

    // The state an upgrade leaves: rows recorded before capture existed.
    p.a.db()
        .execute_batch(
            "UPDATE activity_log SET event_id = NULL, device = NULL; \
             DELETE FROM replica_feed WHERE entity_kind = 'activity_event'; \
             DELETE FROM replica_revision WHERE entity_kind = 'activity_event'; \
             DELETE FROM replica_payload WHERE entity_kind = 'activity_event'; \
             DELETE FROM schema_meta WHERE key LIKE 'replica_activity_capture%';",
        )
        .expect("upgrade state");
    let first = changes(&p.a, &[KIND]);
    let ids: Vec<Value> = activity_rows(&p.a.data_dir())
        .into_iter()
        .map(|r| r["event_id"].clone())
        .collect();
    assert_eq!(first.len(), rows.len(), "legacy rows are captured once");
    let second = changes(&p.a, &[KIND]);
    assert_eq!(second.len(), first.len(), "and never again");
    let again: Vec<Value> = activity_rows(&p.a.data_dir())
        .into_iter()
        .map(|r| r["event_id"].clone())
        .collect();
    assert_eq!(again, ids, "with stable ids");
}

#[test]
fn the_exchange_reaches_a_fixed_point() {
    let p = pair();
    feedback(&p.a, None, &p.scene, true);
    let own =
        p.b.cli(&["save", BODY, "--kind", "decision", "--repo", "checkout-b"]);
    let found = find_as(&p.b, None, MEMORY_QUERY, Some("checkout-b"));
    p.b.cli(&[
        "feedback",
        found["query_id"].as_str().expect("query"),
        "--used",
        own["id"].as_str().expect("id"),
    ]);
    // A real legacy import on B, which records a `sync.import` row there.
    let (_, legacy) = p.a.get("/api/v1/sync/changes?since=0&limit=10");
    let entry = &legacy["data"]["entries"][0];
    let (status, imported) = p.b.post(
        "/api/v1/sync/import",
        &json!({"cursor": 0, "entries": [{
            "op": entry["op"], "id": entry["id"], "content_hash": entry["content_hash"],
            "at": entry["at"], "record": entry["record"],
        }]}),
    );
    assert_eq!(status, 200, "legacy import: {imported}");

    let snapshot = |engine: &Engine| {
        let dir = engine.data_dir();
        let local: i64 = engine
            .db()
            .query_row(
                "SELECT count(*) FROM replica_feed WHERE origin = 'local'",
                [],
                |r| r.get(0),
            )
            .expect("local");
        (
            count(&dir, "replica_feed"),
            local,
            count(&dir, "replica_operation"),
            count(&dir, "activity_log"),
            count(&dir, "feedback_events"),
        )
    };
    let mut settled = None;
    for round in 0..5 {
        transfer(&p.a, &p.b, &EVENT_KINDS);
        transfer(&p.b, &p.a, &EVENT_KINDS);
        for engine in [&p.a, &p.b] {
            let (code, body) = cli_with_env(
                &engine.data_dir(),
                &[],
                &["sync", "--action", "auto", "--json"],
            );
            assert_eq!(code, 0, "reconcile pass: {body}");
        }
        let now = (snapshot(&p.a), snapshot(&p.b));
        match &settled {
            None => settled = Some(now),
            Some(first) => assert_eq!(&now, first, "round {round} moved the fixed point"),
        }
    }
    for engine in [&p.a, &p.b] {
        let echoed: i64 = engine
            .db()
            .query_row(
                "SELECT count(*) FROM replica_feed WHERE origin = 'local' AND entity_key IN ( \
                   SELECT event_id FROM activity_log WHERE device IS NOT NULL \
                   UNION SELECT event_id FROM feedback_events WHERE device IS NOT NULL)",
                [],
                |r| r.get(0),
            )
            .expect("echoed");
        assert_eq!(echoed, 0, "an imported event is never journalled as ours");
    }
    let bookkeeping: Vec<Value> = activity_rows(&p.b.data_dir())
        .into_iter()
        .filter(|r| r["command"] == json!("sync.import"))
        .collect();
    assert_eq!(bookkeeping.len(), 1, "the legacy import recorded its row");
    assert!(bookkeeping[0]["event_id"].is_null(), "and it never left");
    assert!(
        journal(&p.b.data_dir(), KIND)
            .iter()
            .all(|e| e["payload"]["command"] != json!("sync.import"))
    );
}

#[test]
fn display_settings_are_respected_on_either_side() {
    let (_quiet_home, quiet) = configured("[activity]\nsummaries = false\n");
    approve(&quiet, &[("checkout-q", CANONICAL)]);
    quiet.cli(&["save", BODY, "--kind", "decision", "--repo", "checkout-q"]);
    find_as(&quiet, None, MEMORY_QUERY, Some("checkout-q"));
    let entries = changes(&quiet, &[KIND]);
    let shared_find = entries
        .iter()
        .find(|e| e["payload"]["command"] == json!("find"))
        .expect("the find was shared");
    assert_eq!(shared_find["payload"]["summary"], Value::Null);
    let everything = serde_json::to_string(&changes(&quiet, &[])).expect("json");
    assert!(
        !everything.contains(MEMORY_QUERY),
        "no query text in changes"
    );
    let stored: i64 = quiet
        .db()
        .query_row(
            "SELECT count(*) FROM replica_payload WHERE bytes LIKE ?1",
            [format!("%{MEMORY_QUERY}%")],
            |r| r.get(0),
        )
        .expect("payload scan");
    assert_eq!(stored, 0, "nor in any journal copy");

    let (_off_home, off) = configured("[activity]\nenabled = false\n");
    approve(&off, &[("checkout-o", CANONICAL)]);
    off.cli(&["save", BODY, "--kind", "decision", "--repo", "checkout-o"]);
    find_as(&off, None, MEMORY_QUERY, Some("checkout-o"));
    assert!(
        changes(&off, &[KIND]).is_empty(),
        "nothing recorded, nothing shared"
    );

    let p = pair();
    let sent = changes(&p.a, &[KIND]).len();
    assert!(sent > 0);
    let (_r1_home, deaf) = configured("[activity]\nenabled = false\n");
    let results = transfer(&p.a, &deaf, &[KIND]);
    assert!(dispositions(&results).iter().all(|d| d == "accepted"));
    assert_eq!(count(&deaf.data_dir(), "activity_log"), 0, "no row shown");
    assert_eq!(
        journal(&deaf.data_dir(), KIND).len(),
        sent,
        "positions kept"
    );
    assert_eq!(
        count(&deaf.data_dir(), "replica_receipt"),
        i64::try_from(sent).unwrap()
    );

    let (_r2_home, terse) = configured("[activity]\nsummaries = false\n");
    transfer(&p.a, &terse, &[KIND]);
    let rows = activity_rows(&terse.data_dir());
    assert_eq!(rows.len(), sent);
    assert!(rows.iter().all(|r| r["summary"].is_null()), "{rows:?}");
}

#[test]
fn only_approved_scoped_text_leaves_and_a_secret_never_does() {
    // AWS's documented example key id, assembled from parts the way
    // `utilities::secret_scan`'s own tests do, so this fixture is not itself a
    // committed credential to the repository's secret gate.
    let secret = format!("AKIA{}{}", "IOSFODNN7", "EXAMPLE");
    let secret = secret.as_str();
    let home = tempfile::tempdir().expect("home");
    let data_dir = home.path().join(".comemory");
    let log = home.path().join("serve.log");
    let a = Engine::spawn_logged(&data_dir, &log);
    approve(&a, &[("checkout-a", CANONICAL)]);
    a.cli(&["save", BODY, "--kind", "decision", "--repo", "checkout-a"]);
    find_as(&a, None, "unscoped durable memory", None);
    find_as(&a, None, "unapproved durable memory", Some("elsewhere"));
    find_as(
        &a,
        None,
        &format!("rotate {secret} before release"),
        Some("checkout-a"),
    );
    find_as(
        &a,
        None,
        "why does /Users/alice/work/app/src/main.rs panic",
        Some("checkout-a"),
    );

    let entries = changes(&a, &[KIND]);
    let finds: Vec<&Value> = entries
        .iter()
        .filter(|e| e["payload"]["command"] == json!("find"))
        .collect();
    assert_eq!(
        finds.len(),
        2,
        "only the two approved, scoped finds: {finds:?}"
    );
    let withheld = finds
        .iter()
        .find(|e| e["payload"]["summary"]["query_withheld"] == json!(true))
        .expect("the secret-bearing query was withheld");
    assert!(withheld["payload"]["summary"].get("query").is_none());
    assert!(
        finds
            .iter()
            .any(|e| e["payload"]["summary"]["query"] == json!("why does <path> panic")),
        "the machine path was stripped: {finds:?}"
    );

    let feed = serde_json::to_string(&changes(&a, &[])).expect("json");
    let (_, frames) = a.get("/api/v1/sync/replica/events?since=0");
    let stored: i64 = a
        .db()
        .query_row(
            "SELECT count(*) FROM replica_payload WHERE bytes LIKE ?1",
            [format!("%{secret}%")],
            |r| r.get(0),
        )
        .expect("payload scan");
    assert_eq!(stored, 0, "no journal copy holds the secret");
    assert!(!feed.contains(secret), "nor any changes entry");
    assert!(!frames.to_string().contains(secret), "nor any event frame");
    let logged = std::fs::read_to_string(&log).expect("serve log");
    assert!(!logged.is_empty(), "the log was captured at debug level");
    assert!(!logged.contains(secret), "nor any serve log line");
    assert!(!feed.contains("/Users/alice"), "no machine path either");

    let kinds: std::collections::BTreeSet<String> = changes(&a, &[])
        .iter()
        .map(|e| e["entity_kind"].as_str().expect("kind").to_string())
        .collect();
    let permitted = [
        "memory",
        "code_generation",
        "document_revision",
        "feedback_event",
        KIND,
    ];
    assert!(
        kinds.iter().all(|k| permitted.contains(&k.as_str())),
        "{kinds:?}"
    );

    let b = Engine::spawn(&[]);
    transfer(&a, &b, &[]);
    for table in [
        "retrieval_log",
        "candidate_query_observations",
        "candidate_observations",
        "candidate_judgments",
        "bandit_arms",
    ] {
        assert_eq!(count(&b.data_dir(), table), 0, "{table} stays local");
    }

    let entry = finds[0];
    let mut payload = entry["payload"].clone();
    payload["summary"]["top"] = json!(["c2468e46"]);
    let crafted = reissued(entry, &payload);
    let fresh = Engine::spawn(&[]);
    assert_eq!(
        dispositions(&import(&fresh, &[crafted])),
        vec!["rejected_invalid"],
        "a key outside the allowlist is refused"
    );
}

#[test]
fn expired_events_keep_their_dedupe_metadata_and_are_answered_typed() {
    let p = pair();
    feedback(&p.a, None, &p.scene, false);
    changes(&p.a, &[]);
    transfer(&p.a, &p.b, &EVENT_KINDS);
    let before =
        journal(&p.a.data_dir(), KIND).len() + journal(&p.a.data_dir(), "feedback_event").len();

    age(&p.a.data_dir(), "feedback_events", "1 = 1");
    age(&p.a.data_dir(), "activity_log", "1 = 1");
    let (code, _) = cli_with_env(&p.a.data_dir(), &[], &["gc"]);
    assert_eq!(code, 0);

    let expired = changes(&p.a, &EVENT_KINDS);
    assert_eq!(expired.len(), before, "every position stays");
    for entry in &expired {
        assert_eq!(entry["payload_state"], json!("expired"), "{entry}");
        assert!(entry["payload_digest"].is_string());
        assert!(entry["payload"].is_null());
    }
    let revisions: i64 =
        p.a.db()
            .query_row(
                "SELECT count(*) FROM replica_revision \
              WHERE entity_kind IN ('feedback_event', 'activity_event')",
                [],
                |r| r.get(0),
            )
            .expect("revisions");
    assert_eq!(
        revisions,
        i64::try_from(before).unwrap(),
        "dedupe metadata kept"
    );
    assert_eq!(count(&p.a.data_dir(), "feedback_events"), 0, "detail gone");

    let echo = transfer(&p.b, &p.a, &EVENT_KINDS);
    assert!(
        dispositions(&echo).iter().all(|d| d == "payload_expired"),
        "{echo:?}"
    );
    assert_eq!(
        memory_counters(&p.a.data_dir()),
        vec![(p.scene.memory.clone(), 1, 0)],
        "no second contribution"
    );

    let digest_only = operation_of(&expired[0]);
    assert!(digest_only.get("payload").is_none());
    let fresh = Engine::spawn(&[]);
    assert_eq!(
        dispositions(&import(&fresh, std::slice::from_ref(&digest_only))),
        vec!["payload_expired"]
    );
    assert_eq!(
        dispositions(&import(&fresh, &[digest_only])),
        vec!["payload_expired"]
    );
    assert!(memory_counters(&fresh.data_dir()).is_empty());

    let relay = Engine::spawn(&[]);
    let page = changes(&p.a, &[]);
    let answered = transfer(&p.a, &relay, &[]);
    assert_eq!(answered.len(), page.len(), "every operation answered");
    assert!(
        dispositions(&answered)
            .iter()
            .all(|d| d == "accepted" || d == "payload_expired"),
        "{answered:?}"
    );
}

/// Read `want` SSE `activity` events' summary queries from `engine`.
fn streamed_queries(engine: &Engine, want: usize) -> Vec<Value> {
    let response = reqwest::blocking::Client::builder()
        .timeout(None)
        .build()
        .expect("client")
        .get(format!("{}/api/v1/activity/events?after_id=0", engine.base))
        .bearer_auth(&engine.token)
        .send()
        .expect("open stream");
    assert!(response.status().is_success());
    let mut reader = BufReader::new(response);
    let started = Instant::now();
    let mut out = Vec::new();
    let mut line = String::new();
    while out.len() < want {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "stream stalled"
        );
        line.clear();
        reader.read_line(&mut line).expect("read");
        if let Some(data) = line.trim_end().strip_prefix("data:") {
            let row: Value = serde_json::from_str(data.trim()).expect("row json");
            out.push(row["summary"]["query"].clone());
        }
    }
    out
}

/// Page `engine`'s activity positions one at a time; their summary queries.
fn paged_queries(engine: &Engine) -> Vec<Value> {
    let mut since = 0;
    let mut out = Vec::new();
    loop {
        let (_, body) = engine.get(&format!(
            "/api/v1/sync/replica/changes?since={since}&limit=1&kind={KIND}"
        ));
        let Some(next) = body["data"]["next_sequence"].as_i64() else {
            return out;
        };
        out.push(body["data"]["entries"][0]["payload"]["summary"]["query"].clone());
        since = next;
    }
}

#[test]
fn tied_timestamps_keep_acceptance_order() {
    let p = pair();
    find_as(&p.a, None, "first tied query", Some("checkout-a"));
    find_as(&p.a, None, "second tied query", Some("checkout-a"));
    let entries = changes(&p.a, &[KIND]);
    let pick = |q: &str| {
        entries
            .iter()
            .find(|e| e["payload"]["summary"]["query"] == json!(q))
            .unwrap_or_else(|| panic!("no event for {q}"))
            .clone()
    };
    let (x, y) = (pick("first tied query"), pick("second tied query"));
    let instant = x["payload"]["at"].clone();
    let mut x_payload = x["payload"].clone();
    let mut y_payload = y["payload"].clone();
    x_payload["at"] = instant.clone();
    y_payload["at"] = instant;
    let (x_op, y_op) = (reissued(&x, &x_payload), reissued(&y, &y_payload));

    let forward = Engine::spawn(&[]);
    import(&forward, &[x_op.clone(), y_op.clone()]);
    let order = vec![json!("first tied query"), json!("second tied query")];
    assert_eq!(paged_queries(&forward), order);
    assert_eq!(streamed_queries(&forward, 2), order);

    let reverse = Engine::spawn(&[]);
    import(&reverse, &[y_op, x_op]);
    let reversed: Vec<Value> = order.into_iter().rev().collect();
    assert_eq!(paged_queries(&reverse), reversed);
    assert_eq!(streamed_queries(&reverse, 2), reversed);
}
