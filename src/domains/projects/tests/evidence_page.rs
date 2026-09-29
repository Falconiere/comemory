#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/evidence_page.rs` over a real store
//! filled through the attach core: each filter alone and every combination
//! returns exactly the matching rows newest first; a stored trust this build
//! does not know reads as `invalid` and is kept by the `invalid` filter; a
//! `limit` walk visits every row once and ends on a `null` cursor; `limit`
//! 100 is accepted and 0 and 101 refused; malformed input is a `400`, an
//! unknown project a `404`, an unknown item filter an empty page; and an
//! agent without `project.read` is refused before the store opens.

use comemory::config::{Config, Paths};
use comemory::domains::projects::authority::{self, Capabilities, Envelope};
use comemory::domains::projects::{create, evidence_add, evidence_page};
use comemory::errors::Result;
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use comemory::utilities::error_code::{Class, classify};
use serde_json::{Value, json};

const PROJECT: &str = "11111111-1111-4111-8111-111111111111";
const ITEM: &str = "b0000000-0000-4000-8000-000000000002";
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

struct Home {
    _dir: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    conn: rusqlite::Connection,
    /// Attached ids, oldest first.
    ids: Vec<String>,
}

impl Home {
    /// [`PROJECT`] with the plan seed and six attached rows:
    /// 0 commit/pending, 1 test_run/self_reported on ITEM, 2 commit/pending
    /// on ITEM, 3 external_url/self_reported, 4 memory/pending on ITEM (its
    /// trust then rewritten to `bogus`), 5 deployment/self_reported on ITEM.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure_dirs().unwrap();
        let mut home = Self {
            conn: connection::open(paths.db_path()).unwrap(),
            _dir: dir,
            paths,
            cfg: Config::defaults(),
            ids: Vec::new(),
        };
        let req: create::Request = serde_json::from_value(json!({
            "idempotencyKey": "c", "id": PROJECT, "name": "Page", "keyPrefix": "PAGE",
            "outcome": "Evidence pages", "repositories": ["falconiere/comemory"]
        }))
        .unwrap();
        home.run(&Envelope::local_operator(), req).unwrap();
        home.conn
            .execute_batch(include_str!(
                "../../../../tests/fixtures/projects/plan_seed.sql"
            ))
            .unwrap();
        let commit = json!({"kind": "commit", "repo": "falconiere/comemory", "commitSha": SHA});
        let rows = [
            commit.clone(),
            json!({"kind": "test_run", "workItemId": ITEM}),
            merged(&commit, json!({"workItemId": ITEM})),
            json!({"kind": "external_url"}),
            json!({"kind": "memory", "externalId": "ab12cd34", "workItemId": ITEM}),
            json!({"kind": "deployment", "workItemId": ITEM}),
        ];
        for (n, extra) in rows.into_iter().enumerate() {
            let body = merged(
                &json!({"projectId": PROJECT, "idempotencyKey": format!("k{n}"), "source": "ci"}),
                extra,
            );
            let req: evidence_add::Request = serde_json::from_value(body).unwrap();
            let id = home.run(&Envelope::local_agent(), req).unwrap().evidence.id;
            // Distinct, increasing stamps, so newest-first order is known.
            home.conn
                .execute(
                    "UPDATE project_evidence SET created_at = ?1 WHERE id = ?2",
                    rusqlite::params![1_000 + n as i64, id],
                )
                .unwrap();
            home.ids.push(id);
        }
        home.conn
            .execute(
                "UPDATE project_evidence SET trust = 'bogus' WHERE id = ?1",
                [&home.ids[4]],
            )
            .unwrap();
        home
    }

    fn run<C: authority::Command>(
        &mut self,
        envelope: &Envelope,
        command: C,
    ) -> Result<C::Response> {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        authority::run(&mut ctx, envelope, command)
    }

    fn page(&mut self, filter: Value) -> Result<evidence_page::Response> {
        let req: evidence_page::Request =
            serde_json::from_value(merged(&json!({"projectId": PROJECT}), filter)).unwrap();
        self.run(&Envelope::local_agent(), req)
    }

    /// The attached indexes a filter returns, in page order.
    fn indexes(&mut self, filter: Value) -> Vec<usize> {
        let page = self.page(filter).unwrap();
        page.evidence
            .iter()
            .map(|e| self.ids.iter().position(|id| *id == e.id).unwrap())
            .collect()
    }
}

fn merged(base: &Value, extra: Value) -> Value {
    let mut out = base.clone();
    out.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    out
}

#[test]
fn each_filter_and_every_combination_returns_the_matching_rows() {
    let mut home = Home::new();
    let cases = [
        (json!({}), vec![5, 4, 3, 2, 1, 0]),
        (json!({"kind": "commit"}), vec![2, 0]),
        (json!({"trust": "self_reported"}), vec![5, 3, 1]),
        (json!({"trust": "pending"}), vec![2, 0]),
        (json!({"trust": "invalid"}), vec![4]),
        (json!({"trust": "verified"}), vec![]),
        (json!({"workItemId": ITEM.to_uppercase()}), vec![5, 4, 2, 1]),
        (json!({"kind": "commit", "trust": "pending"}), vec![2, 0]),
        (json!({"kind": "commit", "workItemId": ITEM}), vec![2]),
        (
            json!({"trust": "self_reported", "workItemId": ITEM}),
            vec![5, 1],
        ),
        (
            json!({"kind": "memory", "trust": "invalid", "workItemId": ITEM}),
            vec![4],
        ),
        (
            json!({"kind": "commit", "trust": "self_reported", "workItemId": ITEM}),
            vec![],
        ),
        (
            json!({"workItemId": "b0000000-0000-4000-8000-000000000099"}),
            vec![],
        ),
    ];
    for (filter, expected) in cases {
        assert_eq!(home.indexes(filter.clone()), expected, "{filter}");
    }
    let shown = home.page(json!({"trust": "invalid"})).unwrap();
    assert_eq!(shown.evidence[0].trust, "invalid");
}

#[test]
fn a_limit_walk_visits_every_row_once_and_ends_on_a_null_cursor() {
    let mut home = Home::new();
    let mut walked = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let page = home
            .page(json!({"limit": 4, "cursor": cursor, "workItemId": ITEM}))
            .unwrap();
        walked.extend(page.evidence.into_iter().map(|e| e.id));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    let expected: Vec<String> = [5, 4, 2, 1].iter().map(|n| home.ids[*n].clone()).collect();
    assert_eq!(walked, expected);
    assert!(cursor.is_none());
    assert_eq!(home.page(json!({"limit": 100})).unwrap().evidence.len(), 6);
}

#[test]
fn malformed_unknown_and_out_of_range_reads_are_refused() {
    let mut home = Home::new();
    let cases = [
        (
            json!({"limit": 0}),
            ("invalid_request", Class::Unprocessable),
        ),
        (
            json!({"limit": 101}),
            ("invalid_request", Class::Unprocessable),
        ),
        (
            json!({"cursor": "nope"}),
            ("invalid_request", Class::BadRequest),
        ),
        (
            json!({"kind": "screenshot"}),
            ("invalid_request", Class::BadRequest),
        ),
        (
            json!({"trust": "bogus"}),
            ("invalid_request", Class::BadRequest),
        ),
        (
            json!({"workItemId": "nope"}),
            ("invalid_request", Class::BadRequest),
        ),
        (
            json!({"projectId": "nope"}),
            ("invalid_request", Class::BadRequest),
        ),
        (
            json!({"projectId": "00000000-0000-4000-8000-000000000000"}),
            ("project_not_found", Class::NotFound),
        ),
    ];
    for (filter, expected) in cases {
        let e = home.page(filter.clone()).unwrap_err();
        assert_eq!(classify(&e), expected, "{filter}");
    }
}

#[test]
fn an_agent_without_project_read_is_refused_before_the_store_opens() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let writer = Envelope::agent("writer", Capabilities::parse("t", &["evidence.create"]));
    let req = evidence_page::Request {
        project_id: PROJECT.into(),
        ..Default::default()
    };
    let e = authority::run(&mut ctx, &writer, req).unwrap_err();
    assert_eq!(classify(&e), ("project_agent_scope", Class::Forbidden));
    assert!(!paths.db_path().exists(), "the refusal opened the store");
}
