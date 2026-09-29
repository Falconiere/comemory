#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! Test mirror for `src/domains/projects/lifecycle.rs`, against a real
//! migrated temp data directory: every source state against every verb, the
//! pause-reason and body rules, the agent and member refusals before any
//! write, the version check, one event per success, and the receipt's replay
//! and conflict edges.

use serde_json::{Value, json};

use super::{Body, Kind, Request, Response};
use crate::config::{Config, Paths};
use crate::domains::projects::authority::{self, Capabilities, Command, Envelope, Tier};
use crate::domains::projects::create;
use crate::errors::{Error, Result};
use crate::store::schema_projects::PROJECT_TABLES;
use crate::store::{Connection, connection};
use crate::utilities::context::Ctx;
use crate::utilities::error_code::{Class, classify};

/// Every status the platform declares.
const STATUSES: [&str; 6] = [
    "draft",
    "planning",
    "active",
    "paused",
    "completed",
    "canceled",
];

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

    /// A draft project at version 1, then `status` and `archived` seeded
    /// directly (no verb reaches `completed` or `canceled` yet) without
    /// moving its version.
    fn project(&mut self, key_prefix: &str, status: &str, archived: bool) -> String {
        let req: create::Request = serde_json::from_value(json!({
            "idempotencyKey": format!("create-{key_prefix}"),
            "name": format!("Project {key_prefix}"),
            "keyPrefix": key_prefix,
            "outcome": "Ship it",
        }))
        .unwrap();
        let id = self
            .run(&Envelope::local_operator(), req)
            .unwrap()
            .project
            .id;
        let archived_at = archived.then_some(1_727_481_600_000_i64);
        self.conn
            .execute(
                "UPDATE projects SET status = ?1, archived_at = ?2 WHERE id = ?3",
                rusqlite::params![status, archived_at, id],
            )
            .unwrap();
        id
    }

    /// `req` run by the local operator.
    fn operator(&mut self, req: Request) -> Result<Response> {
        self.run(&Envelope::local_operator(), req)
    }

    /// Row counts of every project table, `project_changes`, then
    /// `activity_log`.
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

    /// Every project-state count: all but the `activity_log` telemetry row a
    /// failed command still records.
    fn state(&self) -> Vec<i64> {
        let mut counts = self.counts();
        counts.pop();
        counts
    }

    fn count(&self, table: &str) -> i64 {
        let at = PROJECT_TABLES
            .iter()
            .chain(&["project_changes", "activity_log"])
            .position(|t| *t == table)
            .unwrap();
        self.counts()[at]
    }

    /// `(status, version, archived_at)` of `id`.
    fn row(&self, id: &str) -> (String, i64, Option<i64>) {
        self.conn
            .query_row(
                "SELECT status, version, archived_at FROM projects WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    }

    /// The newest activity event of `id`: `(type, actor, payload)`.
    fn last_event(&self, id: &str) -> (String, String, Value) {
        let (event, actor, payload): (String, String, String) = self
            .conn
            .query_row(
                "SELECT event_type, actor_principal_type || ':' || actor_principal_id, payload
                 FROM project_activity_events WHERE project_id = ?1
                 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        (event, actor, serde_json::from_str(&payload).unwrap())
    }
}

fn request(kind: Kind, id: &str, key: &str, version: i64, reason: Option<&str>) -> Request {
    Request {
        kind,
        id: id.to_string(),
        body: Body {
            workspace_id: None,
            idempotency_key: key.to_string(),
            expected_version: version,
            reason: reason.map(str::to_string),
        },
    }
}

/// The refusal's code, class, message and details.
fn refusal<T: std::fmt::Debug>(result: Result<T>) -> (&'static str, Class, String, Value) {
    let e = result.expect_err("expected a refusal");
    let (code, class) = classify(&e);
    let Error::Project(project) = &e else {
        panic!("not a project refusal: {e:?}")
    };
    let details = serde_json::to_value(project.details()).unwrap();
    (code, class, e.to_string(), details)
}

/// The spec's transition table: the `(status, archived)` a legal pair
/// leaves, or the platform's sentence for a refused one.
fn expected(kind: Kind, status: &str, archived: bool) -> std::result::Result<(&str, bool), &str> {
    let terminal = status == "completed" || status == "canceled";
    match kind {
        Kind::Restore if !archived => Err("This project is not archived"),
        Kind::Restore if terminal => Err("A completed or canceled project cannot be restored"),
        Kind::Restore => Ok((status, false)),
        _ if archived => Err("This project is archived"),
        Kind::Archive => Ok((status, true)),
        Kind::Pause if status == "active" => Ok(("paused", false)),
        Kind::Pause => Err("Only an active project can be paused"),
        Kind::Resume if status == "paused" => Ok(("active", false)),
        Kind::Resume => Err("Only a paused project can be resumed"),
    }
}

#[test]
fn every_source_state_against_every_verb() {
    let mut home = Home::new();
    let (mut legal, mut refused) = (0, 0);
    for (k, kind) in Kind::ALL.into_iter().enumerate() {
        for (s, status) in STATUSES.into_iter().enumerate() {
            for archived in [false, true] {
                let prefix = format!("P{k}{s}{}", u8::from(archived));
                let id = home.project(&prefix, status, archived);
                let before = home.counts();
                let result = home.operator(request(kind, &id, &prefix, 1, Some("because")));
                let pair = format!("{kind:?} from {status} (archived: {archived})");
                match expected(kind, status, archived) {
                    Ok((next_status, next_archived)) => {
                        legal += 1;
                        let project = result.unwrap_or_else(|e| panic!("{pair}: {e}")).project;
                        assert_eq!(project.status, next_status, "{pair}");
                        assert_eq!(project.archived_at.is_some(), next_archived, "{pair}");
                        assert_eq!(project.version, 2, "{pair}");
                        assert!(project.updated_at >= project.created_at, "{pair}");
                        assert_eq!(home.row(&id).0, next_status, "{pair}");
                        // One event, one feed frame, one receipt, one
                        // telemetry row — nothing else.
                        let grew: Vec<i64> = home
                            .counts()
                            .iter()
                            .zip(&before)
                            .map(|(a, b)| a - b)
                            .collect();
                        let mut want = vec![0; grew.len()];
                        for table in [
                            "project_activity_events",
                            "project_command_receipts",
                            "project_changes",
                            "activity_log",
                        ] {
                            let at = PROJECT_TABLES
                                .iter()
                                .chain(&["project_changes", "activity_log"])
                                .position(|t| *t == table)
                                .unwrap();
                            want[at] = 1;
                        }
                        assert_eq!(grew, want, "{pair}");
                        let (event, actor, payload) = home.last_event(&id);
                        assert_eq!(event, kind.event_type(), "{pair}");
                        assert_eq!(actor, "user:local-operator", "{pair}");
                        assert_eq!(payload, json!({"reason": "because"}), "{pair}");
                    }
                    Err(sentence) => {
                        refused += 1;
                        let (code, class, message, _) = refusal(result);
                        assert_eq!(
                            (code, class),
                            ("invalid_transition", Class::Conflict),
                            "{pair}"
                        );
                        assert_eq!(message, sentence, "{pair}");
                        let mut after = home.counts();
                        let telemetry = after.pop().unwrap() - before[before.len() - 1];
                        assert_eq!(after, before[..before.len() - 1], "{pair} wrote state");
                        assert_eq!(telemetry, 1, "{pair}: the failed run is recorded");
                        let archived_at = archived.then_some(1_727_481_600_000);
                        assert_eq!(home.row(&id), (status.into(), 1, archived_at), "{pair}");
                    }
                }
            }
        }
    }
    assert_eq!((legal, refused), (12, 36));
}

#[test]
fn a_blank_missing_or_over_long_reason_is_refused_and_changes_nothing() {
    let mut home = Home::new();
    let id = home.project("PAUSE", "active", false);
    let before = home.state();
    for blank in ["", "   ", "\t\n"] {
        let (code, class, _, details) =
            refusal(home.operator(request(Kind::Pause, &id, "k", 1, Some(blank))));
        assert_eq!((code, class), ("invalid_request", Class::Unprocessable));
        assert_eq!(details, json!({"field": "reason", "reason": "blank"}));
    }
    let (code, class, _, details) = refusal(home.operator(request(Kind::Pause, &id, "k", 1, None)));
    assert_eq!((code, class), ("invalid_request", Class::BadRequest));
    assert_eq!(details, json!({"field": "reason", "reason": "required"}));
    // 4001 UTF-16 units on a verb whose reason is optional.
    let long = "é".repeat(4001);
    let (code, class, _, details) =
        refusal(home.operator(request(Kind::Archive, &id, "k", 1, Some(&long))));
    assert_eq!((code, class), ("invalid_request", Class::Unprocessable));
    assert_eq!(
        details,
        json!({"field": "reason", "reason": "too_long", "limit": 4000})
    );
    assert_eq!(home.state(), before);
    assert_eq!(home.row(&id), ("active".into(), 1, None));

    // At the cap, and untrimmed in the payload.
    let at_cap = format!(" {} ", "x".repeat(3998));
    home.operator(request(Kind::Pause, &id, "k", 1, Some(&at_cap)))
        .unwrap();
    assert_eq!(home.last_event(&id).2, json!({"reason": at_cap}));
    // An absent optional reason is recorded as null.
    home.operator(request(Kind::Resume, &id, "k2", 2, None))
        .unwrap();
    assert_eq!(home.last_event(&id).2, json!({"reason": null}));
}

#[test]
fn a_malformed_id_or_version_is_a_bad_request_and_an_unknown_project_is_not_found() {
    let mut home = Home::new();
    let id = home.project("SHAPE", "active", false);
    let before = home.state();
    let (code, _, _, details) =
        refusal(home.operator(request(Kind::Archive, "nope", "k", 1, None)));
    assert_eq!(code, "invalid_request");
    assert_eq!(details, json!({"field": "projectId", "reason": "invalid"}));
    for version in [0, -1] {
        let (code, class, _, details) =
            refusal(home.operator(request(Kind::Archive, &id, "k", version, None)));
        assert_eq!((code, class), ("invalid_request", Class::BadRequest));
        assert_eq!(
            details,
            json!({"field": "expectedVersion", "reason": "invalid"})
        );
    }
    let unknown = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f";
    let (code, class, _, _) = refusal(home.operator(request(Kind::Archive, unknown, "k", 1, None)));
    assert_eq!((code, class), ("project_not_found", Class::NotFound));
    assert_eq!(home.state(), before);
    // A mixed-case id names the same project.
    let upper = id.to_uppercase();
    let archived = home
        .operator(request(Kind::Archive, &upper, "k", 1, None))
        .unwrap();
    assert_eq!(archived.project.id, id);
}

#[test]
fn a_stale_version_is_a_conflict_carrying_the_current_version() {
    let mut home = Home::new();
    let id = home.project("STALE", "active", false);
    home.operator(request(Kind::Pause, &id, "k1", 1, Some("waiting")))
        .unwrap();
    let before = home.state();
    let (code, class, message, details) =
        refusal(home.operator(request(Kind::Resume, &id, "k2", 1, None)));
    assert_eq!((code, class), ("version_conflict", Class::Conflict));
    assert_eq!(message, "The project has changed since it was last read");
    assert_eq!(
        details,
        json!({"code": "version_conflict", "currentVersion": 2})
    );
    assert_eq!(home.state(), before);
    // The version check comes before the transition check.
    let (code, ..) = refusal(home.operator(request(Kind::Pause, &id, "k3", 7, Some("again"))));
    assert_eq!(code, "version_conflict");
    // Retried at the current version, the same key runs.
    let resumed = home
        .operator(request(Kind::Resume, &id, "k2", 2, None))
        .unwrap();
    assert_eq!(
        (resumed.project.status.as_str(), resumed.project.version),
        ("active", 3)
    );
}

#[test]
fn an_agent_is_refused_before_the_store_is_opened() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().join("fresh"));
    let cfg = Config::defaults();
    let id = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f";
    for envelope in [
        Envelope::local_agent(),
        Envelope::agent("grant", Capabilities::none()),
    ] {
        for kind in Kind::ALL {
            let mut ctx = Ctx::lazy(&paths, &cfg);
            let result = authority::run(&mut ctx, &envelope, request(kind, id, "k", 1, None));
            let (code, class, message, _) = refusal(result);
            assert_eq!((code, class), ("project_agent_scope", Class::Forbidden));
            assert_eq!(message, "This command requires a signed-in human");
        }
    }
    assert!(!paths.db_path().exists(), "a refusal opened the database");

    // Against a real project, an agent writes nothing, telemetry included.
    let mut home = Home::new();
    let id = home.project("AGENT", "active", false);
    let before = home.counts();
    for kind in Kind::ALL {
        let reason = Some("agent says so");
        let result = home.run(&Envelope::local_agent(), request(kind, &id, "k", 1, reason));
        assert_eq!(refusal(result).0, "project_agent_scope");
    }
    assert_eq!(home.counts(), before);
}

#[test]
fn a_member_is_forbidden_and_a_lead_is_admitted() {
    let mut home = Home::new();
    let id = home.project("TIER", "active", false);
    let before = home.counts();
    let member = Envelope::user("member-1", Tier::Member);
    for kind in Kind::ALL {
        let result = home.run(&member, request(kind, &id, "k", 1, Some("why")));
        let (code, class, message, _) = refusal(result);
        assert_eq!((code, class), ("forbidden", Class::Forbidden));
        assert_eq!(
            message,
            "Only the project lead or a workspace admin may run this command"
        );
    }
    assert_eq!(home.counts(), before);
    let lead = Envelope::user("lead-1", Tier::Lead);
    home.run(&lead, request(Kind::Pause, &id, "k", 1, Some("why")))
        .unwrap();
    assert_eq!(home.last_event(&id).1, "user:lead-1");
}

#[test]
fn a_replay_writes_nothing_and_any_other_reuse_of_the_key_conflicts() {
    let mut home = Home::new();
    let a = home.project("AAA", "active", false);
    let b = home.project("BBB", "active", false);
    let first = home
        .operator(request(Kind::Archive, &a, "k", 1, Some("done")))
        .unwrap();
    let after_first = home.counts();
    assert_eq!(home.count("project_command_receipts"), 3);

    let replayed = home
        .operator(request(Kind::Archive, &a, "k", 1, Some("done")))
        .unwrap();
    assert_eq!(replayed, first);
    assert_eq!(home.counts(), after_first, "a replay wrote a row");

    for (kind, id, version, reason) in [
        (Kind::Pause, a.as_str(), 1, Some("done")),
        (Kind::Archive, a.as_str(), 1, Some("other")),
        (Kind::Archive, a.as_str(), 2, Some("done")),
        (Kind::Archive, b.as_str(), 1, Some("done")),
    ] {
        let (code, class, ..) = refusal(home.operator(request(kind, id, "k", version, reason)));
        assert_eq!(
            (code, class),
            ("idempotency_conflict", Class::Conflict),
            "{kind:?} on {id} at {version} with {reason:?}"
        );
    }
    assert_eq!(home.state(), after_first[..after_first.len() - 1]);
    assert_eq!(home.row(&b), ("active".into(), 1, None));
}

#[test]
fn a_failed_command_leaves_no_receipt_so_its_retry_runs() {
    let mut home = Home::new();
    let id = home.project("RETRY", "draft", false);
    let (code, ..) = refusal(home.operator(request(Kind::Pause, &id, "k", 1, Some("wait"))));
    assert_eq!(code, "invalid_transition");
    home.conn
        .execute("UPDATE projects SET status = 'active' WHERE id = ?1", [&id])
        .unwrap();
    let paused = home
        .operator(request(Kind::Pause, &id, "k", 1, Some("wait")))
        .unwrap();
    assert_eq!(paused.project.status, "paused");
}
