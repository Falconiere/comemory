#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! Test mirror for `src/domains/projects/receipt.rs`, against a real temp data
//! directory: a replay returns the first response and writes nothing in any
//! project table or `activity_log`; a reuse for another body or command type
//! is `idempotency_conflict` before the command runs; two principals keep
//! separate scopes; a failed command leaves no receipt and its retry runs;
//! and the key's length is checked in UTF-16 units.

use std::cell::Cell;

use serde_json::json;

use super::{Applied, IDEMPOTENCY_KEY_MAX, Keyed, digest, run};
use crate::config::{Config, Paths};
use crate::domains::projects::authority::{self, Actor, Command, Envelope, Tier, Verb, sealed};
use crate::domains::projects::create::{Request, Response};
use crate::errors::{Error, Result};
use crate::store::projects::{NewProject, ProjectInsert, insert_project};
use crate::store::schema_projects::PROJECT_TABLES;
use crate::store::{Connection, connection};
use crate::utilities::context::Ctx;
use crate::utilities::error_code::{Class, classify};

/// A migrated data directory and its open connection.
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
        Self {
            _dir: dir,
            paths,
            cfg: Config::defaults(),
            conn,
        }
    }

    fn run<C: Command>(&mut self, envelope: &Envelope, command: C) -> Result<C::Response> {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        authority::run(&mut ctx, envelope, command)
    }

    fn create(&mut self, req: Request) -> Result<Response> {
        self.run(&Envelope::local_operator(), req)
    }

    /// Row counts of every project table, then `activity_log`.
    fn counts(&self) -> Vec<i64> {
        PROJECT_TABLES
            .iter()
            .chain(&["project_changes", "activity_log"])
            .map(|t| {
                self.conn
                    .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
                    .unwrap()
            })
            .collect()
    }

    fn count(&self, table: &str) -> i64 {
        let at = PROJECT_TABLES.iter().position(|t| *t == table).unwrap();
        self.counts()[at]
    }
}

fn request(key: &str, key_prefix: &str) -> Request {
    serde_json::from_value(json!({
        "idempotencyKey": key,
        "name": format!("Project {key_prefix}"),
        "keyPrefix": key_prefix,
        "outcome": "Ship it",
        "successCriteria": ["Tests pass"],
        "repositories": ["Falconiere/comemory"],
    }))
    .unwrap()
}

/// The refusal's code, class and message.
fn refusal<T: std::fmt::Debug>(result: Result<T>) -> (&'static str, Class, String) {
    let e = result.expect_err("expected a refusal");
    let (code, class) = classify(&e);
    (code, class, e.to_string())
}

/// A second command type under the wrapper, as #328's lifecycle verbs will
/// be: it writes one project row, or fails after writing it.
struct Probe<'a> {
    key: &'a str,
    command_type: &'static str,
    fail: bool,
    applied: &'a Cell<u32>,
}

impl sealed::Sealed for Probe<'_> {}

impl Command for Probe<'_> {
    type Response = String;

    fn verb(&self) -> Verb {
        Verb::ProjectCreate
    }

    fn execute(self, ctx: &mut Ctx<'_>, actor: &Actor) -> Result<String> {
        let body = json!({"probe": true});
        let keyed = Keyed::new(self.key, self.command_type, &body)?;
        let ran = run(ctx.conn()?, actor, &keyed, |tx| {
            self.applied.set(self.applied.get() + 1);
            let id = "7c1e0b4a-0d6e-4b8f-9a51-3f2d8e6c4b10";
            let row = NewProject {
                id,
                slug: "probe",
                key_prefix: "PROBE",
                name: "Probe",
                outcome: "o",
                constraints: "[]",
                non_goals: "[]",
                lead_type: "user",
                lead_id: "local-operator",
                target_date: None,
                creator_type: "user",
                creator_id: "local-operator",
                at_ms: 1,
            };
            assert_eq!(insert_project(tx, &row)?, ProjectInsert::Inserted);
            if self.fail {
                return Err(Error::Other("probe failed after its write".into()));
            }
            Ok(Applied {
                response: id.to_string(),
                project_id: id.to_string(),
            })
        })?;
        Ok(match ran {
            super::Ran::Applied(id) | super::Ran::Replayed(id) => id,
        })
    }
}

#[test]
fn the_digest_is_stable_key_order_free_and_command_type_sensitive() {
    let a = digest(
        "project.pause",
        &json!({"expectedVersion": 3, "reason": "blocked"}),
    )
    .unwrap();
    let b = digest(
        "project.pause",
        &json!({"reason": "blocked", "expectedVersion": 3}),
    )
    .unwrap();
    assert_eq!(a, b);
    assert_eq!(a.len(), 64);
    assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
    let other_body = digest(
        "project.pause",
        &json!({"expectedVersion": 3, "reason": "waiting"}),
    );
    assert_ne!(a, other_body.unwrap());
    let other_type = digest(
        "project.resume",
        &json!({"expectedVersion": 3, "reason": "blocked"}),
    );
    assert_ne!(a, other_type.unwrap());
}

#[test]
fn a_replay_returns_the_first_response_and_writes_nothing() {
    let mut home = Home::new();
    let first = home.create(request("k1", "SHIP")).unwrap();
    let after_first = home.counts();
    assert_eq!(home.count("project_command_receipts"), 1);

    let replayed = home.create(request("k1", "SHIP")).unwrap();
    assert_eq!(replayed, first);
    assert_eq!(
        serde_json::to_string(&replayed).unwrap(),
        serde_json::to_string(&first).unwrap()
    );
    assert_eq!(home.counts(), after_first, "a replay wrote a row");

    let (key, command, response): (String, String, String) = home
        .conn
        .query_row(
            "SELECT principal_type || ':' || principal_id || ':' || idempotency_key,
                    command_type, response
             FROM project_command_receipts",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(key, "user:local-operator:k1");
    assert_eq!(command, "project.create");
    assert_eq!(response, serde_json::to_string(&first).unwrap());
}

#[test]
fn another_body_under_the_key_is_a_conflict_that_writes_no_project_state() {
    let mut home = Home::new();
    home.create(request("k1", "SHIP")).unwrap();
    let before = home.counts();

    let mut changed = request("k1", "SHIP");
    changed.outcome = "Ship something else".to_string();
    let (code, class, message) = refusal(home.create(changed));
    assert_eq!((code, class), ("idempotency_conflict", Class::Conflict));
    assert_eq!(
        message,
        "This idempotency key was already used for a different command"
    );
    // A different id is a different command too.
    let mut with_id = request("k1", "SHIP");
    with_id.id = Some("0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f".to_string());
    assert_eq!(refusal(home.create(with_id)).0, "idempotency_conflict");

    let after = home.counts();
    let telemetry = after.len() - 1;
    assert_eq!(
        after[..telemetry],
        before[..telemetry],
        "project state changed"
    );
    assert_eq!(
        after[telemetry],
        before[telemetry] + 2,
        "one failure row each"
    );
}

#[test]
fn another_command_type_under_the_key_is_a_conflict_before_it_runs() {
    let mut home = Home::new();
    home.create(request("k1", "SHIP")).unwrap();
    let before = home.counts();
    let applied = Cell::new(0);
    let probe = Probe {
        key: "k1",
        command_type: "project.probe",
        fail: false,
        applied: &applied,
    };
    let (code, class, _) = refusal(home.run(&Envelope::local_operator(), probe));
    assert_eq!((code, class), ("idempotency_conflict", Class::Conflict));
    assert_eq!(applied.get(), 0, "the conflicting command ran");
    assert_eq!(home.counts(), before);
}

#[test]
fn two_principals_may_reuse_one_key() {
    let mut home = Home::new();
    let alice = Envelope::user("alice", Tier::Member);
    let bob = Envelope::user("bob", Tier::Member);
    let a = home.run(&alice, request("shared", "ALICE")).unwrap();
    let b = home.run(&bob, request("shared", "BOB")).unwrap();
    assert_ne!(a.project.id, b.project.id);
    assert_eq!(home.count("projects"), 2);
    assert_eq!(home.count("project_command_receipts"), 2);
    // Each principal still replays its own.
    assert_eq!(home.run(&alice, request("shared", "ALICE")).unwrap(), a);
}

#[test]
fn a_failed_command_leaves_no_receipt_and_its_retry_runs() {
    let mut home = Home::new();
    let applied = Cell::new(0);
    let probe = |fail| Probe {
        key: "k1",
        command_type: "project.probe",
        fail,
        applied: &applied,
    };
    let failed = home.run(&Envelope::local_operator(), probe(true));
    assert_eq!(refusal(failed).0, "internal");
    assert_eq!(home.count("projects"), 0, "the failed write survived");
    assert_eq!(home.count("project_command_receipts"), 0);

    let id = home.run(&Envelope::local_operator(), probe(false)).unwrap();
    assert_eq!(applied.get(), 2);
    assert_eq!(home.count("project_command_receipts"), 1);
    // And from here on it replays without running.
    let again = home.run(&Envelope::local_operator(), probe(false)).unwrap();
    assert_eq!((again, applied.get()), (id, 2));
}

#[test]
fn the_key_is_one_to_two_hundred_utf16_units() {
    let mut home = Home::new();
    let at_cap = "k".repeat(IDEMPOTENCY_KEY_MAX);
    home.create(request(&at_cap, "SHIP")).unwrap();
    // 100 astral characters are 200 UTF-16 units: still at the cap.
    home.create(request(&"😀".repeat(100), "EMOJI")).unwrap();
    let before = home.counts();
    for (key, reason) in [
        (String::new(), "too_short"),
        ("k".repeat(IDEMPOTENCY_KEY_MAX + 1), "too_long"),
        ("😀".repeat(101), "too_long"),
    ] {
        let e = home.create(request(&key, "NOPE")).unwrap_err();
        assert_eq!(classify(&e), ("invalid_request", Class::Unprocessable));
        let Error::Project(project) = &e else {
            panic!("not a project refusal: {e:?}")
        };
        let details = serde_json::to_value(project.details()).unwrap();
        assert_eq!(details["field"], "idempotencyKey", "{details}");
        assert_eq!(details["reason"], reason, "{details}");
    }
    let after = home.counts();
    let telemetry = after.len() - 1;
    assert_eq!(after[..telemetry], before[..telemetry]);
}

#[test]
fn a_stored_response_that_no_longer_parses_is_an_internal_error() {
    let mut home = Home::new();
    home.create(request("k1", "SHIP")).unwrap();
    home.conn
        .execute("UPDATE project_command_receipts SET response = '{'", [])
        .unwrap();
    let (code, class, message) = refusal(home.create(request("k1", "SHIP")));
    assert_eq!((code, class), ("internal_error", Class::Internal));
    assert!(message.contains("does not parse"), "{message}");
}
