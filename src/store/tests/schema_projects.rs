#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Migrations 0031 and 0032 against a real database opened through
//! `store::connection::open` (`PRAGMA foreign_keys=ON`): every project table
//! and the change feed exist at version 32, the registry is leaf first over every declared key,
//! and the platform's composite keys, cascades, `NO ACTION` links and the
//! self-edge CHECK are enforced by SQLite itself.

use comemory::store::connection;
use comemory::store::migrate::{self, list::MIGRATIONS};
use comemory::store::schema_projects::{PROJECT_TABLES, table_defs};
use rusqlite::Connection;

fn fresh() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("count")
}

/// The refusal SQLite gave `sql`, which must fail.
fn refusal(conn: &Connection, sql: &str) -> String {
    conn.execute_batch(sql)
        .expect_err("SQLite must refuse this write")
        .to_string()
}

/// Two projects: `a` with milestone `ma`, items `a1` (in `ma`) and `a2`;
/// `b` with item `b1`.
fn seed(conn: &Connection) {
    conn.execute_batch(
        "INSERT INTO projects (id, slug, key_prefix, name, outcome,
             lead_principal_type, lead_principal_id,
             creator_principal_type, creator_principal_id)
         VALUES ('a', 'alpha', 'ALP', 'Alpha', 'ship alpha', 'user', 'u1', 'user', 'u1'),
                ('b', 'beta', 'BET', 'Beta', 'ship beta', 'user', 'u1', 'user', 'u1');
         INSERT INTO project_milestones (id, project_id, name, description, target_date)
         VALUES ('ma', 'a', 'M1', 'first', 1767225600000);
         INSERT INTO project_work_items (id, project_id, number, milestone_id, kind, title, description)
         VALUES ('a1', 'a', 1, 'ma', 'task', 'one', 'first'),
                ('a2', 'a', 2, NULL, 'task', 'two', 'second'),
                ('b1', 'b', 1, NULL, 'task', 'one', 'first');",
    )
    .expect("seed");
}

/// Every child of `a1`, plus the same shape under `a2`, so the cascade can be
/// seen to stop at the deleted item.
fn seed_children(conn: &Connection) {
    conn.execute_batch(
        "INSERT INTO project_criteria (id, project_id, work_item_id, description)
         VALUES ('c1', 'a', 'a1', 'a1 done'), ('c2', 'a', 'a2', 'a2 done');
         INSERT INTO project_work_item_dependencies (project_id, blocker_id, blocked_id)
         VALUES ('a', 'a1', 'a2');
         INSERT INTO project_executions (id, project_id, work_item_id, actor_principal_type, actor_principal_id)
         VALUES ('e1', 'a', 'a1', 'project_agent', 'g1'), ('e2', 'a', 'a2', 'project_agent', 'g1');
         INSERT INTO project_work_packets (id, project_id, execution_id, work_item_id, plan_version,
             work_item_version, engine_query_id, requester_principal_type, requester_principal_id)
         VALUES ('p1', 'a', 'e1', 'a1', 1, 1, 'q1', 'project_agent', 'g1'),
                ('p2', 'a', 'e2', 'a2', 1, 1, 'q2', 'project_agent', 'g1');
         INSERT INTO project_evidence (id, project_id, work_item_id, execution_id, kind, source,
             creator_principal_type, creator_principal_id)
         VALUES ('v1', 'a', 'a1', 'e1', 'commit', 'git', 'project_agent', 'g1'),
                ('v2', 'a', 'a2', 'e2', 'commit', 'git', 'project_agent', 'g1');",
    )
    .expect("seed children");
}

#[test]
fn schema_projects_fresh_open_reaches_v32_with_every_table() {
    let (_dir, conn) = fresh();
    let version: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'version'",
            [],
            |r| r.get(0),
        )
        .expect("version");
    assert_eq!(version, "32");
    assert_eq!(migrate::CURRENT_VERSION, "32");
    assert_eq!(
        MIGRATIONS.len(),
        32,
        "CURRENT_VERSION agrees with MIGRATIONS.len()"
    );
    for table in PROJECT_TABLES {
        assert_eq!(
            count(
                &conn,
                &format!(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = '{table}'"
                )
            ),
            1,
            "{table} exists"
        );
    }
    assert_eq!(PROJECT_TABLES.len(), 14);
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'project_changes'"
        ),
        1,
        "the change feed exists"
    );
    assert!(
        !PROJECT_TABLES.contains(&"project_changes"),
        "the change feed stays outside the cascade registry"
    );
}

#[test]
fn schema_projects_registry_is_leaf_first_over_every_declared_key() {
    let defs = table_defs();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, PROJECT_TABLES, "table_defs() follows PROJECT_TABLES");
    let position = |table: &str| {
        PROJECT_TABLES
            .iter()
            .position(|t| *t == table)
            .unwrap_or_else(|| panic!("{table} is not a project table"))
    };
    let mut checked = 0;
    for def in &defs {
        let column_parents = def
            .columns
            .iter()
            .filter_map(|c| c.references.as_deref())
            .map(|r| r.split('(').next().unwrap_or(r).to_string());
        let composite_parents = def
            .foreign_keys
            .iter()
            .map(|fk| fk.references_table.clone());
        for parent in column_parents.chain(composite_parents) {
            checked += 1;
            if parent == def.name {
                continue;
            }
            assert!(
                position(&def.name) < position(&parent),
                "{} references {parent}, so it must come first",
                def.name
            );
        }
    }
    let composite: usize = defs.iter().map(|d| d.foreign_keys.len()).sum();
    assert_eq!(
        composite, 10,
        "the platform's ten composite keys are declared"
    );
    assert!(checked > composite, "column-level keys were checked too");
}

#[test]
fn schema_projects_composite_key_refuses_a_cross_project_pair() {
    let (_dir, conn) = fresh();
    seed(&conn);
    let error = refusal(
        &conn,
        "INSERT INTO project_criteria (id, project_id, work_item_id, description)
         VALUES ('x', 'b', 'a1', 'names project a''s item from project b');",
    );
    assert!(error.contains("FOREIGN KEY constraint failed"), "{error}");
    let dangling = refusal(
        &conn,
        "INSERT INTO project_evidence (id, project_id, work_item_id, kind, source,
             creator_principal_type, creator_principal_id)
         VALUES ('x', 'a', 'a9', 'commit', 'git', 'user', 'u1');",
    );
    assert!(
        dangling.contains("FOREIGN KEY constraint failed"),
        "{dangling}"
    );
    conn.execute_batch(
        "INSERT INTO project_criteria (id, project_id, work_item_id, description)
         VALUES ('pl', 'b', NULL, 'a project-level criterion skips the key');",
    )
    .expect("a NULL member skips the composite key");
}

#[test]
fn schema_projects_deleting_a_work_item_cascades_to_its_children_only() {
    let (_dir, conn) = fresh();
    seed(&conn);
    seed_children(&conn);
    conn.execute_batch("DELETE FROM project_work_items WHERE id = 'a1';")
        .expect("delete a1");
    for (table, expected) in [
        ("project_criteria", 1),
        ("project_work_item_dependencies", 0),
        ("project_executions", 1),
        ("project_work_packets", 1),
        ("project_evidence", 1),
    ] {
        assert_eq!(
            count(&conn, &format!("SELECT count(*) FROM {table}")),
            expected,
            "{table} after deleting a1"
        );
        assert_eq!(
            count(
                &conn,
                &format!(
                    "SELECT count(*) FROM {table} WHERE {}",
                    if table == "project_work_item_dependencies" {
                        "blocker_id = 'a1' OR blocked_id = 'a1'"
                    } else {
                        "work_item_id = 'a1'"
                    }
                )
            ),
            0,
            "nothing in {table} still names a1"
        );
    }
}

#[test]
fn schema_projects_check_refuses_a_self_edge() {
    let (_dir, conn) = fresh();
    seed(&conn);
    let error = refusal(
        &conn,
        "INSERT INTO project_work_item_dependencies (project_id, blocker_id, blocked_id)
         VALUES ('a', 'a1', 'a1');",
    );
    assert!(error.contains("CHECK constraint failed"), "{error}");
}

#[test]
fn schema_projects_no_action_links_refuse_deleting_a_named_parent() {
    let (_dir, conn) = fresh();
    seed(&conn);
    let milestone = refusal(&conn, "DELETE FROM project_milestones WHERE id = 'ma';");
    assert!(
        milestone.contains("FOREIGN KEY constraint failed"),
        "{milestone}"
    );
    conn.execute_batch("UPDATE project_work_items SET parent_work_item_id = 'a1' WHERE id = 'a2';")
        .expect("a same-project parent");
    let parent = refusal(&conn, "DELETE FROM project_work_items WHERE id = 'a1';");
    assert!(parent.contains("FOREIGN KEY constraint failed"), "{parent}");
    let cross_parent = refusal(
        &conn,
        "UPDATE project_work_items SET parent_work_item_id = 'a1' WHERE id = 'b1';",
    );
    assert!(
        cross_parent.contains("FOREIGN KEY constraint failed"),
        "{cross_parent}"
    );
}
