#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The `exchange` status block over a real store and a REAL engine:
//! `caught_up` holds only when the cursor equals the head the last pass saw,
//! nothing eligible is owed, nothing stalled and the network is `ok`.

use crate::domains::sync::AuthFile;
use crate::domains::sync::drain::session::{Legs, Mode};
use crate::domains::sync::drain::status::status;
use crate::domains::sync::drain::test_support::{LiveEngine, client_of};
use crate::domains::sync::drain::{self};
use crate::domains::sync::replica::test_support::BODY;
use crate::store::sync_exchange::{self, ExchangeKey};

#[test]
fn caught_up_holds_only_while_nothing_is_owed_either_way() {
    let engine = LiveEngine::start();
    let mut home = client_of(&engine);
    let auth = AuthFile::load(&home.paths).expect("load").expect("auth");
    let (paths, cfg) = (home.paths.clone(), home.cfg.clone());

    let before = status(&home.conn, &auth).expect("status");
    assert_eq!(
        (before.protocol.as_deref(), before.caught_up),
        (None, false)
    );

    home.save(BODY, &["sync"]);
    let owed = status(&home.conn, &auth).expect("status");
    assert_eq!(owed.outbox.pending, 1);
    assert!(!owed.caught_up);

    drain::drain(
        &paths,
        &cfg,
        &mut home.conn,
        &auth,
        (Mode::Manual, Legs::Both),
    )
    .expect("drain");
    let drained = status(&home.conn, &auth).expect("status");
    assert_eq!(drained.protocol.as_deref(), Some("replica-v1"));
    // The pass saw an empty upstream at open and pushed its save to position
    // 1; the cursor equals the head it saw, and the next pull settles the
    // save as this client's own.
    assert_eq!(drained.upstream_head, Some(drained.applied_sequence));
    assert!(drained.caught_up, "{drained:?}");

    // A stall, or a network that is not ok, is never caught up.
    let key = ExchangeKey::new(&engine.api_url, "ws");
    let mut row = sync_exchange::load(&home.conn, &key)
        .expect("load")
        .expect("row");
    row.network_state = "backoff".into();
    sync_exchange::save(&home.conn, &row, "t").expect("save");
    assert!(!status(&home.conn, &auth).expect("status").caught_up);
    row.network_state = "ok".into();
    row.stall_sequence = Some(2);
    sync_exchange::save(&home.conn, &row, "t").expect("save");
    assert!(!status(&home.conn, &auth).expect("status").caught_up);

    // A legacy key has no replica cursor to be caught up with.
    row.stall_sequence = None;
    row.protocol = Some("legacy".into());
    sync_exchange::save(&home.conn, &row, "t").expect("save");
    let legacy = status(&home.conn, &auth).expect("status");
    assert_eq!(
        (legacy.coverage.as_deref(), legacy.caught_up),
        (Some("partial"), false)
    );
}

/// Drain `home` against its engine once, the way `comemory sync` does.
fn drain_once(home: &mut crate::domains::sync::replica::test_support::Home, auth: &AuthFile) {
    let (paths, cfg) = (home.paths.clone(), home.cfg.clone());
    drain::drain(
        &paths,
        &cfg,
        &mut home.conn,
        auth,
        (Mode::Manual, Legs::Both),
    )
    .expect("drain");
}

#[test]
fn an_upstream_whose_restore_is_unverified_holds_the_exchange() {
    use crate::domains::sync::replica::restore_state::{self, State};
    let engine = LiveEngine::start();
    let mut home = client_of(&engine);
    let auth = AuthFile::load(&home.paths).expect("load").expect("auth");
    home.save(BODY, &["sync"]);
    let upstream = crate::store::connection::open(engine.session.home.path().join("comemory.db"))
        .expect("the engine's database");
    restore_state::set(&upstream, State::ErasureUnknown).expect("set");

    drain_once(&mut home, &auth);

    let held = status(&home.conn, &auth).expect("status");
    assert_eq!(held.network, "restore_unverified", "{held:?}");
    assert_eq!(held.retry_at, None, "no backoff: the next pass asks again");
    assert!(!held.caught_up);
    assert_eq!(held.outbox.pending, 1, "the edit is kept, not sent");
    assert!(engine.feed().is_empty(), "the upstream received nothing");

    restore_state::clear(&upstream).expect("clear");
    drain_once(&mut home, &auth);

    let released = status(&home.conn, &auth).expect("status");
    assert_eq!(released.network, "ok", "{released:?}");
    assert_eq!(released.outbox.pending, 0);
    assert_eq!(engine.feed().len(), 1, "the edit reached the upstream");
}

#[test]
fn a_restore_pending_here_records_restore_unverified_and_sends_nothing() {
    use crate::domains::sync::replica::restore_state;
    let engine = LiveEngine::start();
    let mut home = client_of(&engine);
    let auth = AuthFile::load(&home.paths).expect("load").expect("auth");
    home.save(BODY, &["sync"]);
    let pending = restore_state::pending_path(&home.paths);
    std::fs::write(&pending, "{}").expect("a restore mid-swap");

    drain_once(&mut home, &auth);

    let held = status(&home.conn, &auth).expect("status");
    assert_eq!(held.network, "restore_unverified", "{held:?}");
    assert!(
        held.last_error
            .as_deref()
            .is_some_and(|e| e.contains("backup restore")),
        "{held:?}"
    );
    assert_eq!(held.outbox.pending, 1);
    assert!(engine.feed().is_empty(), "nothing was sent");

    std::fs::remove_file(&pending).expect("the restore finished");
    drain_once(&mut home, &auth);
    assert_eq!(status(&home.conn, &auth).expect("status").network, "ok");
    assert_eq!(engine.feed().len(), 1);
}
