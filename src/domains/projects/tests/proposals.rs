#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/proposals.rs`, against a real temp
//! data directory: the list filters by state and chains pages through
//! `nextCursor` at `limit 1`; a reviewed row carries its review; a corrupt
//! stored operations column reads as empty; and show answers `404
//! proposal_not_found` for an unknown proposal or one of another project.

use comemory::config::{Config, Paths};
use comemory::domains::projects::authority::{self, Envelope};
use comemory::domains::projects::proposals::{ListRequest, ShowRequest};
use comemory::domains::projects::{create, propose};
use comemory::errors::Result;
use comemory::store::{Connection, connection};
use comemory::utilities::context::Ctx;
use comemory::utilities::error_code::{Class, classify};
use comemory::utilities::project_body::body;
use serde_json::json;

const PROJECT: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";

struct Home {
    _dir: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    conn: Connection,
}

impl Home {
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
        for (id, key) in [(PROJECT, "ONE"), (OTHER, "TWO")] {
            let charter: create::Request = serde_json::from_value(json!({
                "id": id, "idempotencyKey": key, "name": key, "keyPrefix": key, "outcome": "o"
            }))
            .unwrap();
            home.run_as(&Envelope::local_operator(), charter).unwrap();
        }
        home
    }

    fn run_as<C: authority::Command>(
        &mut self,
        envelope: &Envelope,
        req: C,
    ) -> Result<C::Response> {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        authority::run(&mut ctx, envelope, req)
    }

    /// Run `req` as the local agent, a proposal reader and writer.
    fn run<C: authority::Command>(&mut self, req: C) -> Result<C::Response> {
        self.run_as(&Envelope::local_agent(), req)
    }

    /// Submit one archive operation to `project` and return the new id. Its
    /// `created_at` is then pinned to the count of proposals before it, so
    /// submissions inside one millisecond still order as written (a real
    /// tie falls back to `id DESC`, which a random UUID makes arbitrary).
    fn submit(&mut self, project: &str, key: &str) -> String {
        let raw = json!({
            "projectId": project, "idempotencyKey": key, "basePlanVersion": 0,
            "operations": [{"op": "criterion.archive",
                "criterionId": "c0000000-0000-4000-8000-000000000001"}],
            "rationale": key
        });
        let req: propose::Request = body(raw.to_string().as_bytes()).unwrap();
        let id = self.run(req).unwrap().proposal.id;
        self.conn
            .execute(
                "UPDATE project_plan_proposals SET created_at = \
                 (SELECT COUNT(*) FROM project_plan_proposals) WHERE id = ?1",
                [&id],
            )
            .unwrap();
        id
    }

    fn list(
        &mut self,
        state: Option<&str>,
        cursor: Option<String>,
        limit: i64,
    ) -> Result<(Vec<String>, Option<String>)> {
        let page = self.run(ListRequest {
            project_id: PROJECT.to_string(),
            limit: Some(limit),
            cursor,
            state: state.map(str::to_string),
        })?;
        let ids = page.proposals.into_iter().map(|p| p.id).collect();
        Ok((ids, page.next_cursor))
    }
}

#[test]
fn the_list_filters_by_state_and_chains_pages_newest_first() {
    let mut home = Home::new();
    let first = home.submit(PROJECT, "a");
    let second = home.submit(PROJECT, "b");
    let third = home.submit(PROJECT, "c");
    home.submit(OTHER, "d");
    home.conn
        .execute(
            "UPDATE project_plan_proposals SET state = 'approved' WHERE id = ?1",
            [&second],
        )
        .unwrap();

    let mut walked = Vec::new();
    let mut cursor = None;
    loop {
        let (ids, next) = home.list(None, cursor, 1).unwrap();
        walked.extend(ids);
        match next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(walked, [third.clone(), second.clone(), first.clone()]);
    assert_eq!(
        home.list(Some("pending"), None, 100).unwrap().0,
        [third, first]
    );
    assert_eq!(home.list(Some("approved"), None, 100).unwrap().0, [second]);
    assert!(home.list(Some("rejected"), None, 100).unwrap().0.is_empty());
}

#[test]
fn a_reviewed_proposal_carries_its_review_and_a_corrupt_column_reads_empty() {
    let mut home = Home::new();
    let id = home.submit(PROJECT, "a");
    home.conn
        .execute(
            "INSERT INTO project_approvals (id, project_id, proposal_id, decision, \
             reviewer_principal_type, reviewer_principal_id, rationale, created_at) \
             VALUES ('f0000000-0000-4000-8000-000000000001', ?1, ?2, 'approve', 'user', \
             'local-operator', NULL, 0)",
            [PROJECT, id.as_str()],
        )
        .unwrap();
    home.conn
        .execute(
            "UPDATE project_plan_proposals SET operations = 'not json'",
            [],
        )
        .unwrap();
    let shown = home
        .run(ShowRequest {
            project_id: PROJECT.to_string(),
            proposal_id: id,
        })
        .unwrap()
        .proposal;
    assert!(shown.operations.is_empty());
    assert_eq!(
        serde_json::to_value(shown.review).unwrap(),
        json!({"decision": "approve", "reviewerId": "local-operator", "rationale": null,
               "createdAt": "1970-01-01T00:00:00.000Z"})
    );
}

#[test]
fn show_and_list_refuse_unknown_foreign_and_malformed_ids() {
    let mut home = Home::new();
    let foreign = home.submit(OTHER, "a");
    let show = |project: &str, proposal: &str| ShowRequest {
        project_id: project.to_string(),
        proposal_id: proposal.to_string(),
    };
    for (req, code, class) in [
        (
            show(PROJECT, &foreign),
            "proposal_not_found",
            Class::NotFound,
        ),
        (
            show(PROJECT, "33333333-3333-4333-8333-333333333333"),
            "proposal_not_found",
            Class::NotFound,
        ),
        (show(PROJECT, "nope"), "invalid_request", Class::BadRequest),
        (
            show("99999999-9999-4999-8999-999999999999", &foreign),
            "project_not_found",
            Class::NotFound,
        ),
    ] {
        let e = home.run(req).expect_err("refused");
        assert_eq!(classify(&e), (code, class), "{e}");
    }
    for (state, cursor, limit, class) in [
        (Some("merged"), None, 20, Class::BadRequest),
        (None, Some("abc".to_string()), 20, Class::BadRequest),
        (None, None, 101, Class::Unprocessable),
    ] {
        let e = home.list(state, cursor, limit).expect_err("refused");
        assert_eq!(classify(&e), ("invalid_request", class), "{e}");
    }
    let e = home
        .run(ListRequest {
            project_id: "99999999-9999-4999-8999-999999999999".to_string(),
            ..ListRequest::default()
        })
        .expect_err("unknown project");
    assert_eq!(classify(&e).0, "project_not_found");
}
