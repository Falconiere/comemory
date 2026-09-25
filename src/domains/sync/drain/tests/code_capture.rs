#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Push-time code capture over a real indexed git checkout whose `origin`
//! names the canonical repository while its label is `scratch`:
//! a moved index is queued once as a staged generation keyed by the canonical
//! repository, an unchanged one is not, and nothing is captured without an
//! approved identity.

use std::collections::BTreeMap;

use crate::config::{Config, Paths};
use crate::domains::sync::drain::code_capture;
use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::store::code_generation::{self, State};
use crate::store::connection;
use crate::store::replica_outbox::{self, Scope};
use crate::store::sync_exchange::ExchangeKey;
use crate::store::sync_policy_snapshot::{self, PolicySnapshot};
use crate::test_common::code_sync_fixture;

const REPO: &str = "falconiere/comemory";

fn policy(conn: &rusqlite::Connection, repos: &[&str]) -> RepositoryPolicy {
    let key = ExchangeKey::new("http://127.0.0.1:9/api", "ws");
    sync_policy_snapshot::save(
        conn,
        &key,
        &PolicySnapshot {
            revision: 1,
            fingerprint: "fp".into(),
            allowlist: repos.iter().map(|r| (*r).to_string()).collect(),
            mappings: BTreeMap::new(),
            loaded_at: "t".into(),
        },
    )
    .expect("snapshot");
    RepositoryPolicy::from_snapshot(conn, &key).expect("policy")
}

#[test]
fn a_moved_index_is_queued_once_under_its_canonical_name() {
    let home = tempfile::tempdir().expect("home");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    let tree = code_sync_fixture::write_ts_repo(home.path());
    code_sync_fixture::index(&paths, &Config::defaults(), &mut conn, &tree);
    // Indexed under the label `scratch`; its origin names the canonical repo.
    let approved = policy(&conn, &[REPO]);

    let first = code_capture::capture(&mut conn, &Config::defaults(), &approved).expect("capture");
    let again = code_capture::capture(&mut conn, &Config::defaults(), &approved).expect("again");

    assert_eq!(first, 1);
    assert_eq!(again, 0, "one generation in flight per repository");
    let queued =
        replica_outbox::read(&conn, Scope::Entity("code_generation", REPO), 5).expect("read");
    assert_eq!(queued.len(), 1);
    let staged = code_generation::all(&conn, REPO).expect("all");
    assert_eq!(staged.len(), 1);
    assert_eq!(
        staged[0].state,
        State::Staged,
        "active only once the upstream accepts it"
    );
}

#[test]
fn nothing_is_captured_without_an_approved_identity_or_with_code_index_off() {
    let home = tempfile::tempdir().expect("home");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    let tree = code_sync_fixture::write_ts_repo(home.path());
    code_sync_fixture::index(&paths, &Config::defaults(), &mut conn, &tree);

    let unapproved = policy(&conn, &["acme/other"]);
    assert_eq!(
        code_capture::capture(&mut conn, &Config::defaults(), &unapproved).expect("x"),
        0
    );
    let mut off = Config::defaults();
    off.sync.code_index = false;
    let approved = policy(&conn, &[REPO]);
    assert_eq!(
        code_capture::capture(&mut conn, &off, &approved).expect("off"),
        0
    );
}

/// Regression: a generation's id is minted over its parent, so an unchanged
/// index planned on top of the accepted generation got a new id and was
/// queued again on every pass. Contents decide, not ids.
#[test]
fn an_accepted_generation_is_not_recaptured_until_the_index_moves() {
    use crate::store::replica_outbox::Outcome;

    let home = tempfile::tempdir().expect("home");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).expect("db");
    let tree = code_sync_fixture::write_ts_repo(home.path());
    code_sync_fixture::index(&paths, &cfg, &mut conn, &tree);
    let approved = policy(&conn, &[REPO]);
    assert_eq!(
        code_capture::capture(&mut conn, &cfg, &approved).expect("capture"),
        1
    );
    let op = replica_outbox::read(&conn, Scope::Entity("code_generation", REPO), 1)
        .expect("read")
        .remove(0);
    let accepted = Outcome::Accepted {
        sequence: Some(1),
        disposition: "accepted",
        epoch: Some("e"),
    };
    replica_outbox::record(&conn, &op.operation_id, accepted, "t").expect("settle");
    let digest = op.payload_digest.expect("a generation carries its payload");
    code_capture::activate(&conn, REPO, &digest, "t").expect("activate");

    let unchanged = code_capture::capture(&mut conn, &cfg, &approved).expect("unchanged");
    assert_eq!(unchanged, 0, "the same contents are not a new generation");

    code_sync_fixture::commit_hookless(
        &tree,
        &[(
            "src/a.ts",
            "export function alpha(): number {\n  return 9;\n}\n",
        )],
        "move the index",
    );
    code_sync_fixture::index(&paths, &cfg, &mut conn, &tree);
    let moved = code_capture::capture(&mut conn, &cfg, &approved).expect("moved");
    assert_eq!(moved, 1, "a moved index is the next generation");
}
