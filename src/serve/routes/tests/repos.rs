#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `GET /api/v1/repos` through the real in-process router — specifically
//! the rule this resource's module doc singles out: the inventory is the
//! one repo-bearing read that stays OUTSIDE the default repo scope, since
//! it is how a client discovers which scopes exist. A `serve --repo alpha`
//! server must still list `beta`, while an explicit `?repo=` narrows for
//! everyone.
//!
//! The `"indexing"` overlay has its own coverage over a real index job in
//! `repos_admin.rs`; this file seeds `repo_marker` rows through the real
//! store writers instead, which is all the inventory read needs.

use comemory::config::Paths;
use comemory::memory::Kind;
use comemory::serve::RootOverrides;
use comemory::store::{code_row, connection};

use crate::test_common::serve_state::{self, Session};

/// The HEAD oid [`mark_repo`] records, asserted back out of `GET /repos`.
const MARKED_HEAD: &str = "0000000000000000000000000000000000000000";

/// Record `repo` in `repo_marker` exactly as an `index-code` run does —
/// its working-tree root, then its last-indexed HEAD.
fn mark_repo(session: &Session, repo: &str) {
    let paths = Paths::new(session.home.path());
    let conn = connection::open(paths.db_path()).expect("open db");
    let root = session.home.path().join(repo);
    code_row::upsert_repo_root(&conn, repo, &root.to_string_lossy()).expect("upsert repo root");
    code_row::upsert_last_indexed(&conn, repo, MARKED_HEAD).expect("upsert last indexed");
}

/// The repo labels `GET /repos` reported, in response order.
fn labels(json: &serde_json::Value) -> Vec<String> {
    json["data"]["repos"]
        .as_array()
        .expect("data.repos array")
        .iter()
        .map(|r| r["repo"].as_str().expect("repo label").to_string())
        .collect()
}

/// A session (optionally repo-pinned) with `alpha` and `beta` indexed and
/// one memory filed under `alpha`.
fn seeded(pin: Option<&str>) -> Session {
    let session = serve_state::session_with(
        false,
        RootOverrides::new(),
        pin.map(std::string::ToString::to_string),
    );
    mark_repo(&session, "alpha");
    mark_repo(&session, "beta");
    serve_state::save(&session, "alpha's one memory", Kind::Note, "alpha");
    session
}

#[tokio::test]
async fn repos_lists_every_indexed_repo_with_its_counters() {
    let session = seeded(None);

    let res = serve_state::send(&session, "GET", "/api/v1/repos", None).await;

    assert_eq!(res.status.as_u16(), 200, "body: {}", res.text);
    assert_eq!(res.json["ok"], true);
    assert_eq!(res.json["meta"]["command"], "repos");
    assert_eq!(
        labels(&res.json),
        vec!["alpha".to_string(), "beta".to_string()]
    );
    let alpha = &res.json["data"]["repos"][0];
    assert_eq!(
        alpha["last_head"].as_str(),
        Some(MARKED_HEAD),
        "the seeded HEAD is reported back verbatim: {}",
        res.text
    );
    assert!(
        alpha["last_indexed_at"].as_str().is_some(),
        "the marker's index stamp is reported: {}",
        res.text
    );
    assert_eq!(alpha["memories"], 1, "the memory counter is a real join");
    assert_eq!(alpha["symbols"], 0, "nothing was indexed, only marked");
    assert_eq!(alpha["archived"], false);
    assert_eq!(
        alpha["indexing_job"],
        serde_json::Value::Null,
        "no live job, so no overlay: {}",
        res.text
    );
}

#[tokio::test]
async fn a_pinned_server_still_lists_every_repo() {
    let session = seeded(Some("alpha"));

    let res = serve_state::send(&session, "GET", "/api/v1/repos", None).await;

    assert_eq!(res.status.as_u16(), 200, "body: {}", res.text);
    assert_eq!(
        labels(&res.json),
        vec!["alpha".to_string(), "beta".to_string()],
        "the inventory is how a client discovers scopes, so `serve --repo alpha` \
         must not hide `beta`: {}",
        res.text
    );
}

#[tokio::test]
async fn an_explicit_repo_query_narrows_the_inventory() {
    let session = seeded(Some("alpha"));

    let res = serve_state::send(&session, "GET", "/api/v1/repos?repo=beta", None).await;

    assert_eq!(res.status.as_u16(), 200, "body: {}", res.text);
    assert_eq!(
        labels(&res.json),
        vec!["beta".to_string()],
        "an explicit query repo narrows — and outranks the pin: {}",
        res.text
    );
}
