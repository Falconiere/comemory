#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/changes.rs`, against a real temp
//! data directory: one frame per committed create and none for a refused,
//! replayed or rolled-back one; a deletion frame that outlives the project;
//! and every cursor or page size the feed cannot serve refused explicitly.

use comemory::config::{Config, Paths};
use comemory::domains::projects::authority::{self, Envelope};
use comemory::domains::projects::changes::{self, ChangeFrame};
use comemory::domains::projects::create;
use comemory::errors::{Error, Result};
use comemory::store::connection::write_transaction;
use comemory::store::{Connection, connection};
use comemory::utilities::context::Ctx;
use comemory::utilities::error_code::{Class, classify};

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

    fn create(&mut self, id: Option<&str>, key_prefix: &str) -> Result<create::Response> {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        let req = create::Request {
            id: id.map(str::to_string),
            workspace_id: None,
            name: format!("Project {key_prefix}"),
            key_prefix: key_prefix.to_string(),
            outcome: "Ship the loop".to_string(),
            success_criteria: vec!["Tests pass".to_string()],
            constraints: Vec::new(),
            non_goals: Vec::new(),
            repositories: Vec::new(),
            lead_user_id: None,
            target_date: None,
        };
        authority::run(&mut ctx, &Envelope::local_operator(), req)
    }

    fn read(&mut self, after: Option<i64>, limit: Option<i64>) -> Result<Vec<ChangeFrame>> {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        let req = changes::Request { after, limit };
        authority::run(&mut ctx, &Envelope::local_operator(), req)
    }

    fn feed(&mut self) -> Vec<ChangeFrame> {
        self.read(None, Some(1000)).unwrap()
    }

    /// The `project.created` event id of `project_id`.
    fn created_event(&self, project_id: &str) -> String {
        self.conn
            .query_row(
                "SELECT id FROM project_activity_events WHERE project_id = ?1",
                [project_id],
                |r| r.get(0),
            )
            .unwrap()
    }
}

/// The refusal's class and its serialized details.
fn refusal<T: std::fmt::Debug>(result: Result<T>) -> (Class, String) {
    let e = result.expect_err("expected a refusal");
    let Error::Project(project) = &e else {
        panic!("not a project refusal: {e:?}")
    };
    (
        classify(&e).1,
        serde_json::to_string(&project.details()).unwrap(),
    )
}

const ID: &str = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2a";

#[test]
fn a_committed_create_emits_exactly_one_frame_naming_its_event() {
    let mut home = Home::new();
    assert!(home.feed().is_empty());
    let project = home.create(Some(ID), "SHIP").unwrap().project;
    let event = home.created_event(&project.id);
    assert_eq!(
        home.feed(),
        [ChangeFrame {
            seq: 1,
            entity: "project",
            project_id: ID.to_string(),
            event_id: event,
            op: "changed".to_string(),
        }]
    );
    let entity_type: String = home
        .conn
        .query_row("SELECT entity_type FROM project_changes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(entity_type, "project");
}

#[test]
fn refused_and_replayed_creates_emit_nothing() {
    let mut home = Home::new();
    home.create(Some(ID), "SHIP").unwrap();
    let before = home.feed();

    // Refused before the transaction: an invalid key prefix.
    refusal(home.create(None, "1X"));
    // Refused inside it: a taken key prefix.
    refusal(home.create(None, "SHIP"));
    // Today's replay of a client-generated-id command: the same id and body.
    let (class, details) = refusal(home.create(Some(ID), "SHIP"));
    assert_eq!(class, Class::Unprocessable);
    assert!(details.contains("duplicate"), "{details}");

    assert_eq!(home.feed(), before);
}

#[test]
fn a_mutation_rolled_back_after_its_feed_row_emits_nothing() {
    let mut home = Home::new();
    home.conn
        .execute_batch(
            "CREATE TRIGGER refuse_change AFTER INSERT ON project_changes
             BEGIN SELECT RAISE(ABORT, 'change refused'); END;",
        )
        .unwrap();
    let e = home.create(Some(ID), "SHIP").unwrap_err();
    assert!(e.to_string().contains("change refused"), "{e}");
    home.conn
        .execute_batch("DROP TRIGGER refuse_change")
        .unwrap();
    assert!(home.feed().is_empty());
    let projects: i64 = home
        .conn
        .query_row("SELECT count(*) FROM projects", [], |r| r.get(0))
        .unwrap();
    assert_eq!(projects, 0, "the project rolled back with its feed row");
}

#[test]
fn a_deletion_frame_outlives_the_project_and_is_served() {
    let mut home = Home::new();
    home.create(Some(ID), "SHIP").unwrap();
    let tx = write_transaction(&mut home.conn).unwrap();
    changes::record_deletion(&tx, ID, 42).unwrap();
    tx.execute("DELETE FROM projects WHERE id = ?1", [ID])
        .unwrap();
    tx.commit().unwrap();

    let ops: Vec<(i64, String, String, String)> = home
        .feed()
        .into_iter()
        .map(|f| (f.seq, f.project_id, f.event_id, f.op))
        .collect();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0].3, "changed");
    assert_eq!(
        ops[1],
        (2, ID.to_string(), ID.to_string(), "deleted".to_string())
    );
}

#[test]
fn cursors_page_forward_and_out_of_window_cursors_are_refused() {
    let mut home = Home::new();
    for key in ["AA", "BB", "CC"] {
        home.create(None, key).unwrap();
    }
    let seqs = |frames: Vec<ChangeFrame>| frames.iter().map(|f| f.seq).collect::<Vec<_>>();
    assert_eq!(seqs(home.read(Some(1), Some(1)).unwrap()), [2]);
    assert_eq!(seqs(home.read(Some(1), None).unwrap()), [2, 3]);
    assert!(
        home.read(Some(3), None).unwrap().is_empty(),
        "the head is a valid cursor"
    );

    let cases = [
        (
            Some(4),
            None,
            Class::Unprocessable,
            r#"{"field":"after","reason":"cursor_ahead"}"#,
        ),
        (
            Some(-1),
            None,
            Class::BadRequest,
            r#"{"field":"after","reason":"invalid"}"#,
        ),
        (
            None,
            Some(0),
            Class::Unprocessable,
            r#"{"field":"limit","reason":"too_small","limit":1}"#,
        ),
        (
            None,
            Some(1001),
            Class::Unprocessable,
            r#"{"field":"limit","reason":"too_large","limit":1000}"#,
        ),
    ];
    for (after, limit, class, details) in cases {
        assert_eq!(
            refusal(home.read(after, limit)),
            (class, details.to_string())
        );
    }

    // A pruned window: rows 1 and 2 are gone, so a reader at 0 or 1 would
    // silently skip. It is refused; a reader at 2 continues.
    home.conn
        .execute_batch("DELETE FROM project_changes WHERE seq <= 2")
        .unwrap();
    let expired = r#"{"field":"after","reason":"cursor_expired"}"#;
    for after in [0, 1] {
        assert_eq!(
            refusal(home.read(Some(after), None)),
            (Class::Unprocessable, expired.to_string())
        );
    }
    assert_eq!(seqs(home.read(Some(2), None).unwrap()), [3]);
}

#[test]
fn an_empty_feed_serves_only_the_start() {
    let mut home = Home::new();
    assert!(home.read(Some(0), None).unwrap().is_empty());
    assert_eq!(
        refusal(home.read(Some(1), None)).1,
        r#"{"field":"after","reason":"cursor_ahead"}"#
    );
}
