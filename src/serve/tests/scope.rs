#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for `src/serve/scope.rs`: the `X-Comemory-Repo` header as a
//! default `repo` filter, driven end-to-end through the real router
//! (`tests/common/serve_state.rs`) against memories saved under two repo
//! labels (console-api spec AC-2).

use comemory::memory::Kind;
use comemory::serve::RootOverrides;
use comemory::serve::scope::RepoScope;

use crate::test_common::serve_state;

#[test]
fn apply_fills_only_an_absent_repo() {
    let scope = RepoScope(Some("a".into()));
    let mut absent = None;
    scope.fill_if_absent(&mut absent);
    assert_eq!(absent.as_deref(), Some("a"));
    let mut explicit = Some("b".to_string());
    scope.fill_if_absent(&mut explicit);
    assert_eq!(explicit.as_deref(), Some("b"));
    let mut untouched = None;
    RepoScope(None).fill_if_absent(&mut untouched);
    assert_eq!(untouched, None);
}

#[tokio::test]
async fn header_scopes_a_list_and_an_explicit_query_overrides_it() {
    let session = serve_state::session(false);
    serve_state::save(&session, "memory filed under repo a", Kind::Note, "a");
    serve_state::save(&session, "memory filed under repo b", Kind::Note, "b");

    let scoped = serve_state::send_headers(
        &session,
        "GET",
        "/api/v1/memories",
        &[
            ("Host", "127.0.0.1"),
            ("X-Comemory-Token", &session.token),
            ("X-Comemory-Repo", "a"),
        ],
        None,
    )
    .await;
    assert_eq!(scoped.status, 200, "body: {}", scoped.text);
    let items = scoped.json["data"]["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "body: {}", scoped.text);
    assert_eq!(items[0]["repo"], "a");

    let overridden = serve_state::send_headers(
        &session,
        "GET",
        "/api/v1/memories?repo=b",
        &[
            ("Host", "127.0.0.1"),
            ("X-Comemory-Token", &session.token),
            ("X-Comemory-Repo", "a"),
        ],
        None,
    )
    .await;
    assert_eq!(overridden.status, 200, "body: {}", overridden.text);
    let items = overridden.json["data"]["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "body: {}", overridden.text);
    assert_eq!(items[0]["repo"], "b");

    let unscoped = serve_state::send(&session, "GET", "/api/v1/memories", None).await;
    let items = unscoped.json["data"]["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "body: {}", unscoped.text);
}

/// `comemory serve --repo b` is the last fallback: it scopes a request that
/// carries neither the header nor an explicit `repo`, and loses to both.
#[tokio::test]
async fn server_default_repo_is_the_last_fallback() {
    let session = serve_state::session_with(false, RootOverrides::new(), Some("b".to_string()));
    serve_state::save(&session, "memory filed under repo a", Kind::Note, "a");
    serve_state::save(&session, "memory filed under repo b", Kind::Note, "b");

    let defaulted = serve_state::send(&session, "GET", "/api/v1/memories", None).await;
    let items = defaulted.json["data"]["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "body: {}", defaulted.text);
    assert_eq!(items[0]["repo"], "b");

    let header_wins = serve_state::send_headers(
        &session,
        "GET",
        "/api/v1/memories",
        &[
            ("Host", "127.0.0.1"),
            ("X-Comemory-Token", &session.token),
            ("X-Comemory-Repo", "a"),
        ],
        None,
    )
    .await;
    let items = header_wins.json["data"]["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "body: {}", header_wins.text);
    assert_eq!(items[0]["repo"], "a");

    let explicit_wins = serve_state::send(&session, "GET", "/api/v1/memories?repo=a", None).await;
    let items = explicit_wins.json["data"]["items"]
        .as_array()
        .expect("items");
    assert_eq!(items[0]["repo"], "a");
}
