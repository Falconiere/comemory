#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Read-only-outranks-confirm coverage for `POST /api/v1/tune` and
//! `POST /api/v1/bandit` (AC-19), plus the discriminating proof that a
//! successful `tune --apply` job reloads `AppState.cfg` in-memory without a
//! server restart. Confirm-when-apply shape and `POST /api/v1/eval` live in
//! `tests/serve__routes__learning.rs`.

use crate::test_common::serve_learning_support as support;

use serde_json::Value;
use support::{jobs_total, poll_job_terminal, post, save, spawn_serve, spawn_serve_with};
use tempfile::TempDir;

// ── read-only outranks confirm (AC-19) ──────────────────────────────────

#[test]
fn v1_tune_read_only_without_confirm_is_405_not_400_ac19() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();
    let before = jobs_total(&client, &base, &token);

    let res = post(
        &client,
        &base,
        &token,
        "tune",
        &serde_json::json!({ "apply": true }),
    );
    assert_eq!(res.status().as_u16(), 405);
    let body: Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "read_only");
    assert_eq!(jobs_total(&client, &base, &token), before);
}

#[test]
fn v1_tune_read_only_with_confirm_is_still_405_read_only() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();

    let res = post(
        &client,
        &base,
        &token,
        "tune",
        &serde_json::json!({ "apply": true, "confirm": true }),
    );
    assert_eq!(res.status().as_u16(), 405);
    let body: Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "read_only");
}

#[test]
fn v1_tune_read_only_blocks_even_a_no_apply_request() {
    // `tune` is classified mutating regardless of `apply` (§Route map
    // Notes), so the read-only gate must still fire when `apply` is absent.
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();

    let res = post(&client, &base, &token, "tune", &serde_json::json!({}));
    assert_eq!(res.status().as_u16(), 405);
    let body: Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "read_only");
}

#[test]
fn v1_bandit_read_only_without_confirm_is_405_not_400_ac19() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();
    let before = jobs_total(&client, &base, &token);

    let res = post(
        &client,
        &base,
        &token,
        "bandit",
        &serde_json::json!({ "apply": true }),
    );
    assert_eq!(res.status().as_u16(), 405);
    let body: Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "read_only");
    assert_eq!(jobs_total(&client, &base, &token), before);
}

#[test]
fn v1_bandit_read_only_with_confirm_is_still_405_read_only() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();

    let res = post(
        &client,
        &base,
        &token,
        "bandit",
        &serde_json::json!({ "apply": true, "confirm": true }),
    );
    assert_eq!(res.status().as_u16(), 405);
    let body: Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "read_only");
}

// ── AppState.cfg reload after a successful tune --apply job ────────────

/// Search hits' top `memory_id`, via a real HTTP `GET /api/v1/memories/search`.
fn top_hit_id(client: &reqwest::blocking::Client, base: &str, token: &str, query: &str) -> String {
    let res = client
        .get(format!("{base}/api/v1/memories/search"))
        .header("X-Comemory-Token", token)
        .query(&[("query", query), ("k", "1")])
        .send()
        .expect("search");
    let status = res.status().as_u16();
    let body: Value = res.json().expect("json");
    assert_eq!(status, 200, "search failed: {body}");
    body["data"]["hits"][0]["memory_id"]
        .as_str()
        .expect("top hit memory_id")
        .to_string()
}

/// Build the tag-discriminated fixture: `config.toml` starts with the tags
/// BM25 column zeroed out, so a `postgres`-tagged memory's only textual
/// signal (its tag) contributes nothing and an untagged body-match memory
/// outranks it for `"postgres"`. Returns the golden file path and the
/// tagged memory's id.
fn tag_discriminated_fixture(home: &TempDir) -> (std::path::PathBuf, String) {
    save(home, "postgres advisory lock migration ordering", &[]);
    let tagged = save(
        home,
        "kubernetes ingress certificate renewal notes",
        &["postgres"],
    );
    let plain = save(home, "terraform state locking dynamodb backend", &[]);
    let golden = home.path().join("golden.yaml");
    std::fs::write(
        &golden,
        format!(
            "- query: postgres\n  relevant: [{tagged}]\n\
             - query: terraform state locking\n  relevant: [{plain}]\n"
        ),
    )
    .expect("write golden file");
    let config = home.path().join(".comemory").join("config.toml");
    std::fs::write(&config, "[retrieval]\nbm25_weights = [1.0, 0.0]\n")
        .expect("write starting config.toml");
    (golden, tagged)
}

/// `POST /api/v1/tune {golden,golden_only:true,k:1,seed:42,apply:true,
/// confirm:true}`, polled to a terminal status. Returns the job's `data`.
fn run_confirmed_tune_apply(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    golden: &std::path::Path,
) -> Value {
    let res = post(
        client,
        base,
        token,
        "tune",
        &serde_json::json!({
            "golden": golden.to_str().expect("utf8 path"),
            "golden_only": true,
            "k": 1,
            "seed": 42,
            "apply": true,
            "confirm": true,
        }),
    );
    assert_eq!(res.status().as_u16(), 202);
    let body: Value = res.json().expect("json");
    let job_id = body["data"]["job_id"].as_str().expect("job_id").to_string();
    poll_job_terminal(client, base, token, &job_id)
}

/// The discriminating proof that a successful `tune --apply` job reloads
/// `AppState.cfg` **in-memory**, not just on disk. Reading `config.toml`
/// off disk only proves the file write (any implementation gets that for
/// free); re-issuing the SAME query against the SAME still-running server
/// — no restart — and observing the ranking flip is the part unique to
/// `AppState::reload_cfg` (mirrors AC-16's cross-process rebuild-swap
/// discipline, adapted to an in-process reload).
#[test]
fn v1_tune_apply_reloads_cfg_in_memory_without_a_restart() {
    let home = TempDir::new().expect("home");
    let (golden, tagged) = tag_discriminated_fixture(&home);
    let config = home.path().join(".comemory").join("config.toml");

    let allowed = home.path().to_str().expect("utf8 path").to_string();
    let (base, token, _guard) = spawn_serve_with(
        &home,
        &["--allow-path", &allowed],
        &[
            ("COMEMORY_TUNE_MIN_GOLDEN", "2"),
            ("COMEMORY_DISABLE_ACCESS_TRACKING", "true"),
        ],
    );
    let client = reqwest::blocking::Client::new();

    let before = top_hit_id(&client, &base, &token, "postgres");
    assert_ne!(
        before, tagged,
        "with the zero-weight tags column, the tagged memory must not win yet"
    );

    let job = run_confirmed_tune_apply(&client, &base, &token, &golden);
    assert_eq!(job["status"], "done", "job body: {job}");
    assert_eq!(
        job["result"]["applied"],
        serde_json::json!(true),
        "the zero-weight tags column must be beatable by the default grid"
    );

    // Proof #1: the FILE was rewritten.
    let raw = std::fs::read_to_string(&config).expect("read applied config.toml");
    assert_ne!(
        raw.replace([' ', '\n'], ""),
        "[retrieval]bm25_weights=[1.0,0.0]".to_string(),
        "applied config.toml must have been rewritten"
    );

    // Proof #2 (the one unique to this step): the SAME still-running server
    // now ranks the tagged memory first for the same query, with no
    // restart — only possible if AppState.cfg was reloaded in-memory.
    let after = top_hit_id(&client, &base, &token, "postgres");
    assert_eq!(
        after, tagged,
        "AppState.cfg must have reloaded in-memory: a fresh search on the \
         same server must now rank the tagged memory first"
    );
}
