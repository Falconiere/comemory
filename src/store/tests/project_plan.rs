#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Coverage for `src/store/project_plan.rs` against a real migrated
//! `comemory.db` seeded with `tests/fixtures/projects/plan_seed.sql`: every
//! plan row comes back, archived ones included, in the platform's display
//! order, and a project with no plan reads four empty collections.

use comemory::store::connection;
use comemory::store::project_plan::plan_rows;
use comemory::store::projects::{NewProject, insert_project};
use tempfile::tempdir;

/// The project `plan_seed.sql` writes its plan into.
const PROJECT: &str = "11111111-1111-4111-8111-111111111111";

/// A charter row with `id` and key prefix `key`.
fn charter(conn: &rusqlite::Connection, id: &str, key: &str) {
    insert_project(
        conn,
        &NewProject {
            id,
            slug: &key.to_lowercase(),
            key_prefix: key,
            name: key,
            outcome: "o",
            constraints: "[]",
            non_goals: "[]",
            lead_type: "user",
            lead_id: "lead",
            target_date: None,
            creator_type: "user",
            creator_id: "local-operator",
            at_ms: 5,
        },
    )
    .unwrap();
}

/// The last UUID group of every id, in order: `a..01` reads `01`.
fn tails<'a>(ids: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    ids.map(|id| &id[id.len() - 2..]).collect()
}

/// A fresh database holding [`PROJECT`] and the plan `plan_seed.sql` seeds.
fn seeded() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    charter(&conn, PROJECT, "PLAN");
    conn.execute_batch(include_str!(
        "../../../tests/fixtures/projects/plan_seed.sql"
    ))
    .unwrap();
    (dir, conn)
}

#[test]
fn every_plan_row_comes_back_in_display_order_archived_included() {
    let (_dir, conn) = seeded();

    let rows = plan_rows(&conn, PROJECT).unwrap();

    // Milestones by (position, id): the two at position 0 tie-break on id.
    let milestones = tails(rows.milestones.iter().map(|m| m.id.as_str()));
    assert_eq!(milestones, ["01", "02", "03"]);
    let alpha = &rows.milestones[0];
    assert_eq!(
        (alpha.name.as_str(), alpha.target_date, alpha.archived_at),
        ("Alpha", 1_790_812_800_000, None)
    );
    assert_eq!(rows.milestones[2].archived_at, Some(1_790_000_000_000));

    // Work items by (position, number).
    let items = tails(rows.work_items.iter().map(|w| w.id.as_str()));
    assert_eq!(items, ["02", "03", "04", "01"]);
    let nested = &rows.work_items[0];
    assert_eq!(nested.number, 2);
    assert_eq!(
        nested.parent_work_item_id.as_deref(),
        Some("b0000000-0000-4000-8000-000000000001")
    );
    assert_eq!(
        nested.milestone_id.as_deref(),
        Some("a0000000-0000-4000-8000-000000000001")
    );
    assert_eq!(
        (
            nested.status.as_str(),
            nested.priority.as_str(),
            nested.estimate,
            nested.version
        ),
        ("ready", "high", Some(3), 2)
    );
    assert_eq!(
        (
            nested.assignee_principal_type.as_deref(),
            nested.assignee_principal_id.as_deref(),
            nested.repo.as_deref()
        ),
        (
            Some("user"),
            Some("local-operator"),
            Some("falconiere/comemory")
        )
    );
    let bare = &rows.work_items[2];
    assert_eq!(
        (
            bare.kind.as_str(),
            bare.milestone_id.as_deref(),
            bare.estimate
        ),
        ("bug", None, None)
    );
    assert_eq!(rows.work_items[1].archived_at, Some(1_790_000_000_000));

    // Criteria by (position, id), both levels and the archived one.
    let criteria = tails(rows.criteria.iter().map(|c| c.criterion.id.as_str()));
    assert_eq!(criteria, ["01", "03", "02"]);
    assert_eq!(rows.criteria[0].work_item_id, None);
    let item_level = &rows.criteria[2];
    assert_eq!(
        item_level.work_item_id.as_deref(),
        Some("b0000000-0000-4000-8000-000000000002")
    );
    assert!(!item_level.criterion.required);
    assert_eq!(
        item_level.criterion.resolution_rationale.as_deref(),
        Some("Covered elsewhere")
    );
    assert_eq!(rows.criteria[1].archived_at, Some(1_790_000_000_000));

    // Every edge, by (blocker, blocked), whatever order it was written in.
    let edges: Vec<(&str, &str)> = rows
        .dependencies
        .iter()
        .map(|d| (&d.blocker_id[34..], &d.blocked_id[34..]))
        .collect();
    assert_eq!(
        edges,
        [("01", "02"), ("01", "03"), ("03", "04"), ("04", "02")]
    );
}

#[test]
fn a_project_without_a_plan_reads_empty_and_never_sees_another_projects_rows() {
    let (_dir, conn) = seeded();
    let other = "22222222-2222-4222-8222-222222222222";
    charter(&conn, other, "OTHER");

    let rows = plan_rows(&conn, other).unwrap();
    assert!(rows.milestones.is_empty());
    assert!(rows.work_items.is_empty());
    assert!(rows.criteria.is_empty());
    assert!(rows.dependencies.is_empty());
}
