#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Opening a pass against a REAL engine on a loopback socket: an unmanaged
//! engine selects `replica-v1` under the key's persisted snapshot (or approves
//! nothing without one), a refused credential suspends, and a backoff gates an
//! unattended pass before any request.

use std::collections::BTreeMap;

use crate::config::Config;
use crate::domains::sync::AuthFile;
use crate::domains::sync::drain::negotiate::Protocol;
use crate::domains::sync::drain::session::{self, Mode, Opened};
use crate::domains::sync::drain::test_support::LiveEngine;
use crate::domains::sync::replica::test_support::Home;
use crate::store::sync_exchange::{self, ExchangeKey, ExchangeRow};
use crate::store::sync_policy_snapshot::{self, PolicySnapshot};

const REPO: &str = "falconiere/comemory";

fn auth(api_url: &str, secret: &str) -> AuthFile {
    AuthFile {
        version: 2,
        secret: secret.to_string(),
        key_prefix: "cmk_test".to_string(),
        api_url: api_url.to_string(),
        organization_id: "org".to_string(),
        organization_slug: "org".to_string(),
        organization_name: "Org".to_string(),
        workspace_id: "ws_session".to_string(),
        email: None,
    }
}

fn approve(home: &Home, api_url: &str) {
    sync_policy_snapshot::save(
        &home.conn,
        &ExchangeKey::new(api_url, "ws_session"),
        &PolicySnapshot {
            revision: 1,
            fingerprint: "fp".into(),
            allowlist: vec![REPO.into()],
            mappings: BTreeMap::new(),
            loaded_at: "t".into(),
        },
    )
    .expect("snapshot");
}

fn ready(opened: Opened) -> Box<session::Session> {
    match opened {
        Opened::Ready(session) => session,
        Opened::Skipped(row) => panic!("skipped: {row:?}"),
    }
}

#[test]
fn an_unmanaged_engine_selects_replica_under_the_keys_snapshot() {
    let engine = LiveEngine::start();
    let mut home = Home::new();
    approve(&home, &engine.api_url);
    let auth = auth(&engine.api_url, &engine.token());

    let first = ready(
        session::open(&mut home.conn, &Config::defaults(), &auth, Mode::Manual).expect("open"),
    );
    assert_eq!(first.protocol, Protocol::Replica);
    assert!(first.upgraded, "the first selection is the upgrade");
    assert!(
        !first.replica.is_managed(),
        "a bare engine has no policy authority"
    );
    assert_eq!(first.policy.memory_repository(REPO), Some(REPO));
    assert!(first.manifest.is_some());
    sync_exchange::save(&home.conn, &first.row, "t").expect("persist the pass");

    let second = ready(
        session::open(&mut home.conn, &Config::defaults(), &auth, Mode::Manual).expect("again"),
    );
    assert!(
        !second.upgraded,
        "selection is persisted, the upgrade happens once"
    );
}

#[test]
fn without_a_snapshot_an_unmanaged_session_approves_nothing() {
    let engine = LiveEngine::start();
    let mut home = Home::new();
    let auth = auth(&engine.api_url, &engine.token());
    let session = ready(
        session::open(&mut home.conn, &Config::defaults(), &auth, Mode::Manual).expect("open"),
    );
    assert_eq!(session.policy.memory_repository(REPO), None);
}

#[test]
fn a_refused_credential_suspends_and_the_next_pass_makes_no_request() {
    let engine = LiveEngine::start();
    let mut home = Home::new();
    let auth = auth(&engine.api_url, "a-revoked-key");

    let Opened::Skipped(row) =
        session::open(&mut home.conn, &Config::defaults(), &auth, Mode::Unattended).expect("open")
    else {
        panic!("a refused credential never reaches the drain");
    };
    assert_eq!(row.network_state, "auth_suspended");
    assert_eq!(row.protocol, None, "and never selects anything");
    let saved = sync_exchange::load(&home.conn, &ExchangeKey::new(&engine.api_url, "ws_session"))
        .expect("load")
        .expect("row");
    assert_eq!(saved.network_state, "auth_suspended");
    assert!(matches!(
        session::open(&mut home.conn, &Config::defaults(), &auth, Mode::Manual).expect("again"),
        Opened::Skipped(_)
    ));
}

#[test]
fn a_backoff_gates_an_unattended_pass_but_not_a_manual_one() {
    let engine = LiveEngine::start();
    let mut home = Home::new();
    approve(&home, &engine.api_url);
    let key = ExchangeKey::new(&engine.api_url, "ws_session");
    let mut row = ExchangeRow::fresh(&key);
    row.network_state = "backoff".into();
    row.retry_at = Some("2999-01-01T00:00:00Z".into());
    row.last_error = Some("HTTP 502".into());
    sync_exchange::save(&home.conn, &row, "t").expect("save");
    let auth = auth(&engine.api_url, &engine.token());

    assert!(matches!(
        session::open(&mut home.conn, &Config::defaults(), &auth, Mode::Unattended).expect("hook"),
        Opened::Skipped(_)
    ));
    assert!(matches!(
        session::open(&mut home.conn, &Config::defaults(), &auth, Mode::Manual).expect("manual"),
        Opened::Ready(_)
    ));
}
