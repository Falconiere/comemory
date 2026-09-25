#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The drain's stop check against a REAL engine: a logout barrier raised
//! after a caller loaded its credential ends the run `cancelled` before it
//! sends anything, and the owed operations stay owed.

use crate::domains::sync::AuthFile;
use crate::domains::sync::auth_barrier;
use crate::domains::sync::drain::report::End;
use crate::domains::sync::drain::session::{Legs, Mode};
use crate::domains::sync::drain::test_support::{LiveEngine, client_of};
use crate::domains::sync::drain::{drain, stop};
use crate::domains::sync::replica::test_support::BODY;

#[test]
fn a_barrier_raised_under_a_loaded_credential_cancels_the_run() {
    let engine = LiveEngine::start();
    let mut a = client_of(&engine);
    a.save(BODY, &["sync"]);
    let auth = AuthFile::load(&a.paths).expect("load").expect("auth");
    auth_barrier::raise(&a.paths).expect("raise");
    assert!(stop::requested(&a.paths));

    let (paths, cfg) = (a.paths.clone(), a.cfg.clone());
    let drained = drain(
        &paths,
        &cfg,
        &mut a.conn,
        &auth,
        (Mode::Unattended, Legs::Both),
    )
    .expect("drain");
    assert_eq!(drained.exchange.end, End::Cancelled);
    assert_eq!((drained.exchange.pushed, drained.exchange.pulled), (0, 0));
    assert!(
        !drained.exchange.more,
        "a cancelled run asks for no other pass"
    );
    assert_eq!(
        engine.changes(0, None).entries.len(),
        0,
        "nothing reached the engine"
    );

    // The credential comes back through an explicit login, and the same
    // operation goes out then.
    auth.save(&a.paths).expect("save clears the barrier");
    let drained = drain(
        &paths,
        &cfg,
        &mut a.conn,
        &auth,
        (Mode::Unattended, Legs::Both),
    )
    .expect("drain");
    assert_eq!(
        (drained.exchange.pushed, drained.exchange.end),
        (1, End::CaughtUp)
    );
}

#[test]
fn no_barrier_means_no_stop() {
    let engine = LiveEngine::start();
    let a = client_of(&engine);
    assert!(!stop::requested(&a.paths));
}
