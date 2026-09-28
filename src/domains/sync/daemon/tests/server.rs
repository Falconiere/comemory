#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Tests for the control server's `status` answer: it re-probes the store,
//! so a restore left unverified after the last pass reads unhealthy now.

use std::sync::Arc;

use super::{Ctx, status};
use crate::config::Paths;
use crate::domains::sync::daemon::coordinator::initial_readiness;
use crate::domains::sync::daemon::readiness::StoreState;
use crate::domains::sync::daemon::state::State;
use crate::domains::sync::daemon::worker::Queue;
use crate::domains::sync::replica::restore_state;

#[tokio::test]
async fn status_reprobes_the_store_on_every_request() {
    let home = tempfile::tempdir().expect("home");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let socket = home.path().join("daemon.sock");
    let state = State::new(initial_readiness(&paths, home.path(), &socket).expect("readiness"));
    let ctx = Ctx {
        token: String::new(),
        uid: 0,
        state: Arc::clone(&state),
        queue: Queue::new(),
        shutdown: Arc::default(),
        reload: Arc::default(),
        paths: paths.clone(),
    };
    assert_eq!(status(&ctx).await.store, StoreState::Absent);

    let conn = crate::store::connection::open(paths.db_path()).expect("open");
    assert_eq!(status(&ctx).await.store, StoreState::Ready);

    restore_state::set(&conn, restore_state::State::Merging).expect("set");
    let unverified = status(&ctx).await;
    assert_eq!(unverified.store, StoreState::RestoreUnverified);
    assert!(!unverified.store.is_healthy());
}
