#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! Test mirror for `src/domains/projects/import.rs`, between real data
//! directories: a project exported from one loaded with the every-table seed
//! lands in another under the same ids (AC-1); an identical repeat changes no
//! project row (AC-2); a differing copy is skipped with both intact (AC-3); a
//! taken key prefix or slug (AC-4), a child id held elsewhere (AC-10) and a
//! tampered bundle (AC-9) are refused with nothing written; a remap rewrites
//! only the matching actor and repeats as `unchanged` (AC-8).

use comemory::config::{Config, Paths};
use comemory::domains::projects::authority::{self, Envelope, Tier};
use comemory::domains::projects::binding;
use comemory::domains::projects::bundle::{ActorRemap, Bundle};
use comemory::domains::projects::export::{self, snapshot};
use comemory::domains::projects::import::{Import, Outcome, Response};
use comemory::domains::projects::principal::{LOCAL_OPERATOR_ID, Principal, PrincipalType};
use comemory::errors::{Error, Result};
use comemory::store::schema_projects::PROJECT_TABLES;
use comemory::store::{Connection, connection};
use comemory::utilities::context::Ctx;
use comemory::utilities::error_code::{Class, classify};
use serde_json::{Value, json};

const SEED: &str = include_str!("../../../../tests/fixtures/projects/every_table_seed.sql");
const A: &str = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f";

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

    fn seeded() -> Self {
        let home = Self::new();
        home.conn.execute_batch(SEED).unwrap();
        home
    }

    fn export(&mut self) -> Bundle {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        authority::run(
            &mut ctx,
            &Envelope::local_operator(),
            export::Request { id: A.into() },
        )
        .unwrap()
    }

    fn import(&mut self, bundle: &Bundle, remap: Option<ActorRemap>) -> Result<Response> {
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn);
        let import = Import {
            bundle: bundle.clone(),
            remote: "file:/tmp/alpha.json".into(),
            remap,
        };
        authority::run(&mut ctx, &Envelope::local_operator(), import)
    }

    /// Every row of every project table, sorted: what "nothing written"
    /// compares.
    fn project_state(&self) -> Vec<(String, Vec<String>)> {
        PROJECT_TABLES
            .iter()
            .map(|table| {
                let mut statement = self
                    .conn
                    .prepare(&format!("SELECT * FROM \"{table}\""))
                    .unwrap();
                let width = statement.column_count();
                let mut rows: Vec<String> = statement
                    .query_map([], |r| {
                        (0..width)
                            .map(|i| r.get::<_, rusqlite::types::Value>(i))
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .unwrap()
                    .map(|row| format!("{:?}", row.unwrap()))
                    .collect();
                rows.sort();
                ((*table).to_string(), rows)
            })
            .collect()
    }

    /// The row count of every table but `activity_log` (command telemetry,
    /// which every run writes): "nothing written" beyond the project tables
    /// too — no journal, no `sync_log`.
    fn all_counts(&self) -> Vec<(String, i64)> {
        let names: Vec<String> = self
            .conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table'
                 AND name NOT LIKE 'sqlite_%' AND name != 'activity_log' ORDER BY name",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        names
            .into_iter()
            .map(|name| {
                let n = self
                    .conn
                    .query_row(&format!("SELECT count(*) FROM \"{name}\""), [], |r| {
                        r.get(0)
                    })
                    .unwrap();
                (name, n)
            })
            .collect()
    }

    fn scalar(&self, sql: &str) -> Value {
        let value: rusqlite::types::Value = self.conn.query_row(sql, [], |r| r.get(0)).unwrap();
        match value {
            rusqlite::types::Value::Text(s) => Value::from(s),
            rusqlite::types::Value::Integer(i) => Value::from(i),
            _ => Value::Null,
        }
    }
}

/// The refusal's class and serialized details.
fn refusal(result: Result<Response>) -> (Class, Value) {
    let e = result.expect_err("expected a refusal");
    let Error::Project(project) = &e else {
        panic!("not a project refusal: {e:?}")
    };
    (
        classify(&e).1,
        serde_json::to_value(project.details()).unwrap(),
    )
}

#[test]
fn import_round_trips_into_an_empty_directory() {
    let mut a = Home::seeded();
    let bundle = a.export();
    let mut b = Home::new();

    let done = b.import(&bundle, None).unwrap();
    assert_eq!(done.outcome, Outcome::Imported);
    assert_eq!(done.digest, bundle.digest);
    assert_eq!(done.local_digest, None);
    let total: usize = bundle.tables.iter().map(|t| t.rows.len()).sum();
    assert_eq!(done.rows, total);

    assert_eq!(
        snapshot(&b.conn, A).unwrap(),
        bundle,
        "every carried row, column for column"
    );
    assert_eq!(b.scalar("SELECT count(*) FROM project_command_receipts"), 0);
    assert_eq!(b.scalar("SELECT count(*) FROM project_activity_events"), 1);
    let bound = binding::find(&b.conn, A).unwrap().unwrap();
    assert_eq!(
        (
            bound.direction.as_str(),
            bound.remote.as_str(),
            bound.digest.as_str()
        ),
        ("imported", "file:/tmp/alpha.json", bundle.digest.as_str())
    );
    assert_eq!(bound.remapped_from, None);
    let frames: (i64, String, String, String) = b
        .conn
        .query_row(
            "SELECT count(*), max(project_id), max(event_id), max(op) FROM project_changes",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        frames,
        (1, A.to_string(), A.to_string(), "changed".to_string()),
        "one body-free frame tells a connected console the project arrived"
    );
}

#[test]
fn identical_reimport_changes_no_project_row() {
    let bundle = Home::seeded().export();
    let mut b = Home::new();
    b.import(&bundle, None).unwrap();
    let before = b.project_state();
    let counts = b.all_counts();

    let again = b.import(&bundle, None).unwrap();
    assert_eq!(again.outcome, Outcome::Unchanged);
    assert_eq!(
        b.all_counts(),
        counts,
        "no table but activity_log gained a row"
    );
    assert_eq!(again.local_digest.as_deref(), Some(bundle.digest.as_str()));
    assert_eq!(again.rows, 0);
    assert_eq!(b.project_state(), before, "binding included");
}

#[test]
fn differing_import_is_skipped_with_both_digests() {
    let mut a = Home::seeded();
    let bundle = a.export();
    let mut b = Home::new();
    b.import(&bundle, None).unwrap();
    b.conn
        .execute_batch("UPDATE project_work_items SET title = 'edited in B' WHERE id = 'w-1';")
        .unwrap();
    let before = b.project_state();
    let counts = b.all_counts();

    let skipped = b.import(&bundle, None).unwrap();
    assert_eq!(skipped.outcome, Outcome::Skipped);
    assert_eq!(
        b.all_counts(),
        counts,
        "no table but activity_log gained a row"
    );
    let local = skipped.local_digest.clone().unwrap();
    assert_ne!(local, skipped.digest);
    assert_eq!(skipped.digest, bundle.digest);
    assert_eq!(b.project_state(), before);
    assert_eq!(
        b.scalar("SELECT title FROM project_work_items WHERE id = 'w-1'"),
        "edited in B"
    );
    assert_eq!(
        a.scalar("SELECT title FROM project_work_items WHERE id = 'w-1'"),
        "root"
    );
    assert_eq!(snapshot(&b.conn, A).unwrap().digest, local);
}

#[test]
fn key_prefix_and_slug_collisions_are_refused_with_details() {
    let bundle = Home::seeded().export();
    for (sql, expected) in [
        (
            "INSERT INTO projects (id, slug, key_prefix, name, outcome, lead_principal_type,
                 lead_principal_id, creator_principal_type, creator_principal_id)
             VALUES ('11111111-1111-4111-8111-111111111111', 'other', 'ALP', 'Other', 'o',
                 'user', 'u1', 'user', 'u1');",
            json!({"field": "keyPrefix", "reason": "duplicate", "value": "ALP"}),
        ),
        (
            "INSERT INTO projects (id, slug, key_prefix, name, outcome, lead_principal_type,
                 lead_principal_id, creator_principal_type, creator_principal_id)
             VALUES ('11111111-1111-4111-8111-111111111111', 'alpha', 'OTH', 'Alpha', 'o',
                 'user', 'u1', 'user', 'u1');",
            json!({"field": "slug", "reason": "duplicate", "value": "alpha"}),
        ),
    ] {
        let mut c = Home::new();
        c.conn.execute_batch(sql).unwrap();
        let before = c.project_state();
        let result = c.import(&bundle, None);
        let message = result.as_ref().map(|_| ()).unwrap_err().to_string();
        let (class, details) = refusal(result);
        assert_eq!(class, Class::Unprocessable);
        assert_eq!(details, expected);
        let field = expected["field"].as_str().unwrap();
        assert_eq!(
            message,
            format!("{field} is already used in this workspace")
        );
        assert_eq!(
            c.project_state(),
            before,
            "nothing of the bundle was written"
        );
    }
}

#[test]
fn a_child_id_held_elsewhere_is_a_conflict_and_rolls_back() {
    let bundle = Home::seeded().export();
    let mut c = Home::new();
    c.conn
        .execute_batch(
            "INSERT INTO projects (id, slug, key_prefix, name, outcome, lead_principal_type,
                 lead_principal_id, creator_principal_type, creator_principal_id)
             VALUES ('22222222-2222-4222-8222-222222222222', 'held', 'HLD', 'Held', 'o',
                 'user', 'u1', 'user', 'u1');
             INSERT INTO project_milestones (id, project_id, name, description, target_date)
             VALUES ('m-1', '22222222-2222-4222-8222-222222222222', 'Mine', 'd', 1);",
        )
        .unwrap();
    let before = c.project_state();

    let (class, details) = refusal(c.import(&bundle, None));
    assert_eq!(class, Class::Unprocessable);
    assert_eq!(
        details,
        json!({"field": "bundle", "reason": "conflict", "table": "project_milestones"})
    );
    assert_eq!(
        c.project_state(),
        before,
        "the project row written first rolled back"
    );
}

#[test]
fn tampered_bundle_is_refused_before_any_write() {
    let mut tampered = Home::seeded().export();
    let items = tampered
        .tables
        .iter_mut()
        .find(|t| t.table == "project_work_items")
        .unwrap();
    let title = items.columns.iter().position(|c| c == "title").unwrap();
    items.rows[0][title] = json!("not what was digested");
    let mut b = Home::new();
    let before = b.project_state();

    let (class, details) = refusal(b.import(&tampered, None));
    assert_eq!(class, Class::BadRequest);
    assert_eq!(details, json!({"field": "digest", "reason": "mismatch"}));
    assert_eq!(b.project_state(), before);
}

#[test]
fn remapped_import_rewrites_actors_records_origin_and_repeats_unchanged() {
    let bundle = Home::seeded().export();
    let mut b = Home::new();
    let remap = || {
        Some(ActorRemap {
            from: Principal::new(PrincipalType::User, LOCAL_OPERATOR_ID),
            to: Principal::new(PrincipalType::User, "u-platform"),
        })
    };

    let done = b.import(&bundle, remap()).unwrap();
    assert_eq!(done.outcome, Outcome::Imported);
    assert_ne!(
        done.digest, bundle.digest,
        "the effective digest is post-remap"
    );
    assert_eq!(snapshot(&b.conn, A).unwrap().digest, done.digest);
    assert_eq!(
        b.scalar("SELECT creator_principal_id FROM projects"),
        "u-platform"
    );
    assert_eq!(b.scalar("SELECT lead_principal_id FROM projects"), "u-lead");
    assert_eq!(
        b.scalar("SELECT count(*) FROM project_work_items WHERE assignee_principal_id = 'g-1'"),
        1,
        "the project agent is untouched"
    );
    assert_eq!(
        b.scalar("SELECT count(*) FROM project_activity_events WHERE actor_principal_id = 'local-operator'"),
        0
    );
    let bound = binding::find(&b.conn, A).unwrap().unwrap();
    assert_eq!(bound.remapped_from.as_deref(), Some("user:local-operator"));
    assert_eq!(bound.digest, done.digest);

    assert_eq!(
        b.import(&bundle, remap()).unwrap().outcome,
        Outcome::Unchanged
    );
    assert_eq!(
        b.import(&bundle, None).unwrap().outcome,
        Outcome::Skipped,
        "without the remap the same bundle differs from the stored copy"
    );
}

/// Another engine may send rows in any order: the import puts them in
/// canonical order before verifying the digest, so a permuted bundle is the
/// same bundle.
#[test]
fn a_bundle_with_rows_in_another_order_imports_under_the_same_digest() {
    let bundle = Home::seeded().export();
    let mut permuted = bundle.clone();
    for table in &mut permuted.tables {
        table.rows.reverse();
    }
    permuted.tables.reverse();
    assert_ne!(permuted, bundle);
    let mut b = Home::new();

    let done = b.import(&permuted, None).unwrap();
    assert_eq!(done.outcome, Outcome::Imported);
    assert_eq!(done.digest, bundle.digest);
    assert_eq!(snapshot(&b.conn, A).unwrap(), bundle);
}

/// The transfer verbs are the envelope's to admit: a lead is refused import
/// (owner-only) and the local agent is refused both — each before the store,
/// with the platform-shaped sentence, and nothing written.
#[test]
fn transfer_verbs_refuse_an_agent_and_a_lead_import_before_the_store() {
    let bundle = Home::seeded().export();
    let mut b = Home::new();
    let before = b.all_counts();
    let refused = |b: &mut Home, envelope: &Envelope, import: bool| {
        let mut ctx = Ctx::borrowed(&b.paths, &b.cfg, &mut b.conn);
        let e = if import {
            let request = Import {
                bundle: bundle.clone(),
                remote: "file:/tmp/alpha.json".into(),
                remap: None,
            };
            authority::run(&mut ctx, envelope, request)
                .map(|_| ())
                .unwrap_err()
        } else {
            let request = export::Request { id: A.into() };
            authority::run(&mut ctx, envelope, request)
                .map(|_| ())
                .unwrap_err()
        };
        (classify(&e).0, e.to_string())
    };
    let lead = Envelope::user("lee", Tier::Lead);
    assert_eq!(
        refused(&mut b, &lead, true),
        (
            "forbidden",
            "Only a workspace owner or admin can import a project".to_string()
        )
    );
    let agent = Envelope::local_agent();
    let human_only = (
        "project_agent_scope",
        "This command requires a signed-in human".to_string(),
    );
    assert_eq!(refused(&mut b, &agent, true), human_only);
    assert_eq!(refused(&mut b, &agent, false), human_only);
    assert_eq!(b.all_counts(), before, "a refused transfer writes nothing");
    let logged: i64 = b
        .conn
        .query_row("SELECT count(*) FROM activity_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(logged, 0, "refused before the core, so not even telemetry");
}
