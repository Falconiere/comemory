#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! In-process coverage of `PUT /api/v1/hooks/{name}?repo=` — the per-hook
//! path form added for the console (spec §6). Real temp git repos: the
//! assertions are against the `.git/hooks/` files themselves, not just the
//! reported rows.

use std::path::{Path, PathBuf};

use serde_json::json;
use tempfile::TempDir;

use crate::test_common::git_repo;
use crate::test_common::serve_state::{self, Session};

/// A real git repo to install hooks into, plus a session to drive. The
/// session is started with `--allow-path <repo>`: this route writes into a
/// caller-supplied working tree, so its path goes through the same
/// containment gate as `POST /sources` and `POST /code/index`, and a temp
/// repo under no configured root would otherwise be refused `403`.
fn repo_and_session(read_only: bool) -> (TempDir, PathBuf, Session) {
    let workspace = TempDir::new().expect("workspace");
    let repo = workspace.path().join("hooked");
    git_repo::init_repo(&repo);
    let session = serve_state::session_allowing(read_only, &[&repo]);
    (workspace, repo, session)
}

fn hook_file(repo: &Path, name: &str) -> PathBuf {
    repo.join(".git").join("hooks").join(name)
}

fn put_path(repo: &Path, name: &str) -> String {
    format!(
        "/api/v1/hooks/{name}?repo={}",
        repo.to_str().expect("utf8 path")
    )
}

/// The reported row for `name`, as `(installed, source)`. The lookup is by
/// name, so asserting the found row's own name would assert the lookup;
/// what the response has to get right — that the CANONICAL spelling is
/// what comes back, whatever the request used — is checked by
/// [`assert_canonical_names`] instead.
fn row(body: &serde_json::Value, name: &str) -> (bool, String) {
    let hooks = body["data"]["hooks"].as_array().expect("hooks array");
    let found = hooks
        .iter()
        .find(|row| row["name"] == name)
        .unwrap_or_else(|| panic!("no row for {name} in {body}"));
    (
        found["installed"].as_bool().unwrap_or(false),
        found["source"].as_str().unwrap_or_default().to_string(),
    )
}

/// Every reported row is named in the canonical, hyphenated spelling — no
/// row echoes the underscore form a caller may have sent.
fn assert_canonical_names(body: &serde_json::Value) {
    let hooks = body["data"]["hooks"].as_array().expect("hooks array");
    for row in hooks {
        let name = row["name"].as_str().unwrap_or_default();
        assert!(
            !name.contains('_'),
            "hook rows report the canonical hyphenated name, got {name:?} in {body}"
        );
    }
}

#[tokio::test]
async fn put_enables_exactly_one_hook_and_the_underscore_spelling_is_accepted() {
    let (_workspace, repo, session) = repo_and_session(false);

    let resp = serve_state::send(
        &session,
        "PUT",
        &put_path(&repo, "post_commit"),
        Some(json!({"enabled": true})),
    )
    .await;

    assert_eq!(resp.status, 200, "body: {}", resp.text);
    // The request spelled it `post_commit`; the response must not.
    assert_canonical_names(&resp.json);
    assert_eq!(row(&resp.json, "post-commit"), (true, "git".to_string()));
    assert!(!row(&resp.json, "post-merge").0);
    assert!(!row(&resp.json, "post-checkout").0);
    assert!(hook_file(&repo, "post-commit").exists());
    assert!(!hook_file(&repo, "post-merge").exists());
}

#[tokio::test]
async fn put_disables_the_hook_it_installed() {
    let (_workspace, repo, session) = repo_and_session(false);
    serve_state::send(
        &session,
        "PUT",
        &put_path(&repo, "post-commit"),
        Some(json!({"enabled": true})),
    )
    .await;
    assert!(hook_file(&repo, "post-commit").exists());

    let resp = serve_state::send(
        &session,
        "PUT",
        &put_path(&repo, "post-commit"),
        Some(json!({"enabled": false})),
    )
    .await;

    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert!(!row(&resp.json, "post-commit").0);
    assert!(!hook_file(&repo, "post-commit").exists());
}

#[tokio::test]
async fn the_config_backed_reinforcement_row_toggles_through_the_same_route() {
    let (_workspace, repo, session) = repo_and_session(false);

    let resp = serve_state::send(
        &session,
        "PUT",
        &put_path(&repo, "search-edit-reinforcement"),
        Some(json!({"enabled": false})),
    )
    .await;

    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(
        row(&resp.json, "search-edit-reinforcement"),
        (false, "config".to_string())
    );
    let config = std::fs::read_to_string(session.home.path().join("config.toml"))
        .expect("config.toml written");
    assert!(config.contains("enabled = false"), "config: {config}");
}

#[tokio::test]
async fn an_unknown_hook_name_is_rejected() {
    let (_workspace, repo, session) = repo_and_session(false);

    let resp = serve_state::send(
        &session,
        "PUT",
        &put_path(&repo, "pre_push"),
        Some(json!({"enabled": true})),
    )
    .await;

    assert_eq!(resp.status, 400, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], "usage");
}

#[tokio::test]
async fn a_read_only_server_refuses_the_put_with_405() {
    let (_workspace, repo, session) = repo_and_session(true);

    let resp = serve_state::send(
        &session,
        "PUT",
        &put_path(&repo, "post-commit"),
        Some(json!({"enabled": true})),
    )
    .await;

    assert_eq!(resp.status, 405, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], "read_only");
    assert!(!hook_file(&repo, "post-commit").exists());
}
