#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The console-api spec §9 additions to the prune resource, driven through
//! the real in-process router: `GET /api/v1/prune/candidates` (an alias of
//! `GET /prune`), `POST /api/v1/prune {ids}` (apply only these candidates)
//! and `POST /api/v1/prune {dry_run}` (the HTTP-only inverse of `apply`).
//!
//! Candidates are real: memories saved through `api::save` and then made
//! prune-eligible by the same mirror-doctoring the CLI prune suite uses
//! (`tests/common/cli_prune_support.rs`'s rule — quality 2 plus a
//! back-dated `last_accessed`), so `low_value::detect` flags them for the
//! documented reason rather than a fabricated one.

use crate::test_common::serve_state::{self, Session};

use comemory::memory::Kind;
use serde_json::json;

/// Save a memory into `session`'s store and return its id.
fn save(session: &Session, body: &str) -> String {
    serve_state::save(session, body, Kind::Note, "demo");
    let conn = comemory::store::connection::open(session.home.path().join("comemory.db"))
        .expect("open db");
    conn.query_row("SELECT id FROM memories WHERE body = ?1", [body], |r| {
        r.get::<_, String>(0)
    })
    .expect("read saved id")
}

/// Doctor the mirror row so `low_value::detect` flags it: quality 2 and a
/// long-past `last_accessed` push the ACT-R activation below the -2.0 floor.
fn make_prune_eligible(session: &Session, id: &str) {
    let conn = comemory::store::connection::open(session.home.path().join("comemory.db"))
        .expect("open db");
    conn.execute(
        "UPDATE memories SET quality = 2, last_accessed = '2025-01-01T00:00:00Z' WHERE id = ?1",
        [id],
    )
    .expect("doctor row");
}

/// Live markdown files whose name starts with `id`.
fn live_files(session: &Session, id: &str) -> usize {
    std::fs::read_dir(session.home.path().join("memories"))
        .expect("read memories dir")
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(id))
        .count()
}

#[tokio::test]
async fn prune_candidates_is_an_alias_of_prune() {
    let session = serve_state::session(false);
    let id = save(&session, "an alias-visible prune candidate");
    make_prune_eligible(&session, &id);

    let direct = serve_state::send(&session, "GET", "/api/v1/prune", None).await;
    let alias = serve_state::send(&session, "GET", "/api/v1/prune/candidates", None).await;

    assert_eq!(direct.status.as_u16(), 200, "body: {}", direct.text);
    assert_eq!(alias.status.as_u16(), 200, "body: {}", alias.text);
    assert_eq!(
        alias.json["data"], direct.json["data"],
        "the alias must return the same report"
    );
    assert_eq!(
        alias.json["data"]["low_value_memories"]["items"][0]["id"],
        json!(id)
    );
}

#[tokio::test]
async fn post_prune_with_ids_soft_deletes_only_the_listed_candidate() {
    let session = serve_state::session(false);
    let doomed = save(&session, "the candidate the console selected");
    let spared = save(&session, "the candidate the console left alone");
    make_prune_eligible(&session, &doomed);
    make_prune_eligible(&session, &spared);

    let res = serve_state::send(
        &session,
        "POST",
        "/api/v1/prune",
        Some(json!({ "apply": true, "confirm": true, "ids": [doomed] })),
    )
    .await;

    assert_eq!(res.status.as_u16(), 200, "body: {}", res.text);
    assert_eq!(live_files(&session, &doomed), 0, "the listed id is pruned");
    assert_eq!(live_files(&session, &spared), 1, "the other one survives");
}

#[tokio::test]
async fn post_prune_with_dry_run_true_deletes_nothing_even_with_confirm() {
    let session = serve_state::session(false);
    let id = save(&session, "a candidate that must survive a dry run");
    make_prune_eligible(&session, &id);

    let res = serve_state::send(
        &session,
        "POST",
        "/api/v1/prune",
        Some(json!({ "apply": true, "dry_run": true, "confirm": true })),
    )
    .await;

    assert_eq!(res.status.as_u16(), 200, "body: {}", res.text);
    assert_eq!(
        res.json["data"]["low_value_memories"]["items"][0]["id"],
        json!(id),
        "the scan still reports the candidate"
    );
    assert_eq!(
        live_files(&session, &id),
        1,
        "dry_run:true must override apply:true"
    );
}

#[tokio::test]
async fn post_prune_with_dry_run_false_applies() {
    let session = serve_state::session(false);
    let id = save(&session, "a candidate a dry_run:false request prunes");
    make_prune_eligible(&session, &id);

    let res = serve_state::send(
        &session,
        "POST",
        "/api/v1/prune",
        Some(json!({ "dry_run": false, "confirm": true })),
    )
    .await;

    assert_eq!(res.status.as_u16(), 200, "body: {}", res.text);
    assert_eq!(
        live_files(&session, &id),
        0,
        "dry_run:false must apply even without an explicit `apply`"
    );
}

#[tokio::test]
async fn post_prune_with_ids_still_requires_confirm() {
    let session = serve_state::session(false);
    let id = save(&session, "a candidate protected by the confirm gate");
    make_prune_eligible(&session, &id);

    let res = serve_state::send(
        &session,
        "POST",
        "/api/v1/prune",
        Some(json!({ "apply": true, "ids": [id.clone()] })),
    )
    .await;

    assert_eq!(res.status.as_u16(), 400, "body: {}", res.text);
    assert_eq!(res.json["error"]["code"], "confirmation_required");
    assert_eq!(live_files(&session, &id), 1);
}
