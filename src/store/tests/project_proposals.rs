#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Coverage for `src/store/project_proposals.rs` against a real migrated
//! `comemory.db`: a written proposal reads back column for column, only
//! under its own project; the page walks newest first with `id` breaking a
//! same-millisecond tie, filters by state and carries a seeded review; and
//! `start_planning` moves only a `draft` project, once.

use comemory::store::connection;
use comemory::store::project_proposals::{
    NewProposal, ProposalPage, insert, proposal, proposal_page, start_planning,
};
use comemory::store::projects::{NewProject, insert_project};
use rusqlite::Connection;
use tempfile::tempdir;

const PROJECT: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";

fn charter(conn: &Connection, id: &str, key: &str) {
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

/// A database with [`PROJECT`] and [`OTHER`] chartered.
fn home() -> (tempfile::TempDir, Connection) {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    charter(&conn, PROJECT, "ONE");
    charter(&conn, OTHER, "TWO");
    (dir, conn)
}

/// Proposal `e..0<n>` of `project` written at `at_ms`.
fn write(conn: &Connection, project: &str, n: u8, at_ms: i64) -> String {
    let id = format!("e0000000-0000-4000-8000-00000000000{n}");
    insert(
        conn,
        &NewProposal {
            id: &id,
            project_id: project,
            base_plan_version: 0,
            operations: r#"[{"op":"criterion.archive","criterionId":"c0000000-0000-4000-8000-000000000001"}]"#,
            assumptions: r#"["a"]"#,
            risks: "[]",
            rationale: "why",
            proposer_type: "project_agent",
            proposer_id: "local-agent",
            request_digest: "digest",
            at_ms,
        },
    )
    .unwrap();
    id
}

fn page<'a>(state: Option<&'a str>, after: Option<(i64, &'a str)>, limit: i64) -> ProposalPage<'a> {
    ProposalPage {
        project_id: PROJECT,
        state,
        after,
        limit,
    }
}

#[test]
fn a_written_proposal_reads_back_only_under_its_own_project() {
    let (_dir, conn) = home();
    let id = write(&conn, PROJECT, 1, 42);
    let row = proposal(&conn, PROJECT, &id).unwrap().unwrap();
    assert_eq!(
        (
            row.state.as_str(),
            row.base_plan_version,
            row.created_at,
            row.updated_at
        ),
        ("pending", 0, 42, 42)
    );
    assert_eq!(
        (
            row.assumptions.as_str(),
            row.risks.as_str(),
            row.rationale.as_str()
        ),
        (r#"["a"]"#, "[]", "why")
    );
    assert_eq!(
        (
            row.proposer_principal_type.as_str(),
            row.proposer_principal_id.as_str()
        ),
        ("project_agent", "local-agent")
    );
    assert!(row.review.is_none());
    assert!(proposal(&conn, OTHER, &id).unwrap().is_none());
}

#[test]
fn the_page_walks_newest_first_breaks_ties_by_id_and_filters_by_state() {
    let (_dir, conn) = home();
    let first = write(&conn, PROJECT, 1, 10);
    let tied_low = write(&conn, PROJECT, 2, 20);
    let tied_high = write(&conn, PROJECT, 3, 20);
    write(&conn, OTHER, 4, 30);
    conn.execute(
        "UPDATE project_plan_proposals SET state = 'rejected' WHERE id = ?1",
        [&tied_low],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO project_approvals (id, project_id, proposal_id, decision, \
         reviewer_principal_type, reviewer_principal_id, rationale, created_at) \
         VALUES ('f0000000-0000-4000-8000-000000000001', ?1, ?2, 'reject', 'user', \
         'local-operator', 'no', 25)",
        [PROJECT, tied_low.as_str()],
    )
    .unwrap();

    let all = proposal_page(&conn, &page(None, None, 10)).unwrap();
    let ids: Vec<&str> = all.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, [tied_high.as_str(), tied_low.as_str(), first.as_str()]);
    let review = all[1].review.as_ref().unwrap();
    assert_eq!(
        (
            review.decision.as_str(),
            review.reviewer_principal_id.as_str()
        ),
        ("reject", "local-operator")
    );
    assert_eq!(
        (review.rationale.as_deref(), review.created_at),
        (Some("no"), 25)
    );

    let after_high = proposal_page(&conn, &page(None, Some((20, &tied_high)), 1)).unwrap();
    assert_eq!(after_high[0].id, tied_low);
    let pending = proposal_page(&conn, &page(Some("pending"), None, 10)).unwrap();
    let ids: Vec<&str> = pending.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, [tied_high.as_str(), first.as_str()]);
    let rejected = proposal_page(&conn, &page(Some("rejected"), None, 10)).unwrap();
    assert_eq!(rejected.len(), 1);
}

#[test]
fn start_planning_moves_only_a_draft_project_once() {
    let (_dir, conn) = home();
    let read = |conn: &Connection| -> (String, i64, i64) {
        conn.query_row(
            "SELECT status, version, updated_at FROM projects WHERE id = ?1",
            [PROJECT],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    };
    let (_, version, _) = read(&conn);
    assert!(start_planning(&conn, PROJECT, 99).unwrap());
    assert_eq!(read(&conn), ("planning".to_string(), version + 1, 99));
    assert!(!start_planning(&conn, PROJECT, 100).unwrap());
    assert_eq!(read(&conn), ("planning".to_string(), version + 1, 99));
}
