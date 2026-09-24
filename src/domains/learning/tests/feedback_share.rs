#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Record-time journalling through the real feedback core: a real saved
//! memory, a real indexed-symbol row, a real approval map, and the real
//! transaction the verdicts commit in.

use crate::config::{Config, Paths};
use crate::domains::learning::feedback::{self, Request};
use crate::domains::learning::feedback_tracking::record_implicit_used;
use crate::domains::memories::save;
use crate::store::code_row::{self, CodeSymbolRow};
use crate::store::{connection, replica_device, repository_approval};
use crate::utilities::activity::Origin;
use crate::utilities::context::Ctx;
use crate::utilities::telemetry::{
    COACTIVATION_QUERY_ID, PROV_AUTO_COACTIVATION, PROV_AUTO_SEARCH_EDIT, SEARCH_EDIT_QUERY_ID,
};
use rusqlite::Connection;
use serde_json::{Value, json};

const CANONICAL: &str = "Falconiere/comemory";
const QUERY: &str = "q-20260924-1a2b3c4d";

struct Home {
    _dir: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    conn: Connection,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::new(dir.path());
        paths.ensure_dirs().expect("dirs");
        let conn = connection::open(paths.db_path()).expect("open");
        Self {
            _dir: dir,
            paths,
            cfg: Config::defaults(),
            conn,
        }
    }

    fn save(&mut self, body: &str, repo: &str) -> String {
        let request: save::Request =
            serde_json::from_value(json!({"body": body, "repo": repo})).expect("request");
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        save::run(&mut ctx, request, false, None).expect("save").id
    }

    fn symbol(&self, repo: &str) -> i64 {
        code_row::insert(
            &self.conn,
            &CodeSymbolRow {
                repo,
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
        .expect("symbol")
    }

    fn approve(&self, label: &str) {
        repository_approval::replace_all(
            &self.conn,
            &[(label.to_string(), CANONICAL.to_string())],
            "2026-09-24T10:00:00Z",
        )
        .expect("approve");
    }

    fn feedback(
        &mut self,
        actor: Option<&str>,
        memory: &str,
        code: Option<i64>,
    ) -> crate::prelude::Result<feedback::Response> {
        let origin = Origin::mcp(&self.cfg, actor);
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn).with_origin(origin);
        feedback::run(
            &mut ctx,
            Request {
                query_id: QUERY.to_string(),
                used: vec![memory.to_string()],
                irrelevant: Vec::new(),
                used_code: code.map(|c| c.to_string()).into_iter().collect(),
                irrelevant_code: Vec::new(),
                source: None,
            },
        )
    }

    fn journalled(&self) -> Vec<Value> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT p.bytes FROM replica_feed f JOIN replica_payload p \
                   ON p.digest = f.payload_digest \
                  WHERE f.entity_kind = 'feedback_event' ORDER BY f.sequence",
            )
            .expect("prepare");
        statement
            .query_map([], |r| r.get::<_, String>(0))
            .expect("query")
            .map(|b| serde_json::from_str(&b.expect("row")).expect("json"))
            .collect()
    }

    fn event_ids(&self) -> Vec<Option<String>> {
        let mut statement = self
            .conn
            .prepare("SELECT event_id FROM feedback_events ORDER BY id")
            .expect("prepare");
        statement
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect")
    }
}

const BODY: &str = "comemory keeps a durable, searchable memory of the decisions, bugs and \
     conventions a codebase accumulates, and links them to the code they describe.";

#[test]
fn a_scoped_verdict_is_journalled_with_its_origin_in_the_verdict_transaction() {
    let mut home = Home::new();
    let memory = home.save(BODY, "demo");
    let symbol = home.symbol("demo");
    home.approve("demo");

    home.feedback(Some("host/1.0"), &memory, Some(symbol))
        .expect("feedback");

    let device = replica_device::id(&home.conn).expect("device");
    let journalled = home.journalled();
    assert_eq!(journalled.len(), 2);
    for payload in &journalled {
        assert_eq!(payload["device"], json!(device));
        assert_eq!(payload["surface"], json!("mcp"));
        assert_eq!(payload["actor"], json!("host/1.0"));
        assert_eq!(
            payload["origin_query_id"],
            json!(format!("{device}:{QUERY}"))
        );
    }
    assert_eq!(
        journalled[0]["target"],
        json!({"kind": "memory", "id": memory})
    );
    assert_eq!(
        journalled[1]["target"],
        json!({"kind": "code", "repo": CANONICAL, "path": "src/store/feedback.rs",
               "symbol": "upsert_used", "version": "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"})
    );
    let stamped = home.event_ids();
    assert_eq!(
        stamped,
        journalled
            .iter()
            .map(|p| p["event_id"].as_str().map(str::to_string))
            .collect::<Vec<_>>(),
        "each row carries the id its payload was journalled under"
    );
}

#[test]
fn an_unapproved_or_unscoped_target_stays_local() {
    let mut home = Home::new();
    let scoped = home.save(BODY, "demo");
    let unscoped = home.save("A memory saved with no repository at all.", "");
    let symbol = home.symbol("demo");

    home.feedback(None, &scoped, Some(symbol))
        .expect("unapproved");
    home.approve("demo");
    home.feedback(None, &unscoped, None).expect("unscoped");

    assert!(home.journalled().is_empty(), "nothing leaves");
    assert_eq!(
        home.event_ids(),
        vec![None, None, None],
        "and nothing is stamped"
    );
}

#[test]
fn a_secret_bearing_actor_never_reaches_the_payload() {
    let mut home = Home::new();
    let memory = home.save(BODY, "demo");
    home.approve("demo");
    let actor = format!("wrapper AKIA{}{}", "IOSFODNN7", "EXAMPLE");

    home.feedback(Some(&actor), &memory, None)
        .expect("feedback");

    let journalled = home.journalled();
    assert_eq!(journalled[0]["actor"], Value::Null);
    let local: String = home
        .conn
        .query_row("SELECT actor FROM feedback_events", [], |r| r.get(0))
        .expect("local actor");
    assert_eq!(local, actor, "the local row keeps what the caller declared");
}

#[test]
fn a_failed_code_identity_rolls_the_journal_back_with_the_verdicts() {
    let mut home = Home::new();
    let memory = home.save(BODY, "demo");
    home.approve("demo");

    assert!(home.feedback(None, &memory, Some(9_999)).is_err());

    assert!(home.journalled().is_empty());
    assert!(home.event_ids().is_empty());
}

#[test]
fn a_search_edit_reward_is_shared_and_a_coactivation_reward_is_not() {
    let mut home = Home::new();
    let memory = home.save(BODY, "demo");
    home.approve("demo");
    let at = "2026-09-24T10:00:00.000000000Z";

    record_implicit_used(
        &home.conn,
        &memory,
        at,
        PROV_AUTO_COACTIVATION,
        COACTIVATION_QUERY_ID,
    )
    .expect("coactivation");
    record_implicit_used(
        &home.conn,
        &memory,
        at,
        PROV_AUTO_SEARCH_EDIT,
        SEARCH_EDIT_QUERY_ID,
    )
    .expect("search edit");

    let journalled = home.journalled();
    assert_eq!(
        journalled.len(),
        1,
        "one commit mined everywhere must not count everywhere"
    );
    assert_eq!(journalled[0]["provenance"], json!("auto_search_edit"));
    assert_eq!(
        journalled[0]["surface"],
        Value::Null,
        "an internal reward has no surface"
    );
    let used: i64 = home
        .conn
        .query_row("SELECT used_count FROM feedback", [], |r| r.get(0))
        .expect("counter");
    assert_eq!(used, 2, "both still count here");
}
