#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Coverage for `src/store/projects.rs` against a real migrated
//! `comemory.db`: each unique index a charter can hit is reported as its own
//! outcome, repository and criterion rows land beside the project, and a
//! lifecycle patch lands only at the version it names.

use comemory::store::connection;
use comemory::store::project_read;
use comemory::store::projects::{
    LifecyclePatch, NewProject, ProjectInsert, insert_project, insert_relations, update_lifecycle,
};
use tempfile::tempdir;

fn charter<'a>(id: &'a str, slug: &'a str, key_prefix: &'a str) -> NewProject<'a> {
    NewProject {
        id,
        slug,
        key_prefix,
        name: "Ship it",
        outcome: "Shipped",
        constraints: r#"["offline"]"#,
        non_goals: "[]",
        lead_type: "user",
        lead_id: "local-operator",
        target_date: Some(1_727_481_600_000),
        creator_type: "user",
        creator_id: "local-operator",
        at_ms: 1_727_481_600_123,
    }
}

const A: &str = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2a";
const B: &str = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2b";

#[test]
fn each_unique_index_is_its_own_outcome() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    assert_eq!(
        insert_project(&conn, &charter(A, "ship-it", "SHIP")).unwrap(),
        ProjectInsert::Inserted
    );
    assert_eq!(
        insert_project(&conn, &charter(B, "ship-it", "OTHER")).unwrap(),
        ProjectInsert::SlugTaken
    );
    assert_eq!(
        insert_project(&conn, &charter(B, "ship-it-2", "SHIP")).unwrap(),
        ProjectInsert::KeyPrefixTaken
    );
    assert_eq!(
        insert_project(&conn, &charter(A, "fresh", "FRESH")).unwrap(),
        ProjectInsert::IdTaken
    );
    // Both slug and key prefix taken: SQLite names the key prefix, so the
    // create core refuses the duplicate without retrying a slug.
    assert_eq!(
        insert_project(&conn, &charter(B, "ship-it", "SHIP")).unwrap(),
        ProjectInsert::KeyPrefixTaken
    );
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn a_non_unique_failure_propagates() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER refuse BEFORE INSERT ON projects BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();
    let e = insert_project(&conn, &charter(A, "ship-it", "SHIP")).unwrap_err();
    assert!(e.to_string().contains("refused"), "{e}");
}

#[test]
fn repositories_and_criteria_are_written_beside_the_project() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    let project = charter(A, "ship-it", "SHIP");
    insert_project(&conn, &project).unwrap();
    insert_relations(
        &conn,
        &project,
        &["zeta/app".into(), "acme/api".into()],
        &[
            ("c1".into(), "Tests pass".into()),
            ("c2".into(), "Docs ship".into()),
        ],
    )
    .unwrap();
    let relations = project_read::relations(&conn, &[A.to_string()]).unwrap();
    assert_eq!(
        relations.repositories,
        vec![(A.into(), "acme/api".into()), (A.into(), "zeta/app".into())]
    );
    let criteria = relations.criteria;
    assert_eq!(criteria.len(), 2);
    assert_eq!(
        (criteria[0].position, criteria[0].description.as_str()),
        (0, "Tests pass")
    );
    assert_eq!(
        (criteria[1].position, criteria[1].description.as_str()),
        (1, "Docs ship")
    );
    assert!(criteria[0].required);
    assert_eq!(criteria[0].evidence_requirement, "reported");
    assert_eq!(criteria[0].resolution, "open");
    assert_eq!(criteria[0].resolution_rationale, None);
}

#[test]
fn a_lifecycle_patch_lands_only_at_the_version_it_names() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    insert_project(&conn, &charter(A, "ship-it", "SHIP")).unwrap();
    let pause = LifecyclePatch {
        status: Some("paused"),
        archived_at: None,
        at_ms: 1_727_481_700_000,
    };
    assert!(update_lifecycle(&conn, A, 1, &pause).unwrap());
    let row = project_read::project(&conn, A).unwrap().unwrap();
    assert_eq!(
        (
            row.status.as_str(),
            row.version,
            row.updated_at,
            row.archived_at
        ),
        ("paused", 2, 1_727_481_700_000, None)
    );

    // A writer still holding version 1 lost the race: nothing changes.
    let stale = LifecyclePatch {
        status: Some("active"),
        archived_at: Some(Some(1)),
        at_ms: 1_727_481_800_000,
    };
    assert!(!update_lifecycle(&conn, A, 1, &stale).unwrap());
    assert_eq!(project_read::project(&conn, A).unwrap().unwrap(), row);

    let archive = LifecyclePatch {
        status: None,
        archived_at: Some(Some(1_727_481_900_000)),
        at_ms: 1_727_481_900_000,
    };
    assert!(update_lifecycle(&conn, A, 2, &archive).unwrap());
    let archived = project_read::project(&conn, A).unwrap().unwrap();
    assert_eq!(
        (
            archived.status.as_str(),
            archived.version,
            archived.archived_at
        ),
        ("paused", 3, Some(1_727_481_900_000))
    );

    let restore = LifecyclePatch {
        status: None,
        archived_at: Some(None),
        at_ms: 1_727_482_000_000,
    };
    assert!(update_lifecycle(&conn, A, 3, &restore).unwrap());
    let restored = project_read::project(&conn, A).unwrap().unwrap();
    assert_eq!((restored.version, restored.archived_at), (4, None));

    // An unknown id matches nothing either.
    assert!(!update_lifecycle(&conn, B, 1, &restore).unwrap());
}
