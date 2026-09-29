#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/local_only.rs` (AC-7): a mutation
//! run through the real authority gate records its event with the shared
//! writer, which reports a bound project as `local_only`, naming the side the
//! change will not reach, and an unbound one as `None`; the event is written
//! either way, and nothing about the binding can fail the mutation.

use comemory::config::{Config, Paths};
use comemory::domains::projects::activity::{self, Event, Recorded};
use comemory::domains::projects::authority::{self, Actor, Command, Envelope, Verb, sealed};
use comemory::domains::projects::binding::{self, Direction, NewBinding};
use comemory::errors::Result;
use comemory::store::Connection;
use comemory::store::connection::{self, write_transaction};
use comemory::utilities::context::Ctx;

const SEED: &str = include_str!("../../../../tests/fixtures/projects/every_table_seed.sql");
const BOUND: &str = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f";
const UNBOUND: &str = "5d3b9a41-6c2e-4f7a-8b1d-2e9c0a7f4b36";

/// A stand-in lifecycle mutation (#328 has not landed): one
/// `project.paused` event on `project_id`, through the shared writer, in its
/// own transaction — exactly what a real mutation core does.
struct Pause {
    project_id: &'static str,
}

impl sealed::Sealed for Pause {}

impl Command for Pause {
    type Response = Recorded;

    fn verb(&self) -> Verb {
        Verb::Pause
    }

    fn execute(self, ctx: &mut Ctx<'_>, actor: &Actor) -> Result<Recorded> {
        let tx = write_transaction(ctx.conn()?)?;
        let payload = serde_json::json!({});
        let event = Event {
            project_id: self.project_id,
            event_type: "project.paused",
            entity_type: "project",
            entity_id: self.project_id,
            payload: &payload,
        };
        let recorded = activity::record(&tx, actor, &event, 1_759_000_100_000)?;
        tx.commit()?;
        Ok(recorded)
    }
}

/// A migrated data directory holding the seed (whose project is bound) and a
/// second, unbound project.
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
        conn.execute_batch(SEED).unwrap();
        conn.execute_batch(&format!(
            "INSERT INTO projects (id, slug, key_prefix, name, outcome, lead_principal_type,
                 lead_principal_id, creator_principal_type, creator_principal_id)
             VALUES ('{UNBOUND}', 'beta', 'BET', 'Beta', 'ship beta', 'user', 'u1', 'user', 'u1');"
        ))
        .unwrap();
        Self {
            _dir: dir,
            paths,
            cfg: Config::defaults(),
            conn,
        }
    }

    fn pause(&mut self, project_id: &'static str) -> Recorded {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        authority::run(&mut ctx, &Envelope::local_operator(), Pause { project_id }).unwrap()
    }

    fn paused_events(&self) -> i64 {
        self.conn
            .query_row(
                "SELECT count(*) FROM project_activity_events WHERE event_type = 'project.paused'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }
}

#[test]
fn activity_record_flags_a_bound_project_as_local_only() {
    let mut home = Home::new();

    let bound = home.pause(BOUND);
    let warning = bound.local_only.expect("a bound project warns");
    assert_eq!(warning.code, "local_only");
    assert_eq!(warning.project_id, BOUND);
    assert_eq!(warning.direction, "imported");
    assert_eq!(warning.remote, "ws-origin");
    assert_eq!(
        warning.message,
        "This project was imported from ws-origin; this change stays in this data \
         directory and will not reach ws-origin"
    );

    let unbound = home.pause(UNBOUND);
    assert!(unbound.local_only.is_none());
    assert_ne!(bound.id, unbound.id);

    binding::record(
        &home.conn,
        &NewBinding {
            project_id: UNBOUND,
            direction: Direction::Exported,
            remote: "ws-target",
            digest: &"e".repeat(64),
            remapped_from: None,
            at_ms: 1_759_000_100_002,
        },
    )
    .unwrap();
    let exported = home
        .pause(UNBOUND)
        .local_only
        .expect("an exported project warns too");
    assert_eq!(
        exported.message,
        "This project was exported to ws-target; this change stays in this data \
         directory and will not reach ws-target"
    );
    assert_eq!(
        home.paused_events(),
        3,
        "the warning never refuses the mutation"
    );
}

/// The warning never fails the mutation it annotates: a binding whose
/// `transferred_at` no timestamp can render still yields `local_only`, and
/// the event is written.
#[test]
fn an_unrenderable_binding_time_never_fails_the_mutation() {
    let mut home = Home::new();
    home.conn
        .execute_batch(&format!(
            "UPDATE project_transfer_bindings SET transferred_at = {} WHERE project_id = '{BOUND}';",
            i64::MAX
        ))
        .unwrap();

    let recorded = home.pause(BOUND);
    assert_eq!(recorded.local_only.unwrap().remote, "ws-origin");
    assert_eq!(home.paused_events(), 1);
}
