#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `GET /api/v1/consolidate` end to end through the real in-process router
//! (`tests/common/serve_state.rs`) over a real store seeded with genuine
//! near-duplicates: the envelope and report shape, the explicit `repo`
//! filter, and — the reason this route is not a plain passthrough — the
//! request-scope default, where a server pinned with `serve --repo <r>`
//! narrows a query that names no repo of its own.

use comemory::memory::Kind;
use comemory::serve::RootOverrides;

use crate::test_common::serve_state::{self, Session};

/// Two bodies one word apart — near-duplicates under the default
/// `rank.near_dup_hamming` radius, which is what makes a cluster.
const ALPHA_A: &str = "the migration runner takes an advisory lock before applying any schema \
                       change, so two servers starting at once cannot both migrate";
const ALPHA_B: &str = "the migration runner takes an advisory lock before applying every schema \
                       change, so two servers starting at once cannot both migrate";
const BETA_A: &str = "search results are fused with reciprocal rank fusion over the lexical and \
                      vector legs, then reranked by four multiplicative priors";
const BETA_B: &str = "search results are fused with reciprocal rank fusion over the lexical and \
                      vector legs, then reranked by five multiplicative priors";

/// Seed both repos' near-duplicate pairs into `session`.
fn seed_both_repos(session: &Session) {
    serve_state::save(session, ALPHA_A, Kind::Convention, "alpha");
    serve_state::save(session, ALPHA_B, Kind::Convention, "alpha");
    serve_state::save(session, BETA_A, Kind::Discovery, "beta");
    serve_state::save(session, BETA_B, Kind::Discovery, "beta");
}

/// Every repo named by a member of any reported cluster, deduplicated.
fn cluster_repos(data: &serde_json::Value) -> Vec<String> {
    let mut repos: Vec<String> = data["clusters"]["items"]
        .as_array()
        .expect("clusters.items array")
        .iter()
        .flat_map(|c| c["members"].as_array().expect("members array"))
        .filter_map(|m| m["repo"].as_str().map(str::to_owned))
        .collect();
    repos.sort();
    repos.dedup();
    repos
}

#[tokio::test]
async fn consolidate_reports_every_repos_clusters_by_default() {
    let session = serve_state::session(false);
    seed_both_repos(&session);

    let res = serve_state::send(&session, "GET", "/api/v1/consolidate", None).await;

    assert_eq!(res.status.as_u16(), 200, "body: {}", res.text);
    assert_eq!(res.json["ok"], true);
    assert_eq!(res.json["meta"]["command"], "consolidate");
    let data = &res.json["data"];
    assert_eq!(data["scanned"], 4, "all four seeded rows are fingerprinted");
    assert_eq!(data["skipped_unhashed"], 0);
    assert_eq!(
        data["clustered"], 4,
        "both near-duplicate pairs cluster: {}",
        res.text
    );
    assert_eq!(
        cluster_repos(data),
        vec!["alpha".to_string(), "beta".to_string()],
        "an unscoped scan spans every repo: {}",
        res.text
    );
    assert_eq!(data["radius"], 8, "the configured rank.near_dup_hamming");
}

#[tokio::test]
async fn an_explicit_repo_query_narrows_the_scan() {
    let session = serve_state::session(false);
    seed_both_repos(&session);

    let res = serve_state::send(&session, "GET", "/api/v1/consolidate?repo=beta", None).await;

    assert_eq!(res.status.as_u16(), 200, "body: {}", res.text);
    let data = &res.json["data"];
    assert_eq!(data["scanned"], 2, "only beta's rows are compared");
    assert_eq!(data["clustered"], 2);
    assert_eq!(cluster_repos(data), vec!["beta".to_string()]);
}

#[tokio::test]
async fn a_pinned_server_scope_defaults_the_repo_filter() {
    let session = serve_state::session_with(false, RootOverrides::new(), Some("alpha".to_string()));
    seed_both_repos(&session);

    // No `repo` in the query: the request scope supplies it.
    let scoped = serve_state::send(&session, "GET", "/api/v1/consolidate", None).await;
    assert_eq!(scoped.status.as_u16(), 200, "body: {}", scoped.text);
    assert_eq!(
        scoped.json["data"]["scanned"], 2,
        "the pinned scope narrows a query that names no repo: {}",
        scoped.text
    );
    assert_eq!(
        cluster_repos(&scoped.json["data"]),
        vec!["alpha".to_string()]
    );

    // An explicit query repo still wins over the pin.
    let explicit = serve_state::send(&session, "GET", "/api/v1/consolidate?repo=beta", None).await;
    assert_eq!(explicit.status.as_u16(), 200, "body: {}", explicit.text);
    assert_eq!(
        cluster_repos(&explicit.json["data"]),
        vec!["beta".to_string()],
        "an explicit repo overrides the pin: {}",
        explicit.text
    );
}
