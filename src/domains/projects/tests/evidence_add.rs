#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/evidence_add.rs` over a real store:
//! a project-level and an item-level attach each write one row, its criterion
//! links, one `project.evidence.recorded` event and one change frame, with the
//! matrix's initial trust and the platform's stored metadata; every refusal
//! that reads the store (unknown project, foreign repository, criteria of
//! another project, unknown item, metadata over 16 KiB) writes nothing; a
//! replayed key answers the first response and writes nothing; and an agent
//! without `evidence.create` is refused before the store opens.

use comemory::config::{Config, Paths};
use comemory::domains::projects::authority::{self, Capabilities, Envelope};
use comemory::domains::projects::evidence::StoredMetadata;
use comemory::domains::projects::{create, evidence_add};
use comemory::errors::{Error, Result};
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use comemory::utilities::error_code::{Class, classify};
use serde_json::{Map, Value, json};

/// The project `plan_seed.sql` writes its plan into.
const PROJECT: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";
const ITEM: &str = "b0000000-0000-4000-8000-000000000002";
const CRITERION: &str = "c0000000-0000-4000-8000-000000000001";
const ITEM_CRITERION: &str = "c0000000-0000-4000-8000-000000000002";
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// A data directory holding [`PROJECT`] (repository `falconiere/comemory`,
/// the committed plan seed) and an empty [`OTHER`] with one criterion.
struct Home {
    _dir: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    conn: rusqlite::Connection,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure_dirs().unwrap();
        let mut home = Self {
            conn: connection::open(paths.db_path()).unwrap(),
            _dir: dir,
            paths,
            cfg: Config::defaults(),
        };
        for (id, key, extra) in [
            (
                PROJECT,
                "PLAN",
                json!({"repositories": ["falconiere/comemory"]}),
            ),
            (OTHER, "OTHER", json!({"successCriteria": ["Elsewhere"]})),
        ] {
            let mut body = json!({"idempotencyKey": key, "id": id, "name": key,
                                  "keyPrefix": key, "outcome": "Evidence lands"});
            body.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let req: create::Request = serde_json::from_value(body).unwrap();
            home.run(&Envelope::local_operator(), req).unwrap();
        }
        home.conn
            .execute_batch(include_str!(
                "../../../../tests/fixtures/projects/plan_seed.sql"
            ))
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

    /// Attach `extra` merged over an `external_url` claim on [`PROJECT`], as
    /// the local agent, and return the evidence view as JSON.
    fn add(&mut self, extra: Value) -> Result<Value> {
        let mut body = json!({"projectId": PROJECT, "idempotencyKey": "k1",
                              "kind": "external_url", "source": "ci"});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let req: evidence_add::Request = serde_json::from_value(body).unwrap();
        let resp = self.run(&Envelope::local_agent(), req)?;
        Ok(serde_json::to_value(resp.evidence).unwrap())
    }

    /// `[evidence rows, recorded events, evidence receipts, criterion links,
    /// evidence change frames]`.
    fn counts(&self) -> [i64; 5] {
        let sql = "SELECT (SELECT count(*) FROM project_evidence),
            (SELECT count(*) FROM project_activity_events WHERE event_type = 'project.evidence.recorded'),
            (SELECT count(*) FROM project_command_receipts WHERE command_type = 'project.evidence.create'),
            (SELECT count(*) FROM project_evidence_criteria),
            (SELECT count(*) FROM project_changes WHERE entity_type = 'evidence')";
        self.conn
            .query_row(sql, [], |r| {
                Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?])
            })
            .unwrap()
    }

    fn payload(&self, entity_id: &str) -> Value {
        let raw: String = self
            .conn
            .query_row(
                "SELECT payload FROM project_activity_events WHERE entity_id = ?1",
                [entity_id],
                |r| r.get(0),
            )
            .unwrap();
        serde_json::from_str(&raw).unwrap()
    }
}

fn code(e: &Error) -> (&'static str, Class) {
    classify(e)
}

#[test]
fn a_project_level_attach_writes_one_row_event_and_frame() {
    let mut home = Home::new();
    let shown = home
        .add(json!({"url": "https://ci.example/run/7"}))
        .unwrap();
    let id = shown["id"].as_str().unwrap().to_string();
    assert_eq!(shown["workItemId"], Value::Null);
    assert_eq!(shown["executionId"], Value::Null);
    assert_eq!(shown["trust"], "self_reported");
    assert_eq!(shown["url"], "https://ci.example/run/7");
    assert_eq!(shown["creatorPrincipalType"], "project_agent");
    assert_eq!(shown["creatorPrincipalId"], "local-agent");
    assert_eq!(
        shown["metadata"],
        json!({"repo": null, "commitSha": null, "reason": null, "claim": {}, "provider": null})
    );
    assert_eq!(home.counts(), [1, 1, 1, 0, 1]);
    assert_eq!(
        home.payload(&id),
        json!({"kind": "external_url", "trust": "self_reported", "reason": null,
               "workItemId": null, "executionId": null, "criterionIds": []})
    );
}

#[test]
fn an_item_level_commit_is_pending_with_its_claim_and_links() {
    let mut home = Home::new();
    let shown = home
        .add(json!({
            "kind": "commit", "source": "git", "workItemId": ITEM.to_uppercase(),
            "repo": "Falconiere/Comemory", "commitSha": SHA,
            "metadata": {"branch": "main"},
            "criterionIds": [CRITERION, ITEM_CRITERION, CRITERION]
        }))
        .unwrap();
    let id = shown["id"].as_str().unwrap().to_string();
    assert_eq!(shown["workItemId"], ITEM);
    assert_eq!(shown["trust"], "pending");
    assert_eq!(
        shown["metadata"],
        json!({"repo": "falconiere/comemory", "commitSha": SHA,
               "reason": "verification_pending", "claim": {"branch": "main"},
               "provider": null})
    );
    assert_eq!(home.counts(), [1, 1, 1, 2, 1]);
    assert_eq!(
        home.payload(&id),
        json!({"kind": "commit", "trust": "pending", "reason": "verification_pending",
               "workItemId": ITEM, "executionId": null,
               "criterionIds": [CRITERION, ITEM_CRITERION, CRITERION]})
    );
    let memory = home
        .add(json!({"idempotencyKey": "k2", "kind": "memory", "externalId": "ab12cd34"}))
        .unwrap();
    assert_eq!(memory["trust"], "pending");
}

#[test]
fn every_store_refusal_writes_nothing() {
    let mut home = Home::new();
    let cases = [
        (
            json!({"projectId": "00000000-0000-4000-8000-000000000000"}),
            ("project_not_found", Class::NotFound),
        ),
        (
            json!({"kind": "commit", "repo": "someone/else", "commitSha": SHA}),
            ("repo_not_allowed", Class::Forbidden),
        ),
        (
            json!({"criterionIds": [CRITERION, "c0000000-0000-4000-8000-000000000099"]}),
            ("invalid_request", Class::Unprocessable),
        ),
        (
            json!({"workItemId": "b0000000-0000-4000-8000-000000000099"}),
            ("work_item_not_found", Class::NotFound),
        ),
    ];
    for (extra, expected) in cases {
        let e = home.add(extra.clone()).unwrap_err();
        assert_eq!(code(&e), expected, "{extra}");
        assert_eq!(home.counts(), [0, 0, 0, 0, 0], "{extra}");
    }
    let foreign: String = home
        .conn
        .query_row(
            "SELECT id FROM project_criteria WHERE project_id = ?1",
            [OTHER],
            |r| r.get(0),
        )
        .unwrap();
    let Error::Project(refusal) = home.add(json!({"criterionIds": [&foreign]})).unwrap_err() else {
        panic!("expected a project refusal");
    };
    assert_eq!(
        serde_json::to_value(refusal.details()).unwrap(),
        json!({"field": "criterionIds", "reason": "unknown", "criterionIds": [foreign]})
    );
    assert_eq!(
        refusal.to_string(),
        "One or more criteria do not belong to this project"
    );
}

#[test]
fn metadata_is_capped_at_16_kib_encoded() {
    let mut home = Home::new();
    let empty = Map::new();
    let bare = StoredMetadata {
        repo: None,
        commit_sha: None,
        reason: None,
        claim: &empty,
        provider: None,
    };
    let overhead = bare.encode().unwrap().len() + r#""k":"""#.len();
    let fill = 16 * 1024 - overhead;
    let e = home
        .add(json!({"metadata": {"k": "x".repeat(fill + 1)}}))
        .unwrap_err();
    assert_eq!(code(&e), ("invalid_request", Class::Unprocessable));
    assert_eq!(e.to_string(), "metadata is too_large (limit 16384)");
    assert_eq!(home.counts(), [0, 0, 0, 0, 0]);
    // The cap is judged before the store: an unknown project still hears it.
    let unknown = json!({"projectId": "00000000-0000-4000-8000-000000000000",
                         "metadata": {"k": "x".repeat(fill + 1)}});
    let e = home.add(unknown).unwrap_err();
    assert_eq!(code(&e), ("invalid_request", Class::Unprocessable));
    assert_eq!(e.to_string(), "metadata is too_large (limit 16384)");
    assert_eq!(home.counts(), [0, 0, 0, 0, 0]);
    home.add(json!({"metadata": {"k": "x".repeat(fill)}}))
        .unwrap();
    assert_eq!(home.counts(), [1, 1, 1, 0, 1]);
}

#[test]
fn a_replayed_key_answers_the_first_attach_and_another_body_conflicts() {
    let mut home = Home::new();
    let first = home.add(json!({})).unwrap();
    assert_eq!(home.add(json!({})).unwrap(), first);
    assert_eq!(home.counts(), [1, 1, 1, 0, 1]);
    let changed = home.add(json!({"source": "other"})).unwrap_err();
    assert_eq!(code(&changed), ("idempotency_conflict", Class::Conflict));
    let elsewhere = home.add(json!({"projectId": OTHER})).unwrap_err();
    assert_eq!(code(&elsewhere), ("idempotency_conflict", Class::Conflict));
    assert_eq!(home.counts(), [1, 1, 1, 0, 1]);
}

#[test]
fn an_agent_without_evidence_create_is_refused_before_the_store_opens() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let reader = Envelope::agent("reader", Capabilities::parse("t", &["project.read"]));
    let req: evidence_add::Request = serde_json::from_value(json!({
        "projectId": PROJECT, "idempotencyKey": "k", "kind": "decision",
        "source": "adr", "externalId": "adr-7"
    }))
    .unwrap();
    let e = authority::run(&mut ctx, &reader, req).unwrap_err();
    assert_eq!(code(&e), ("project_agent_scope", Class::Forbidden));
    assert_eq!(
        e.to_string(),
        "This grant does not carry the evidence.create capability"
    );
    assert!(!paths.db_path().exists(), "the refusal opened the store");
}
