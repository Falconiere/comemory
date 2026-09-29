#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Coverage for `src/store/project_evidence.rs` against a real migrated
//! `comemory.db` holding the committed plan `tests/fixtures/projects/plan_seed.sql`
//! seeds: rows round-trip column for column, criterion links land, the two
//! membership reads see only the project's own rows, and the keyset page
//! applies every filter together, newest first, walking each row once even
//! when a page edge falls inside a run of one millisecond.

use comemory::store::connection;
use comemory::store::project_evidence::{
    self, EvidencePage, EvidenceRow, NewEvidence, TrustFilter,
};
use comemory::store::projects::{NewProject, insert_project};
use rusqlite::Connection;
use tempfile::tempdir;

/// The project `plan_seed.sql` writes into.
const P: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";
const ITEM: &str = "b0000000-0000-4000-8000-000000000002";
const ARCHIVED_ITEM: &str = "b0000000-0000-4000-8000-000000000003";

fn project(conn: &Connection, id: &str, key: &str) {
    let slug = key.to_lowercase();
    insert_project(
        conn,
        &NewProject {
            id,
            slug: &slug,
            key_prefix: key,
            name: key,
            outcome: "Shipped",
            constraints: "[]",
            non_goals: "[]",
            lead_type: "user",
            lead_id: "local-operator",
            target_date: None,
            creator_type: "user",
            creator_id: "local-operator",
            at_ms: 1,
        },
    )
    .unwrap();
}

/// `(id, project, work item, kind, trust, created_at)`.
type Spec<'a> = (&'a str, &'a str, Option<&'a str>, &'a str, &'a str, i64);

fn evidence(conn: &Connection, (id, project_id, work_item_id, kind, trust, at_ms): Spec<'_>) {
    project_evidence::insert(
        conn,
        &NewEvidence {
            id,
            project_id,
            work_item_id,
            kind,
            source: "ci",
            external_id: Some("42"),
            url: None,
            trust,
            metadata: r#"{"claim":{}}"#,
            creator_type: "project_agent",
            creator_id: "local-agent",
            at_ms,
        },
    )
    .unwrap();
}

fn id(n: u8) -> String {
    format!("e0000000-0000-4000-8000-0000000000{n:02}")
}

/// A store with the plan seed and seven evidence rows on `P` (two sharing
/// one millisecond, one with an unrecognised trust) plus one on `OTHER`.
fn seeded() -> (tempfile::TempDir, Connection) {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    project(&conn, P, "PLAN");
    project(&conn, OTHER, "OTHER");
    conn.execute_batch(include_str!(
        "../../../tests/fixtures/projects/plan_seed.sql"
    ))
    .unwrap();
    let rows = [
        (id(1), P, None, "commit", "pending", 10),
        (id(2), P, Some(ITEM), "test_run", "self_reported", 20),
        (id(3), P, Some(ITEM), "commit", "verified", 30),
        (id(4), P, None, "external_url", "self_reported", 30),
        (id(5), P, Some(ITEM), "commit", "invalid", 40),
        (id(6), P, None, "memory", "bogus", 50),
        (id(7), P, Some(ARCHIVED_ITEM), "commit", "self_reported", 60),
        (id(8), OTHER, None, "commit", "pending", 70),
    ];
    for (eid, project_id, item, kind, trust, at) in &rows {
        evidence(&conn, (eid, project_id, *item, kind, trust, *at));
    }
    (dir, conn)
}

fn page(conn: &Connection, filter: EvidencePage<'_>) -> Vec<String> {
    project_evidence::page(conn, &filter)
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect()
}

fn all(limit: i64) -> EvidencePage<'static> {
    EvidencePage {
        project_id: P,
        kind: None,
        trust: None,
        work_item_id: None,
        before: None,
        limit,
    }
}

#[test]
fn a_row_round_trips_and_its_criteria_link() {
    let (_dir, conn) = seeded();
    let row = project_evidence::one(&conn, P, &id(2)).unwrap().unwrap();
    assert_eq!(
        row,
        EvidenceRow {
            id: id(2),
            work_item_id: Some(ITEM.into()),
            execution_id: None,
            kind: "test_run".into(),
            source: "ci".into(),
            external_id: Some("42".into()),
            url: None,
            trust: "self_reported".into(),
            metadata: r#"{"claim":{}}"#.into(),
            content_hash: None,
            verified_by: None,
            verified_at: None,
            creator_principal_type: "project_agent".into(),
            creator_principal_id: "local-agent".into(),
            created_at: 20,
        }
    );
    assert!(
        project_evidence::one(&conn, OTHER, &id(2))
            .unwrap()
            .is_none()
    );

    let criteria = [
        "c0000000-0000-4000-8000-000000000001".to_string(),
        "c0000000-0000-4000-8000-000000000002".to_string(),
    ];
    project_evidence::link_criteria(&conn, &id(2), &criteria).unwrap();
    let linked: Vec<String> = conn
        .prepare("SELECT criterion_id FROM project_evidence_criteria WHERE evidence_id = ?1 ORDER BY criterion_id")
        .unwrap()
        .query_map([id(2)], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(linked, criteria);
}

#[test]
fn membership_reads_see_only_the_projects_own_rows() {
    let (_dir, conn) = seeded();
    let asked = [
        "c0000000-0000-4000-8000-000000000001".to_string(),
        "c0000000-0000-4000-8000-000000000004".to_string(),
        "c0000000-0000-4000-8000-000000000099".to_string(),
    ];
    let seen = project_evidence::members(&conn, P, Some(ITEM), &asked).unwrap();
    let mut found = seen.criteria.clone();
    found.sort();
    assert_eq!(found, asked[..2]);
    assert!(seen.work_item);
    let elsewhere = project_evidence::members(&conn, OTHER, Some(ITEM), &asked).unwrap();
    assert_eq!(
        elsewhere,
        project_evidence::Members {
            work_item: false,
            criteria: vec![]
        }
    );
    let archived = project_evidence::members(&conn, P, Some(ARCHIVED_ITEM), &[]).unwrap();
    assert!(archived.work_item);
    assert!(
        !project_evidence::members(&conn, P, Some(&id(99)), &[])
            .unwrap()
            .work_item
    );
    assert!(
        !project_evidence::members(&conn, P, None, &[])
            .unwrap()
            .work_item
    );
}

#[test]
fn every_filter_alone_and_combined_selects_the_matching_rows_newest_first() {
    let (_dir, conn) = seeded();
    let known: &[&str] = &["verified", "self_reported", "pending"];
    let cases: Vec<(EvidencePage<'_>, Vec<u8>)> = vec![
        (all(100), vec![7, 6, 5, 4, 3, 2, 1]),
        (
            EvidencePage {
                kind: Some("commit"),
                ..all(100)
            },
            vec![7, 5, 3, 1],
        ),
        (
            EvidencePage {
                trust: Some(TrustFilter::Exactly("self_reported")),
                ..all(100)
            },
            vec![7, 4, 2],
        ),
        (
            EvidencePage {
                trust: Some(TrustFilter::NoneOf(known)),
                ..all(100)
            },
            vec![6, 5],
        ),
        (
            EvidencePage {
                work_item_id: Some(ITEM),
                ..all(100)
            },
            vec![5, 3, 2],
        ),
        (
            EvidencePage {
                kind: Some("commit"),
                trust: Some(TrustFilter::Exactly("self_reported")),
                ..all(100)
            },
            vec![7],
        ),
        (
            EvidencePage {
                kind: Some("commit"),
                work_item_id: Some(ITEM),
                ..all(100)
            },
            vec![5, 3],
        ),
        (
            EvidencePage {
                trust: Some(TrustFilter::Exactly("self_reported")),
                work_item_id: Some(ITEM),
                ..all(100)
            },
            vec![2],
        ),
        (
            EvidencePage {
                kind: Some("commit"),
                trust: Some(TrustFilter::Exactly("verified")),
                work_item_id: Some(ITEM),
                ..all(100)
            },
            vec![3],
        ),
        (
            EvidencePage {
                work_item_id: Some("b0000000-0000-4000-8000-000000000099"),
                ..all(100)
            },
            vec![],
        ),
    ];
    for (filter, expected) in cases {
        let expected: Vec<String> = expected.into_iter().map(id).collect();
        assert_eq!(page(&conn, filter), expected, "{filter:?}");
    }
}

#[test]
fn a_keyset_walk_returns_each_row_once_at_every_page_size() {
    let (_dir, conn) = seeded();
    let every = page(&conn, all(100));
    for size in 1..=4 {
        let mut walked = Vec::new();
        let mut before: Option<(i64, String)> = None;
        loop {
            let filter = EvidencePage {
                before: before.as_ref().map(|(at, id)| (*at, id.as_str())),
                ..all(size)
            };
            let rows = project_evidence::page(&conn, &filter).unwrap();
            let Some(last) = rows.last() else { break };
            before = Some((last.created_at, last.id.clone()));
            walked.extend(rows.into_iter().map(|r| r.id));
        }
        assert_eq!(walked, every, "page size {size}");
    }
}
