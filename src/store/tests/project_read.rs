#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Coverage for `src/store/project_read.rs` against a real migrated
//! `comemory.db`: the stored row round-trips with its defaults, and the
//! keyset page is total over `(created_at, id)` — rows sharing one
//! millisecond page by id, filters narrow, archived rows stay out unless
//! asked for.

use comemory::store::connection;
use comemory::store::project_read::{ProjectPage, project, project_page};
use comemory::store::projects::{NewProject, insert_project};
use tempfile::tempdir;

/// Insert a charter with `id`, key prefix `key` and `created_at` `at_ms`.
fn seed(conn: &rusqlite::Connection, id: &str, key: &str, at_ms: i64) {
    let slug = key.to_lowercase();
    insert_project(
        conn,
        &NewProject {
            id,
            slug: &slug,
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
            at_ms,
        },
    )
    .unwrap();
}

fn id(n: u32) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}

#[test]
fn the_row_round_trips_with_its_declared_defaults() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    seed(&conn, &id(1), "AA", 5);
    let row = project(&conn, &id(1)).unwrap().unwrap();
    assert_eq!(
        (
            row.status.as_str(),
            row.health.as_str(),
            row.completion_policy.as_str()
        ),
        ("draft", "unknown", "manual")
    );
    assert_eq!((row.version, row.current_plan_version), (1, 0));
    assert_eq!(
        (row.created_at, row.updated_at, row.archived_at),
        (5, 5, None)
    );
    assert_eq!(row.lead_principal_id, "lead");
    assert_eq!(project(&conn, &id(2)).unwrap(), None);
}

#[test]
fn a_walk_is_total_over_created_at_and_id() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    // Five rows in one millisecond, two older, one newer.
    for n in 1..=5 {
        seed(&conn, &id(n), &format!("T{n}"), 100);
    }
    seed(&conn, &id(6), "OLDA", 50);
    seed(&conn, &id(7), "OLDB", 50);
    seed(&conn, &id(8), "NEW", 200);
    let mut seen = Vec::new();
    let mut after: Option<(i64, String)> = None;
    loop {
        let page = project_page(
            &conn,
            &ProjectPage {
                after: after.as_ref().map(|(at, id)| (*at, id.as_str())),
                limit: 3,
                ..ProjectPage::default()
            },
        )
        .unwrap();
        if page.is_empty() {
            break;
        }
        let last = page.last().unwrap();
        after = Some((last.created_at, last.id.clone()));
        seen.extend(page.into_iter().map(|row| row.id));
    }
    let expected: Vec<String> = [8, 5, 4, 3, 2, 1, 7, 6].into_iter().map(id).collect();
    assert_eq!(seen, expected);
}

#[test]
fn filters_narrow_and_archived_rows_stay_out_unless_asked() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    seed(&conn, &id(1), "AA", 1);
    seed(&conn, &id(2), "BB", 2);
    seed(&conn, &id(3), "CC", 3);
    conn.execute_batch(&format!(
        "UPDATE projects SET status = 'active' WHERE id = '{a}';
         UPDATE projects SET health = 'at_risk' WHERE id = '{b}';
         UPDATE projects SET archived_at = 9 WHERE id = '{c}';",
        a = id(1),
        b = id(2),
        c = id(3)
    ))
    .unwrap();
    let ids = |page: ProjectPage<'_>| -> Vec<String> {
        project_page(&conn, &ProjectPage { limit: 10, ..page })
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
    };
    assert_eq!(ids(ProjectPage::default()), vec![id(2), id(1)]);
    assert_eq!(
        ids(ProjectPage {
            include_archived: true,
            ..ProjectPage::default()
        }),
        vec![id(3), id(2), id(1)]
    );
    assert_eq!(
        ids(ProjectPage {
            status: Some("active"),
            ..ProjectPage::default()
        }),
        vec![id(1)]
    );
    assert_eq!(
        ids(ProjectPage {
            health: Some("at_risk"),
            ..ProjectPage::default()
        }),
        vec![id(2)]
    );
}
