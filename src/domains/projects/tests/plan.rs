#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/plan.rs` over a real store: a fresh
//! project with no success criteria reads plan version 0 with four empty
//! arrays; the plan `tests/fixtures/projects/plan_seed.sql` seeds renders both
//! criteria levels and every live edge while every archived entity, and each
//! edge naming an archived item, is absent, and a live item or criterion
//! keeps its reference to an archived milestone or item, as on the platform; an agent without `project.read` is refused
//! before the store opens; a malformed id is a `400`, an unknown one a `404`,
//! and a milestone date outside the representable range an `internal_error`.

use comemory::config::{Config, Paths};
use comemory::domains::projects::authority::{self, Capabilities, Envelope};
use comemory::domains::projects::{create, plan};
use comemory::errors::Result;
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use comemory::utilities::error_code::{Class, classify};
use serde_json::json;

/// The project `plan_seed.sql` writes its plan into.
const PROJECT: &str = "11111111-1111-4111-8111-111111111111";

/// A data directory holding [`PROJECT`], chartered by the local operator.
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
        let req: create::Request = serde_json::from_value(json!({
            "idempotencyKey": "k1", "id": PROJECT, "name": "Plan", "keyPrefix": "PLAN",
            "outcome": "A plan reads back"
        }))
        .unwrap();
        home.read(&Envelope::local_operator(), req).unwrap();
        home
    }

    /// Run `command` under `envelope` against this home's store.
    fn read<C: authority::Command>(
        &mut self,
        envelope: &Envelope,
        command: C,
    ) -> Result<C::Response> {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        authority::run(&mut ctx, envelope, command)
    }

    /// The plan of `id`, read by the local agent.
    fn plan(&mut self, id: &str) -> Result<plan::PlanView> {
        let req = plan::Request { id: id.into() };
        Ok(self.read(&Envelope::local_agent(), req)?.plan)
    }
}

#[test]
fn a_fresh_project_reads_plan_version_zero_with_empty_collections() {
    let mut home = Home::new();
    let plan = serde_json::to_value(home.plan(&PROJECT.to_uppercase()).unwrap()).unwrap();
    assert_eq!(
        plan,
        json!({
            "projectId": PROJECT, "planVersion": 0,
            "milestones": [], "workItems": [], "criteria": [], "dependencies": []
        })
    );
}

#[test]
fn a_seeded_plan_renders_live_rows_and_drops_archived_ones() {
    let mut home = Home::new();
    home.conn
        .execute_batch(include_str!(
            "../../../../tests/fixtures/projects/plan_seed.sql"
        ))
        .unwrap();
    let plan = serde_json::to_value(home.plan(PROJECT).unwrap()).unwrap();
    let id = |prefix: char, n: u8| format!("{prefix}0000000-0000-4000-8000-0000000000{n:02}");
    assert_eq!(plan["planVersion"], 3);
    assert_eq!(
        plan["milestones"],
        json!([
            {"id": id('a', 1), "name": "Alpha", "description": "The reader ships",
             "targetDate": "2026-10-01T00:00:00.000Z", "position": 0, "status": "planned"},
            {"id": id('a', 2), "name": "Beta", "description": "Proposals ship",
             "targetDate": "2026-11-15T00:00:00.000Z", "position": 0, "status": "in_progress"}
        ])
    );
    assert_eq!(
        plan["workItems"],
        json!([
            {"id": id('b', 2), "number": 2, "parentWorkItemId": id('b', 1),
             "milestoneId": id('a', 1), "kind": "task", "title": "Render the items",
             "description": "Every live item", "status": "ready", "priority": "high",
             "estimate": 3, "assigneePrincipalType": "user",
             "assigneePrincipalId": "local-operator", "repo": "falconiere/comemory",
             "version": 2, "position": 0},
            {"id": id('b', 4), "number": 4, "parentWorkItemId": null, "milestoneId": null,
             "kind": "bug", "title": "Fix the edge", "description": "No milestone",
             "status": "backlog", "priority": "normal", "estimate": null,
             "assigneePrincipalType": null, "assigneePrincipalId": null, "repo": null,
             "version": 1, "position": 0},
            {"id": id('b', 1), "number": 1, "parentWorkItemId": null,
             "milestoneId": id('a', 1), "kind": "task", "title": "Build the reader",
             "description": "Read the plan", "status": "backlog", "priority": "normal",
             "estimate": null, "assigneePrincipalType": null, "assigneePrincipalId": null,
             "repo": null, "version": 1, "position": 1},
            {"id": id('b', 5), "number": 5, "parentWorkItemId": null,
             "milestoneId": id('a', 3), "kind": "task", "title": "Outlive the milestone",
             "description": "Live in an archived milestone", "status": "backlog",
             "priority": "normal", "estimate": null, "assigneePrincipalType": null,
             "assigneePrincipalId": null, "repo": null, "version": 1, "position": 2}
        ])
    );
    assert_eq!(
        plan["criteria"],
        json!([
            {"id": id('c', 1), "description": "Plans read offline", "required": true,
             "evidenceRequirement": "reported", "resolution": "open",
             "resolutionRationale": null, "position": 0, "workItemId": null},
            {"id": id('c', 2), "description": "Items render", "required": false,
             "evidenceRequirement": "verified", "resolution": "waived",
             "resolutionRationale": "Covered elsewhere", "position": 1,
             "workItemId": id('b', 2)},
            {"id": id('c', 4), "description": "Outlive the item", "required": true,
             "evidenceRequirement": "reported", "resolution": "open",
             "resolutionRationale": null, "position": 2, "workItemId": id('b', 3)}
        ])
    );
    assert_eq!(
        plan["dependencies"],
        json!([
            {"blockerId": id('b', 1), "blockedId": id('b', 2)},
            {"blockerId": id('b', 4), "blockedId": id('b', 2)}
        ])
    );
}

#[test]
fn an_agent_without_project_read_is_refused_before_the_store_opens() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let writer = Envelope::agent(
        "writer",
        Capabilities::parse("t", &["proposal.create", "execution.update"]),
    );
    let e = authority::run(&mut ctx, &writer, plan::Request { id: PROJECT.into() })
        .expect_err("an agent without project.read must be refused");
    assert_eq!(classify(&e), ("project_agent_scope", Class::Forbidden));
    assert_eq!(
        e.to_string(),
        "This grant does not carry the project.read capability"
    );
    assert!(!paths.db_path().exists(), "the refusal opened the store");

    let mut home = Home::new();
    let reader = Envelope::agent("reader", Capabilities::parse("t", &["project.read"]));
    let admitted = home.read(&reader, plan::Request { id: PROJECT.into() });
    assert_eq!(admitted.unwrap().plan.plan_version, 0);
}

#[test]
fn malformed_unknown_and_corrupt_reads_are_refused() {
    let mut home = Home::new();
    let e = home.plan("not-a-uuid").unwrap_err();
    assert_eq!(classify(&e), ("invalid_request", Class::BadRequest));
    let e = home
        .plan("00000000-0000-4000-8000-000000000000")
        .unwrap_err();
    assert_eq!(classify(&e), ("project_not_found", Class::NotFound));

    home.conn
        .execute_batch(include_str!(
            "../../../../tests/fixtures/projects/plan_seed.sql"
        ))
        .unwrap();
    home.conn
        .execute_batch("UPDATE project_milestones SET target_date = 9223372036854775807")
        .unwrap();
    let e = home.plan(PROJECT).unwrap_err();
    assert_eq!(classify(&e), ("internal_error", Class::Internal));
    assert_eq!(
        e.to_string(),
        "project_milestones.target_date of a0000000-0000-4000-8000-000000000001 \
         is outside the representable range"
    );
}
