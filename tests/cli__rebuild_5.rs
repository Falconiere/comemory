#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory rebuild` — part 5: the fourteen project tables (#325).
//!
//! Listing a table in `COPIED_TABLES` satisfies the coverage test and copies
//! nothing on its own (`activity_log` once shipped that way), so this drives
//! the real binary over rows in every project table — every nullable column
//! set in at least one row, both self-references present — and compares each
//! table column for column, by the columns its declaration names.

use assert_cmd::Command;
use comemory::store::schema_projects::{PROJECT_TABLES, table_defs};
use rusqlite::Connection;
use rusqlite::types::Value;
use tempfile::tempdir;

fn run(home: &std::path::Path, args: &[&str]) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home)
        .env("HOME", home)
        .args(args)
        .assert()
        .success();
}

fn db(home: &std::path::Path) -> Connection {
    let conn = Connection::open(home.join("comemory.db")).expect("open db");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enforce keys");
    conn
}

/// One project with a row in every project table, written through real
/// SQLite with foreign keys enforced. `a2` nests under `a1`, and `e1` is
/// superseded by `e2`, so both self-references travel through the copy.
const SEED: &str = "
INSERT INTO projects (id, slug, key_prefix, name, outcome, constraints, non_goals, status,
    health, lead_principal_type, lead_principal_id, target_date, current_plan_version, version,
    completion_policy, creator_principal_type, creator_principal_id, created_at, updated_at,
    archived_at)
VALUES ('p-a', 'alpha', 'ALP', 'Alpha', 'ship alpha', '[\"no downtime\"]', '[\"mobile\"]',
    'active', 'on_track', 'user', 'u-lead', 1767225600000, 2, 7, 'manual', 'user', 'u-maker',
    1759000000000, 1759000000500, 1759000000900);
INSERT INTO project_repositories (project_id, repo, creator_principal_type, creator_principal_id,
    created_at)
VALUES ('p-a', 'Falconiere/comemory', 'user', 'u-maker', 1759000001000);
INSERT INTO project_milestones (id, project_id, name, description, target_date, position, status,
    archived_at)
VALUES ('m-1', 'p-a', 'Beta', 'first cut', 1767225600000, 3, 'active', 1759000002000);
INSERT INTO project_work_items (id, project_id, number, parent_work_item_id, milestone_id, kind,
    title, description, status, priority, estimate, assignee_principal_type,
    assignee_principal_id, repo, version, position, archived_at, created_at, updated_at)
VALUES ('w-1', 'p-a', 1, NULL, 'm-1', 'feature', 'root', 'the root item', 'in_progress', 'high',
        5, 'user', 'u-dev', 'Falconiere/comemory', 2, 1, NULL, 1759000003000, 1759000003100),
       ('w-2', 'p-a', 2, 'w-1', 'm-1', 'task', 'child', 'nested item', 'blocked', 'urgent', 3,
        'project_agent', 'g-1', 'Falconiere/comemory', 4, 2, 1759000003900, 1759000003200,
        1759000003300);
INSERT INTO project_work_item_dependencies (project_id, blocker_id, blocked_id)
VALUES ('p-a', 'w-1', 'w-2');
INSERT INTO project_criteria (id, project_id, work_item_id, description, required,
    evidence_requirement, resolution, resolution_rationale, resolver_principal_type,
    resolver_principal_id, position, archived_at)
VALUES ('c-1', 'p-a', 'w-2', 'tests pass', 0, 'verified', 'waived', 'covered upstream', 'user',
    'u-lead', 4, 1759000004000);
INSERT INTO project_executions (id, project_id, work_item_id, actor_principal_type,
    actor_principal_id, state, started_at, heartbeat_at, finished_at, result_summary, version,
    superseded_by_execution_id)
VALUES ('e-2', 'p-a', 'w-2', 'project_agent', 'g-1', 'finished', 1759000005000, 1759000005100,
        1759000005200, 'resumed and done', 3, NULL),
       ('e-1', 'p-a', 'w-2', 'project_agent', 'g-1', 'canceled', 1759000004500, 1759000004600,
        1759000004700, 'went stale', 2, 'e-2');
INSERT INTO project_work_packets (id, project_id, execution_id, work_item_id, plan_version,
    work_item_version, engine_query_id, citations, requester_principal_type,
    requester_principal_id, created_at)
VALUES ('k-1', 'p-a', 'e-2', 'w-2', 2, 4, 'q-20260928-abc', '{\"queryId\":\"q\",\"items\":[]}',
    'project_agent', 'g-1', 1759000005300);
INSERT INTO project_plan_proposals (id, project_id, base_plan_version, state, operations,
    assumptions, risks, rationale, proposer_principal_type, proposer_principal_id,
    request_digest, created_at, updated_at)
VALUES ('r-1', 'p-a', 1, 'approved', '[{\"op\":\"add\"}]', '[\"a\"]', '[\"r\"]', 'grow scope',
    'project_agent', 'g-1', 'sha256:aa', 1759000006000, 1759000006100);
INSERT INTO project_approvals (id, project_id, proposal_id, decision, reviewer_principal_type,
    reviewer_principal_id, rationale, created_at)
VALUES ('ap-1', 'p-a', 'r-1', 'approve', 'user', 'u-lead', 'looks right', 1759000006200);
INSERT INTO project_evidence (id, project_id, work_item_id, execution_id, kind, source,
    external_id, url, trust, metadata, content_hash, verified_by, verified_at,
    creator_principal_type, creator_principal_id, created_at)
VALUES ('v-1', 'p-a', 'w-2', 'e-2', 'pull_request', 'github', '325',
    'https://github.com/Falconiere/comemory/pull/325', 'verified', '{\"checks\":\"green\"}',
    'sha256:bb', 'engine', 1759000007100, 'project_agent', 'g-1', 1759000007000);
INSERT INTO project_evidence_criteria (evidence_id, criterion_id) VALUES ('v-1', 'c-1');
INSERT INTO project_activity_events (id, project_id, actor_principal_type, actor_principal_id,
    event_type, entity_type, entity_id, payload, created_at)
VALUES ('ev-1', 'p-a', 'user', 'u-lead', 'proposal.approved', 'proposal', 'r-1',
    '{\"version\":2}', 1759000008000);
INSERT INTO project_command_receipts (id, project_id, principal_type, principal_id,
    idempotency_key, command_type, request_digest, response, created_at)
VALUES ('rc-1', 'p-a', 'user', 'u-lead', 'key-1', 'proposal.review', 'sha256:cc',
    '{\"status\":200}', 1759000009000);
";

type Rows = Vec<Vec<Value>>;

/// Every row of every project table, by the columns its declaration names,
/// in a stable order.
fn project_rows(home: &std::path::Path) -> Vec<(String, Vec<String>, Rows)> {
    let conn = db(home);
    table_defs()
        .into_iter()
        .map(|def| {
            let columns: Vec<String> = def.columns.iter().map(|c| c.name.clone()).collect();
            let list = columns.join(", ");
            let mut statement = conn
                .prepare(&format!(
                    "SELECT {list} FROM \"{}\" ORDER BY {list}",
                    def.name
                ))
                .expect("prepare");
            let rows = statement
                .query_map([], |r| {
                    (0..columns.len())
                        .map(|i| r.get::<_, Value>(i))
                        .collect::<Result<Vec<_>, _>>()
                })
                .expect("query")
                .collect::<Result<Rows, _>>()
                .expect("collect");
            (def.name, columns, rows)
        })
        .collect()
}

#[test]
fn a_rebuild_keeps_every_project_row_column_for_column() {
    let home = tempdir().expect("home");
    run(
        home.path(),
        &[
            "save",
            "A project's charter and evidence live only in comemory.db.",
            "--kind",
            "decision",
        ],
    );
    db(home.path())
        .execute_batch(SEED)
        .expect("seed every project table");

    let before = project_rows(home.path());
    assert_eq!(before.len(), PROJECT_TABLES.len());
    for (table, columns, rows) in &before {
        assert!(!rows.is_empty(), "{table} was seeded");
        for (i, column) in columns.iter().enumerate() {
            assert!(
                rows.iter()
                    .any(|row| row.get(i).is_some_and(|v| *v != Value::Null)),
                "{table}.{column} is set in at least one row, so its copy is proven"
            );
        }
    }

    run(home.path(), &["rebuild"]);

    let after = project_rows(home.path());
    for ((table, _, rows_before), (_, _, rows_after)) in before.iter().zip(&after) {
        assert_eq!(rows_after, rows_before, "{table} after rebuild");
    }
}
