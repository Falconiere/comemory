#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! Test mirror for `src/domains/projects/propose.rs`, against a real temp
//! data directory: the twelve-kind proposal is stored normalized and shown
//! back equal; the first submission moves `draft` to `planning` once, with
//! one event and one feed row, and a replay or a second proposal moves
//! nothing; and every refusal — a patch identity, an agent without
//! `proposal.create`, a stale base, an archived or closed project, an
//! unknown project — leaves no row behind.

use comemory::config::{Config, Paths};
use comemory::domains::projects::authority::{self, Capabilities, Envelope};
use comemory::domains::projects::proposals::{ShowRequest, ShowResponse};
use comemory::domains::projects::{create, propose};
use comemory::errors::{Error, Result};
use comemory::store::{Connection, connection};
use comemory::utilities::context::Ctx;
use comemory::utilities::error_code::{Class, classify};
use comemory::utilities::project_body::body;
use serde_json::{Value, json};

const PROJECT: &str = "11111111-1111-4111-8111-111111111111";

struct Home {
    _dir: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    conn: Connection,
}

impl Home {
    /// A data directory holding the draft [`PROJECT`].
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure_dirs().unwrap();
        let conn = connection::open(paths.db_path()).unwrap();
        let mut home = Self {
            _dir: dir,
            paths,
            cfg: Config::defaults(),
            conn,
        };
        let charter: create::Request = serde_json::from_value(json!({
            "id": PROJECT, "idempotencyKey": "charter", "name": "Plan",
            "keyPrefix": "PLAN", "outcome": "Proposals work"
        }))
        .unwrap();
        home.run(&Envelope::local_operator(), charter).unwrap();
        home
    }

    fn run<C: authority::Command>(&mut self, envelope: &Envelope, req: C) -> Result<C::Response> {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        authority::run(&mut ctx, envelope, req)
    }

    /// Submit `body` (a JSON request) as the local agent.
    fn submit(&mut self, body_json: &Value) -> Result<propose::Response> {
        let req: propose::Request = body(body_json.to_string().as_bytes())?;
        self.run(&Envelope::local_agent(), req)
    }

    fn scalar(&self, sql: &str) -> Value {
        self.conn
            .query_row(sql, [], |r| {
                Ok(match r.get_ref(0)? {
                    rusqlite::types::ValueRef::Integer(n) => json!(n),
                    rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                    _ => Value::Null,
                })
            })
            .unwrap()
    }

    /// `(proposals, events, feed rows, receipts, status, version)`.
    fn state(&self) -> (Value, Value, Value, Value, Value, Value) {
        (
            self.scalar("SELECT COUNT(*) FROM project_plan_proposals"),
            self.scalar("SELECT COUNT(*) FROM project_activity_events"),
            self.scalar("SELECT COUNT(*) FROM project_changes"),
            self.scalar("SELECT COUNT(*) FROM project_command_receipts"),
            self.scalar("SELECT status FROM projects"),
            self.scalar("SELECT version FROM projects"),
        )
    }
}

fn fixture() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/projects/proposal_operations.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A submission of `operations` under `key` against plan version 0.
fn request(key: &str, operations: Value) -> Value {
    json!({
        "projectId": PROJECT, "idempotencyKey": key, "basePlanVersion": 0,
        "operations": operations, "rationale": "Scope the reader",
        "assumptions": ["The reader is merged"], "risks": ["Churn"]
    })
}

fn archive_op() -> Value {
    json!([{"op": "criterion.archive", "criterionId": "c0000000-0000-4000-8000-000000000001"}])
}

/// `(code, class, details)` of a project refusal.
fn refusal<T: std::fmt::Debug>(result: Result<T>) -> (&'static str, Class, String) {
    let e = result.expect_err("expected a refusal");
    let Error::Project(project) = &e else {
        panic!("not a project refusal: {e:?}")
    };
    let (code, class) = classify(&e);
    (
        code,
        class,
        serde_json::to_string(&project.details()).unwrap(),
    )
}

#[test]
fn every_kind_is_stored_normalized_and_shown_back() {
    let mut home = Home::new();
    let fixture = fixture();
    let submitted = home
        .submit(&request("all", fixture["operations"].clone()))
        .unwrap()
        .proposal;
    assert_eq!(
        serde_json::to_value(&submitted.operations).unwrap(),
        fixture["normalized"]
    );
    assert_eq!(
        (
            submitted.state.as_str(),
            submitted.base_plan_version,
            submitted.review.is_none()
        ),
        ("pending", 0, true)
    );
    assert_eq!(
        (
            submitted.proposer_principal_type.as_str(),
            submitted.proposer_principal_id.as_str()
        ),
        ("project_agent", "local-agent")
    );
    let show = ShowRequest {
        project_id: PROJECT.to_string(),
        proposal_id: submitted.id.to_uppercase(),
    };
    let ShowResponse { proposal } = home.run(&Envelope::local_operator(), show).unwrap();
    assert_eq!(proposal, submitted);
    let digest = home.scalar("SELECT request_digest FROM project_plan_proposals");
    let receipt = home.scalar("SELECT request_digest FROM project_command_receipts WHERE command_type = 'project.proposal.submit'");
    assert_eq!(digest, receipt);
}

#[test]
fn the_first_submission_moves_draft_to_planning_once() {
    let mut home = Home::new();
    let (_, events, feed, receipts, _, version) = home.state();
    let first = home.submit(&request("one", archive_op())).unwrap();
    assert_eq!(
        home.state(),
        (
            json!(1),
            json!(events.as_i64().unwrap() + 1),
            json!(feed.as_i64().unwrap() + 1),
            json!(receipts.as_i64().unwrap() + 1),
            json!("planning"),
            json!(version.as_i64().unwrap() + 1)
        )
    );
    let event = home.scalar(
        "SELECT event_type || ' ' || entity_type || ' ' || payload FROM project_activity_events \
         WHERE event_type = 'project.proposal_submitted'",
    );
    assert_eq!(
        event,
        json!(
            r#"project.proposal_submitted proposal {"basePlanVersion":0,"operationCount":1,"riskCount":1}"#
        )
    );
    let after_first = home.state();
    let replayed = home.submit(&request("one", archive_op())).unwrap();
    assert_eq!(replayed, first);
    assert_eq!(home.state(), after_first);
    home.submit(&request("two", archive_op())).unwrap();
    let (proposals, .., status, version_now) = home.state();
    assert_eq!(
        (proposals, status, version_now),
        (json!(2), after_first.4, after_first.5)
    );
    let (code, class, _) = refusal(home.submit(&request("one", json!([]))));
    assert_eq!((code, class), ("invalid_request", Class::Unprocessable));
    let conflict = request(
        "two",
        json!([{"op": "work_item.archive",
        "workItemId": "b0000000-0000-4000-8000-000000000001"}]),
    );
    assert_eq!(refusal(home.submit(&conflict)).0, "idempotency_conflict");
}

#[test]
fn a_patch_carrying_an_identity_is_refused_before_anything_is_written() {
    let mut home = Home::new();
    let before = home.state();
    let id = "b0000000-0000-4000-8000-000000000001";
    for patch in [
        json!({"op": "work_item.update", "workItemId": id, "patch": {"id": id}}),
        json!({"op": "criterion.update", "criterionId": id, "patch": {"workItemId": id}}),
        json!({"op": "milestone.update", "milestoneId": id, "patch": {"id": id}}),
    ] {
        let (code, class, details) = refusal(home.submit(&request("k", json!([patch]))));
        assert_eq!(
            (code, class, details.as_str()),
            (
                "invalid_request",
                Class::BadRequest,
                r#"{"field":"operations.0","reason":"invalid"}"#
            )
        );
    }
    assert_eq!(home.state(), before);
}

#[test]
fn an_agent_without_proposal_create_is_refused_and_nothing_is_written() {
    let mut home = Home::new();
    let before = home.state();
    let reader = Envelope::agent("reader", Capabilities::parse("t", &["project.read"]));
    let req: propose::Request = body(request("k", archive_op()).to_string().as_bytes()).unwrap();
    let e = home.run(&reader, req).expect_err("refused");
    assert_eq!(classify(&e), ("project_agent_scope", Class::Forbidden));
    assert_eq!(
        e.to_string(),
        "This grant does not carry the proposal.create capability"
    );
    assert_eq!(home.state(), before);
}

#[test]
fn a_stale_base_an_archived_or_closed_project_and_an_unknown_one_are_refused() {
    let mut home = Home::new();
    let before = home.state();
    let mut stale = request("stale", archive_op());
    stale["basePlanVersion"] = json!(3);
    assert_eq!(
        refusal(home.submit(&stale)),
        (
            "proposal_stale",
            Class::Conflict,
            r#"{"code":"proposal_stale","basePlanVersion":3,"currentPlanVersion":0}"#.to_string()
        )
    );
    let mut unknown = request("unknown", archive_op());
    unknown["projectId"] = json!("99999999-9999-4999-8999-999999999999");
    assert_eq!(refusal(home.submit(&unknown)).0, "project_not_found");
    let mut malformed = request("malformed", archive_op());
    malformed["projectId"] = json!("nope");
    assert_eq!(refusal(home.submit(&malformed)).1, Class::BadRequest);
    // No proposal, event, feed row or receipt, and the draft is untouched.
    assert_eq!(home.state(), before);
    home.conn
        .execute("UPDATE projects SET status = 'completed'", [])
        .unwrap();
    let e = home
        .submit(&request("closed", archive_op()))
        .expect_err("closed");
    assert_eq!(
        (classify(&e).0, e.to_string().as_str()),
        (
            "invalid_transition",
            "This project is closed to new proposals"
        )
    );
    let closed = home.state();
    assert_eq!(
        (closed.0, closed.1, closed.2, closed.3, closed.4, closed.5),
        (
            before.0.clone(),
            before.1.clone(),
            before.2.clone(),
            before.3.clone(),
            json!("completed"),
            before.5.clone()
        )
    );
    home.conn
        .execute("UPDATE projects SET status = 'draft', archived_at = 1", [])
        .unwrap();
    let e = home
        .submit(&request("archived", archive_op()))
        .expect_err("archived");
    assert_eq!(
        (classify(&e).0, e.to_string().as_str()),
        ("invalid_transition", "This project is archived")
    );
    let archived = home.state();
    assert_eq!(
        (
            archived.0, archived.1, archived.2, archived.3, archived.4, archived.5
        ),
        (
            before.0,
            before.1,
            before.2,
            before.3,
            json!("draft"),
            before.5
        )
    );
}
