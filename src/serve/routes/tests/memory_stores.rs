#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `/api/v1/memory-stores*` end-to-end through the real router
//! (`tests/common/serve_state.rs`), against a real data dir: memories saved
//! through `api::save::run`, a real `git init`ed store, and a real bare repo
//! as the push target (console-api spec §10, AC-19).

use std::time::Duration;

use comemory::memory::Kind;
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::test_common::git_repo::{init_repo, run_git};
use crate::test_common::serve_state::{self, Session};

/// Poll `GET /api/v1/jobs/{id}` until the job reports a terminal status,
/// returning the envelope's `data` object.
async fn poll_job(session: &Session, job_id: &str) -> Value {
    for _ in 0..400 {
        let resp = serve_state::send(session, "GET", &format!("/api/v1/jobs/{job_id}"), None).await;
        assert_eq!(resp.status, 200, "job poll body: {}", resp.text);
        let status = resp.json["data"]["status"].as_str().unwrap_or_default();
        if matches!(status, "done" | "error" | "cancelled") {
            return resp.json["data"].clone();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("job {job_id} never reached a terminal status");
}

/// HEAD of a bare repo, read through `--git-dir` (it has no work tree).
fn remote_head(bare: &TempDir) -> String {
    let out = std::process::Command::new("git")
        .args([
            "--git-dir",
            &bare.path().to_string_lossy(),
            "rev-parse",
            "HEAD",
        ])
        .output()
        .expect("invoke git rev-parse");
    assert!(out.status.success(), "git rev-parse in the bare remote");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// AC-19 (first half): exactly one store, and its `markdown_files` is the
/// real on-disk count after two saves.
#[tokio::test]
async fn list_returns_exactly_one_store_matching_the_on_disk_count() {
    let session = serve_state::session(false);
    serve_state::save(
        &session,
        "first console memory store row",
        Kind::Note,
        "demo",
    );
    serve_state::save(
        &session,
        "second console memory store row",
        Kind::Note,
        "demo",
    );

    let resp = serve_state::send(&session, "GET", "/api/v1/memory-stores", None).await;

    assert_eq!(resp.status, 200, "body: {}", resp.text);
    let stores = resp.json["data"].as_array().expect("data array");
    assert_eq!(stores.len(), 1, "body: {}", resp.text);
    let on_disk = std::fs::read_dir(session.home.path().join("memories"))
        .expect("read memories dir")
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .count();
    assert_eq!(on_disk, 2, "two saves must write two markdown files");
    assert_eq!(stores[0]["markdown_files"].as_u64(), Some(on_disk as u64));
    assert_eq!(stores[0]["id"], json!("default"));
    assert_eq!(stores[0]["push_on_save"], json!(false));
    assert_eq!(stores[0]["sync"]["is_git_repo"], json!(false));
    assert_eq!(resp.json["meta"]["command"], json!("memory-stores"));
}

#[tokio::test]
async fn get_by_id_returns_the_store_and_an_unknown_id_is_404() {
    let session = serve_state::session(false);
    serve_state::save(&session, "memory store lookup row", Kind::Note, "demo");

    let ok = serve_state::send(&session, "GET", "/api/v1/memory-stores/default", None).await;
    assert_eq!(ok.status, 200, "body: {}", ok.text);
    assert_eq!(ok.json["data"]["id"], json!("default"));
    assert_eq!(ok.json["data"]["markdown_files"], json!(1));

    let missing = serve_state::send(&session, "GET", "/api/v1/memory-stores/second", None).await;
    assert_eq!(missing.status, 404, "body: {}", missing.text);
    assert_eq!(missing.json["ok"], json!(false));
    assert_eq!(missing.json["error"]["code"], json!("not_found"));
}

/// Spec Non-Goal 3: a second store is `501 unsupported`, not a `404` and not
/// a silent success.
#[tokio::test]
async fn create_is_501_unsupported() {
    let session = serve_state::session(false);

    let resp = serve_state::send(
        &session,
        "POST",
        "/api/v1/memory-stores",
        Some(json!({"path": "/tmp/second-store", "push_on_save": true})),
    )
    .await;

    assert_eq!(resp.status, 501, "body: {}", resp.text);
    assert_eq!(resp.json["ok"], json!(false));
    assert_eq!(resp.json["error"]["code"], json!("unsupported"));
    assert!(
        resp.json["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("one memory store"),
        "body: {}",
        resp.text
    );
}

/// Read-only outranks the `501`: a `--read-only` server refuses the create
/// before the model refusal is ever reached (AC-19 ordering).
#[tokio::test]
async fn create_on_a_read_only_server_is_405_read_only() {
    let session = serve_state::session(true);

    let resp = serve_state::send(
        &session,
        "POST",
        "/api/v1/memory-stores",
        Some(json!({"path": "/tmp/second-store"})),
    )
    .await;

    assert_eq!(resp.status, 405, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], json!("read_only"));
}

/// AC-19 (second half): `PATCH` flips `push_on_save` by writing `[git]
/// auto_sync` into the real `config.toml`, and the next `GET` — served from
/// the reloaded `AppState.cfg` — reports it.
#[tokio::test]
async fn patch_writes_the_git_section_and_the_next_get_reflects_it() {
    let session = serve_state::session(false);
    let config = session.home.path().join("config.toml");
    assert!(!config.exists(), "a fresh data dir has no config.toml");

    let patched = serve_state::send(
        &session,
        "PATCH",
        "/api/v1/memory-stores/default",
        Some(json!({"push_on_save": true})),
    )
    .await;

    assert_eq!(patched.status, 200, "body: {}", patched.text);
    assert_eq!(patched.json["data"]["push_on_save"], json!(true));
    let text = std::fs::read_to_string(&config).expect("config.toml written");
    assert!(text.contains("[git]"), "config.toml:\n{text}");
    assert!(text.contains("auto_sync = true"), "config.toml:\n{text}");

    let after = serve_state::send(&session, "GET", "/api/v1/memory-stores", None).await;
    assert_eq!(
        after.json["data"][0]["push_on_save"],
        json!(true),
        "the reloaded config must drive the next read: {}",
        after.text
    );

    // A second PATCH touches only the key it was given.
    let remote = serve_state::send(
        &session,
        "PATCH",
        "/api/v1/memory-stores/default",
        Some(json!({"remote": "backup"})),
    )
    .await;
    assert_eq!(remote.status, 200, "body: {}", remote.text);
    assert_eq!(remote.json["data"]["remote"], json!("backup"));
    let text = std::fs::read_to_string(&config).expect("config.toml");
    assert!(text.contains("remote = \"backup\""), "config.toml:\n{text}");
    assert!(
        text.contains("auto_sync = true"),
        "the earlier key survives a partial patch:\n{text}"
    );
    let after = serve_state::send(&session, "GET", "/api/v1/memory-stores/default", None).await;
    assert_eq!(after.json["data"]["remote"], json!("backup"));
    assert_eq!(after.json["data"]["push_on_save"], json!(true));
}

#[tokio::test]
async fn patch_on_an_unknown_id_is_404_and_writes_nothing() {
    let session = serve_state::session(false);

    let resp = serve_state::send(
        &session,
        "PATCH",
        "/api/v1/memory-stores/second",
        Some(json!({"push_on_save": true})),
    )
    .await;

    assert_eq!(resp.status, 404, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], json!("not_found"));
    assert!(
        !session.home.path().join("config.toml").exists(),
        "the id is checked before the file is touched"
    );
}

#[tokio::test]
async fn patch_on_a_read_only_server_is_405_read_only() {
    let session = serve_state::session(true);

    let resp = serve_state::send(
        &session,
        "PATCH",
        "/api/v1/memory-stores/default",
        Some(json!({"push_on_save": true})),
    )
    .await;

    assert_eq!(resp.status, 405, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], json!("read_only"));
    assert!(!session.home.path().join("config.toml").exists());
}

#[tokio::test]
async fn sync_on_a_non_git_data_dir_ends_the_job_with_bad_request() {
    let session = serve_state::session(false);
    serve_state::save(&session, "unsyncable memory store row", Kind::Note, "demo");

    let accepted = serve_state::send(
        &session,
        "POST",
        "/api/v1/memory-stores/default/sync",
        Some(json!({})),
    )
    .await;
    assert_eq!(accepted.status, 202, "body: {}", accepted.text);
    let job_id = accepted.json["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();

    let job = poll_job(&session, &job_id).await;

    assert_eq!(job["status"], json!("error"), "job: {job}");
    assert_eq!(job["command"], json!("store-sync"));
    assert_eq!(job["error"]["code"], json!("bad_request"), "job: {job}");
    assert!(
        job["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("not a git repository"),
        "job: {job}"
    );
}

#[tokio::test]
async fn sync_job_commits_and_pushes_to_a_real_remote() {
    let session = serve_state::session(false);
    let bare = TempDir::new().expect("bare tempdir");
    run_git(
        bare.path(),
        &["init", "-q", "--bare", "--initial-branch=main"],
    );
    init_repo(session.home.path());
    run_git(
        session.home.path(),
        &["commit", "-q", "--allow-empty", "-m", "init"],
    );
    run_git(
        session.home.path(),
        &["remote", "add", "origin", &bare.path().to_string_lossy()],
    );
    run_git(session.home.path(), &["push", "-q", "-u", "origin", "main"]);
    let before = remote_head(&bare);
    serve_state::save(&session, "syncable memory store row", Kind::Note, "demo");

    let accepted = serve_state::send(
        &session,
        "POST",
        "/api/v1/memory-stores/default/sync",
        Some(json!({"push": true})),
    )
    .await;
    assert_eq!(accepted.status, 202, "body: {}", accepted.text);
    let job_id = accepted.json["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();

    let job = poll_job(&session, &job_id).await;

    assert_eq!(job["status"], json!("done"), "job: {job}");
    assert_eq!(job["result"]["committed"], json!(true), "job: {job}");
    assert_eq!(job["result"]["pushed"], json!(true), "job: {job}");
    assert_eq!(job["result"]["pulled"], json!(true), "job: {job}");
    let commit = job["result"]["commit"].as_str().expect("commit oid");
    let after = remote_head(&bare);
    assert_ne!(after, before, "the bare remote's HEAD must have advanced");
    assert_eq!(after, commit, "the remote must hold the sync commit");
    let log_tail = job["log_tail"].as_array().expect("log_tail");
    assert!(
        log_tail
            .iter()
            .any(|l| l.as_str().unwrap_or_default().starts_with("git push")),
        "each git step is logged into the job: {log_tail:?}"
    );

    // The store now reports the pushed state: in a repo, on main, nothing
    // ahead of the upstream.
    let store = serve_state::send(&session, "GET", "/api/v1/memory-stores/default", None).await;
    assert_eq!(store.json["data"]["sync"]["is_git_repo"], json!(true));
    assert_eq!(store.json["data"]["sync"]["branch"], json!("main"));
    assert_eq!(
        store.json["data"]["sync"]["ahead"],
        json!(0),
        "body: {}",
        store.text
    );
    assert_eq!(store.json["data"]["sync"]["behind"], json!(0));
    assert_eq!(store.json["data"]["sync"]["dirty"], json!(0));
    assert_eq!(
        store.json["data"]["remote"].as_str(),
        Some(bare.path().to_string_lossy().as_ref()),
        "the origin url is reported when [git] remote is unset"
    );
}

#[tokio::test]
async fn sync_on_a_read_only_server_is_405_read_only() {
    let session = serve_state::session(true);

    let resp = serve_state::send(
        &session,
        "POST",
        "/api/v1/memory-stores/default/sync",
        Some(json!({"push": false})),
    )
    .await;

    assert_eq!(resp.status, 405, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], json!("read_only"));
}

/// The PATCH body is rendered from the RELOADED config, not from the
/// file-only view the patch itself computed: with `COMEMORY_GIT_AUTO_SYNC`
/// set, the reload re-applies the env layer, and the body must agree with
/// the next `GET` — which is what the console renders after the toggle.
#[tokio::test]
async fn patch_answers_from_the_reloaded_config_so_an_env_override_wins() {
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::set_var("COMEMORY_GIT_AUTO_SYNC", "true") };
    let session = serve_state::session(false);
    let patched = serve_state::send(
        &session,
        "PATCH",
        "/api/v1/memory-stores/default",
        Some(json!({"push_on_save": false})),
    )
    .await;
    let after = serve_state::send(&session, "GET", "/api/v1/memory-stores/default", None).await;
    // SAFETY: nextest runs each #[test] in its own process — set_var/remove_var cannot race with another test.
    unsafe { std::env::remove_var("COMEMORY_GIT_AUTO_SYNC") };

    assert_eq!(patched.status, 200, "body: {}", patched.text);
    let text =
        std::fs::read_to_string(session.home.path().join("config.toml")).expect("config.toml");
    assert!(
        text.contains("auto_sync = false"),
        "the file records the request:\n{text}"
    );
    assert_eq!(
        patched.json["data"]["push_on_save"],
        json!(true),
        "the env override wins in the body, as it does on reload: {}",
        patched.text
    );
    assert_eq!(
        patched.json["data"]["push_on_save"], after.json["data"]["push_on_save"],
        "PATCH body and the next GET must agree: {} vs {}",
        patched.text, after.text
    );
}
