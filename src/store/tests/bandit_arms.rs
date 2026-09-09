#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/bandit_arms.rs` — seed/load round-trip and the
//! win/loss posterior update. Moved out of `eval::tests::bandit` alongside
//! `record_outcome` (spec
//! `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`).

use comemory::store::bandit_arms::{self, NewArm};
use comemory::store::connection;

const AT: &str = "2026-07-20T12:00:00Z";

fn seed_one(conn: &rusqlite::Connection, arm_id: &str) {
    bandit_arms::seed(
        conn,
        &NewArm {
            arm_id,
            rrf_k: 60.0,
            decay: 0.5,
            mmr_lambda: 0.7,
            bm25_body: 1.0,
            bm25_tags: 3.0,
            at: AT,
        },
    )
    .expect("seed");
}

#[test]
fn load_returns_none_for_unseeded_arm() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    assert!(
        bandit_arms::load(&conn, "no-such-arm")
            .expect("load")
            .is_none()
    );
}

#[test]
fn seed_then_load_returns_beta_1_1_priors() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_one(&conn, "arm-a");
    let state = bandit_arms::load(&conn, "arm-a")
        .expect("load")
        .expect("row exists");
    assert!((state.alpha - 1.0).abs() < f64::EPSILON);
    assert!((state.beta - 1.0).abs() < f64::EPSILON);
    assert_eq!(state.pulls, 0);
    assert!(state.last_mrr.is_none());
}

#[test]
fn re_seeding_does_not_reset_an_already_pulled_arm() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_one(&conn, "arm-a");
    bandit_arms::record_outcome(&conn, "arm-a", true, 0.9, AT).expect("win");
    seed_one(&conn, "arm-a");
    let state = bandit_arms::load(&conn, "arm-a")
        .expect("load")
        .expect("row exists");
    assert!(
        (state.alpha - 2.0).abs() < f64::EPSILON,
        "re-seed must not reset alpha"
    );
    assert_eq!(state.pulls, 1);
}

#[test]
fn record_outcome_bumps_alpha_on_win_and_beta_on_loss() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    seed_one(&conn, "arm-a");

    bandit_arms::record_outcome(&conn, "arm-a", true, 0.9, AT).expect("win");
    let state = bandit_arms::load(&conn, "arm-a")
        .expect("load")
        .expect("row exists after win");
    assert!((state.alpha - 2.0).abs() < f64::EPSILON);
    assert!((state.beta - 1.0).abs() < f64::EPSILON);
    assert_eq!(state.pulls, 1);
    assert!((state.last_mrr.expect("last_mrr set") - 0.9).abs() < f64::EPSILON);

    bandit_arms::record_outcome(&conn, "arm-a", false, 0.4, AT).expect("loss");
    let state = bandit_arms::load(&conn, "arm-a")
        .expect("load")
        .expect("row exists after loss");
    assert!((state.alpha - 2.0).abs() < f64::EPSILON);
    assert!((state.beta - 2.0).abs() < f64::EPSILON);
    assert_eq!(state.pulls, 2);
    assert!((state.last_mrr.expect("last_mrr set") - 0.4).abs() < f64::EPSILON);
}
