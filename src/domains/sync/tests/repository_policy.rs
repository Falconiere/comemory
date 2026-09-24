#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Repository policy resolution against a real indexed Git checkout.

use comemory::config::{Config, Paths};
use comemory::domains::sync::client_policy::{
    ApprovedRepository, RepositoryMapping, SyncPolicyStatus,
};
use comemory::store::connection;

use super::RepositoryPolicy;
use crate::test_common::code_sync_fixture;

fn status(mappings: Vec<RepositoryMapping>) -> SyncPolicyStatus {
    SyncPolicyStatus {
        workspace_id: "ws-org".into(),
        allowlist: vec![ApprovedRepository {
            full_name: "falconiere/comemory".into(),
            name: "comemory".into(),
        }],
        repo_mappings: mappings,
        policy_revision: 7,
        sync_protocol: "repository-policy-v1".into(),
        import_gate: "repository_allowlist".into(),
    }
}

#[test]
fn checkout_remote_and_confirmed_mapping_resolve_to_the_approved_identity() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let repo = code_sync_fixture::write_ts_repo(home.path());
    let mut conn = connection::open(paths.db_path()).expect("db");
    code_sync_fixture::index(&paths, &Config::defaults(), &mut conn, &repo);

    let policy = RepositoryPolicy::resolve(
        &conn,
        status(vec![RepositoryMapping {
            label: "old-comemory".into(),
            full_name: "falconiere/comemory".into(),
        }]),
        "ws-org",
    )
    .expect("policy");

    assert_eq!(
        policy.code_repository("scratch"),
        Some("falconiere/comemory")
    );
    assert_eq!(
        policy.memory_repository("scratch"),
        Some("falconiere/comemory")
    );
    assert_eq!(
        policy.memory_repository("old-comemory"),
        Some("falconiere/comemory")
    );
    assert_eq!(
        policy.memory_repository("Falconiere/Comemory"),
        Some("falconiere/comemory")
    );
    assert_eq!(policy.memory_repository(""), None);
    assert_eq!(policy.memory_repository("comemory"), None);
}

#[test]
fn status_protocol_workspace_and_mapping_are_validated() {
    let conn = rusqlite::Connection::open_in_memory().expect("db");
    let mut wrong_protocol = status(Vec::new());
    wrong_protocol.sync_protocol = "legacy".into();
    assert!(RepositoryPolicy::resolve(&conn, wrong_protocol, "ws-org").is_err());

    let wrong_mapping = status(vec![RepositoryMapping {
        label: "comemory".into(),
        full_name: "someone/else".into(),
    }]);
    assert!(RepositoryPolicy::resolve(&conn, wrong_mapping, "ws-org").is_err());
}

#[test]
fn the_persisted_map_holds_exactly_the_labels_that_resolve() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let repo = code_sync_fixture::write_ts_repo(home.path());
    let mut conn = connection::open(paths.db_path()).expect("db");
    code_sync_fixture::index(&paths, &Config::defaults(), &mut conn, &repo);
    let mut unapproved = status(vec![RepositoryMapping {
        label: "old-comemory".into(),
        full_name: "falconiere/comemory".into(),
    }]);
    // A mapping the allowlist does not cover must not reach the map: a capture
    // reading it would mint an id for a repository this workspace refuses.
    unapproved.repo_mappings.push(RepositoryMapping {
        label: "elsewhere".into(),
        full_name: "falconiere/comemory".into(),
    });
    let policy = RepositoryPolicy::resolve(&conn, unapproved, "ws-org").expect("policy");

    policy
        .persist_resolved_labels(&mut conn)
        .expect("persist the resolved labels");

    // Every label the policy resolves, and nothing else — read back through
    // the offline reader a capture actually uses.
    for label in [
        "falconiere/comemory",
        "old-comemory",
        "scratch",
        "elsewhere",
    ] {
        assert_eq!(
            comemory::store::repository_approval::canonical_for(&conn, label)
                .expect("read")
                .as_deref(),
            policy.memory_repository(label),
            "`{label}` must read back exactly as the policy resolves it"
        );
    }
    assert_eq!(
        comemory::store::repository_approval::canonical_for(&conn, "never-heard-of-it")
            .expect("read"),
        None
    );
}

#[test]
fn a_later_policy_load_replaces_the_map_rather_than_adding_to_it() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let repo = code_sync_fixture::write_ts_repo(home.path());
    let mut conn = connection::open(paths.db_path()).expect("db");
    code_sync_fixture::index(&paths, &Config::defaults(), &mut conn, &repo);
    let first = RepositoryPolicy::resolve(
        &conn,
        status(vec![RepositoryMapping {
            label: "old-comemory".into(),
            full_name: "falconiere/comemory".into(),
        }]),
        "ws-org",
    )
    .expect("first policy");
    first
        .persist_resolved_labels(&mut conn)
        .expect("first load");
    assert_eq!(
        comemory::store::repository_approval::canonical_for(&conn, "old-comemory")
            .expect("read")
            .as_deref(),
        Some("falconiere/comemory"),
        "it resolved a moment ago, so its disappearance below means something"
    );

    // The next load no longer confirms that mapping.
    let second = RepositoryPolicy::resolve(&conn, status(vec![]), "ws-org").expect("second policy");
    second
        .persist_resolved_labels(&mut conn)
        .expect("second load");

    assert_eq!(
        comemory::store::repository_approval::canonical_for(&conn, "old-comemory").expect("read"),
        None,
        "a withdrawn mapping stops resolving, rather than lingering as a stale row"
    );
    assert_eq!(
        comemory::store::repository_approval::canonical_for(&conn, "falconiere/comemory")
            .expect("read")
            .as_deref(),
        Some("falconiere/comemory"),
        "while the repository itself is still approved"
    );
}
