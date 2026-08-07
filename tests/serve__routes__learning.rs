//! `POST /api/v1/eval` end-to-end (golden containment, AC-7; survives
//! `--read-only`, AC-4) plus the `tune`/`bandit` confirm-when-apply shape
//! (`apply:false` needs no confirm; `apply:true` without confirm is `400`
//! and creates no job; `bandit --apply` refused when `[bandit] enabled =
//! false`). Read-only-outranks-confirm (AC-19) and the `AppState.cfg`
//! reload proof live in `tests/serve__routes__learning_2.rs`.

#[path = "common/serve_learning_support.rs"]
mod support;

use serde_json::Value;
use support::{jobs_total, poll_job_terminal, post, save, spawn_serve, spawn_serve_with};
use tempfile::TempDir;

// ── POST /api/v1/eval ───────────────────────────────────────────────────

#[test]
fn v1_eval_happy_path_reaches_done_with_a_real_report() {
    let home = TempDir::new().expect("home");
    let id = save(&home, "postgres advisory lock migration ordering", &[]);
    let golden = home.path().join("golden.yaml");
    std::fs::write(
        &golden,
        format!("- query: postgres advisory lock migration ordering\n  relevant: [{id}]\n"),
    )
    .expect("write golden");
    let allowed = home.path().to_str().expect("utf8 path").to_string();
    let (base, token, _guard) = spawn_serve(&home, &["--allow-path", &allowed]);
    let client = reqwest::blocking::Client::new();

    let res = post(
        &client,
        &base,
        &token,
        "eval",
        &serde_json::json!({
            "golden": golden.to_str().expect("utf8 path"),
            "golden_only": true,
            "k": 1,
        }),
    );
    assert_eq!(res.status().as_u16(), 202);
    let body: Value = res.json().expect("json");
    let job_id = body["data"]["job_id"].as_str().expect("job_id").to_string();

    let job = poll_job_terminal(&client, &base, &token, &job_id);
    assert_eq!(job["status"], "done", "job body: {job}");
    assert_eq!(job["result"]["queries"].as_u64(), Some(1));
    assert!(job["result"]["recall_at_k"].is_number());
}

#[test]
fn v1_eval_survives_read_only_ac4() {
    let home = TempDir::new().expect("home");
    let id = save(&home, "tokio runtime shutdown sequencing bug", &[]);
    let golden = home.path().join("golden.yaml");
    std::fs::write(
        &golden,
        format!("- query: tokio runtime shutdown sequencing bug\n  relevant: [{id}]\n"),
    )
    .expect("write golden");
    let allowed = home.path().to_str().expect("utf8 path").to_string();
    let (base, token, _guard) = spawn_serve(&home, &["--read-only", "--allow-path", &allowed]);
    let client = reqwest::blocking::Client::new();

    let res = post(
        &client,
        &base,
        &token,
        "eval",
        &serde_json::json!({
            "golden": golden.to_str().expect("utf8 path"),
            "golden_only": true,
        }),
    );
    assert_eq!(
        res.status().as_u16(),
        202,
        "POST /eval must stay functional on a read-only server (AC-4)"
    );
    let body: Value = res.json().expect("json");
    let job_id = body["data"]["job_id"].as_str().expect("job_id").to_string();
    let job = poll_job_terminal(&client, &base, &token, &job_id);
    assert_eq!(job["status"], "done", "job body: {job}");
}

#[test]
fn v1_eval_golden_outside_every_allowed_root_is_403_and_creates_no_job() {
    let home = TempDir::new().expect("home");
    let outside = TempDir::new().expect("outside dir");
    let golden = outside.path().join("golden.yaml");
    std::fs::write(&golden, "- query: q\n  relevant: [aaaaaaaa]\n").expect("write golden");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();
    let before = jobs_total(&client, &base, &token);

    let res = post(
        &client,
        &base,
        &token,
        "eval",
        &serde_json::json!({ "golden": golden.to_str().expect("utf8 path") }),
    );
    assert_eq!(res.status().as_u16(), 403);
    let body: Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "forbidden");
    assert_eq!(jobs_total(&client, &base, &token), before);
}

#[test]
fn v1_eval_golden_nonexistent_is_400_and_creates_no_job() {
    let home = TempDir::new().expect("home");
    let allowed = home.path().to_str().expect("utf8 path").to_string();
    let (base, token, _guard) = spawn_serve(&home, &["--allow-path", &allowed]);
    let client = reqwest::blocking::Client::new();
    let before = jobs_total(&client, &base, &token);

    let missing = home.path().join("no-such-golden.yaml");
    let res = post(
        &client,
        &base,
        &token,
        "eval",
        &serde_json::json!({ "golden": missing.to_str().expect("utf8 path") }),
    );
    assert_eq!(res.status().as_u16(), 400);
    let body: Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "bad_request");
    assert_eq!(jobs_total(&client, &base, &token), before);
}

/// TOCTOU regression: `req.golden` is overwritten with the SAME
/// canonicalized path `contain_abs` just verified before the job body reads
/// it (§Security "Path containment") — a golden file reached only through a
/// symlink inside the allowed root must still be found, read, and scored.
#[test]
fn v1_eval_golden_through_a_symlink_inside_the_allowed_root_still_runs() {
    let home = TempDir::new().expect("home");
    let id = save(&home, "kafka consumer group rebalance strategy", &[]);
    let real_golden = home.path().join("real-golden.yaml");
    std::fs::write(
        &real_golden,
        format!("- query: kafka consumer group rebalance strategy\n  relevant: [{id}]\n"),
    )
    .expect("write golden");
    let golden_link = home.path().join("golden-link.yaml");
    std::os::unix::fs::symlink(&real_golden, &golden_link).expect("create symlink");
    let allowed = home.path().to_str().expect("utf8 path").to_string();
    let (base, token, _guard) = spawn_serve(&home, &["--allow-path", &allowed]);
    let client = reqwest::blocking::Client::new();

    let res = post(
        &client,
        &base,
        &token,
        "eval",
        &serde_json::json!({
            "golden": golden_link.to_str().expect("utf8 path"),
            "golden_only": true,
            "k": 1,
        }),
    );
    assert_eq!(res.status().as_u16(), 202);
    let body: Value = res.json().expect("json");
    let job_id = body["data"]["job_id"].as_str().expect("job_id").to_string();

    let job = poll_job_terminal(&client, &base, &token, &job_id);
    assert_eq!(job["status"], "done", "job body: {job}");
    assert_eq!(job["result"]["queries"].as_u64(), Some(1));
}

// ── POST /api/v1/tune, POST /api/v1/bandit — confirm-when-apply shape ──

#[test]
fn v1_tune_apply_false_needs_no_confirm_and_reaches_done() {
    let home = TempDir::new().expect("home");
    let id = save(&home, "clap derive global flag placement", &[]);
    let golden = home.path().join("golden.yaml");
    std::fs::write(
        &golden,
        format!("- query: clap derive global flag placement\n  relevant: [{id}]\n"),
    )
    .expect("write golden");
    let allowed = home.path().to_str().expect("utf8 path").to_string();
    let (base, token, _guard) = spawn_serve_with(
        &home,
        &["--allow-path", &allowed],
        &[("COMEMORY_TUNE_MIN_GOLDEN", "1")],
    );
    let client = reqwest::blocking::Client::new();

    let res = post(
        &client,
        &base,
        &token,
        "tune",
        &serde_json::json!({
            "golden": golden.to_str().expect("utf8 path"),
            "golden_only": true,
            "k": 1,
            "apply": false,
        }),
    );
    assert_eq!(
        res.status().as_u16(),
        202,
        "apply:false must never require confirm"
    );
    let body: Value = res.json().expect("json");
    let job_id = body["data"]["job_id"].as_str().expect("job_id").to_string();
    let job = poll_job_terminal(&client, &base, &token, &job_id);
    assert_eq!(job["status"], "done", "job body: {job}");
    assert_eq!(job["result"]["applied"], serde_json::json!(false));
}

#[test]
fn v1_tune_apply_true_without_confirm_is_400_and_creates_no_job() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();
    let before = jobs_total(&client, &base, &token);

    let res = post(
        &client,
        &base,
        &token,
        "tune",
        &serde_json::json!({ "apply": true }),
    );
    assert_eq!(res.status().as_u16(), 400);
    let body: Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "confirmation_required");
    assert_eq!(jobs_total(&client, &base, &token), before);
}

#[test]
fn v1_bandit_apply_false_needs_no_confirm_and_reaches_done() {
    let home = TempDir::new().expect("home");
    let id = save(&home, "redis cache eviction policy tuning", &[]);
    let golden = home.path().join("golden.yaml");
    std::fs::write(
        &golden,
        format!("- query: redis cache eviction policy tuning\n  relevant: [{id}]\n"),
    )
    .expect("write golden");
    let allowed = home.path().to_str().expect("utf8 path").to_string();
    let (base, token, _guard) = spawn_serve_with(
        &home,
        &["--allow-path", &allowed],
        &[("COMEMORY_TUNE_MIN_GOLDEN", "1")],
    );
    let client = reqwest::blocking::Client::new();

    let res = post(
        &client,
        &base,
        &token,
        "bandit",
        &serde_json::json!({
            "golden": golden.to_str().expect("utf8 path"),
            "golden_only": true,
            "k": 1,
            "apply": false,
        }),
    );
    assert_eq!(res.status().as_u16(), 202);
    let body: Value = res.json().expect("json");
    let job_id = body["data"]["job_id"].as_str().expect("job_id").to_string();
    let job = poll_job_terminal(&client, &base, &token, &job_id);
    assert_eq!(job["status"], "done", "job body: {job}");
    assert_eq!(job["result"]["applied"], serde_json::json!(false));
}

#[test]
fn v1_bandit_apply_true_without_confirm_is_400_and_creates_no_job() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();
    let before = jobs_total(&client, &base, &token);

    let res = post(
        &client,
        &base,
        &token,
        "bandit",
        &serde_json::json!({ "apply": true }),
    );
    assert_eq!(res.status().as_u16(), 400);
    let body: Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "confirmation_required");
    assert_eq!(jobs_total(&client, &base, &token), before);
}

#[test]
fn v1_bandit_apply_refused_when_disabled_in_config() {
    let home = TempDir::new().expect("home");
    let id = save(&home, "graphql federation gateway timeout", &[]);
    let golden = home.path().join("golden.yaml");
    std::fs::write(
        &golden,
        format!("- query: graphql federation gateway timeout\n  relevant: [{id}]\n"),
    )
    .expect("write golden");
    let config = home.path().join(".comemory").join("config.toml");
    std::fs::write(&config, "[bandit]\nenabled = false\n").expect("write config.toml");
    let allowed = home.path().to_str().expect("utf8 path").to_string();
    let (base, token, _guard) = spawn_serve_with(
        &home,
        &["--allow-path", &allowed],
        &[("COMEMORY_TUNE_MIN_GOLDEN", "1")],
    );
    let client = reqwest::blocking::Client::new();

    let res = post(
        &client,
        &base,
        &token,
        "bandit",
        &serde_json::json!({
            "golden": golden.to_str().expect("utf8 path"),
            "golden_only": true,
            "k": 1,
            "apply": true,
            "confirm": true,
        }),
    );
    assert_eq!(res.status().as_u16(), 202, "confirmed, so a job is created");
    let body: Value = res.json().expect("json");
    let job_id = body["data"]["job_id"].as_str().expect("job_id").to_string();
    let job = poll_job_terminal(&client, &base, &token, &job_id);
    assert_eq!(job["status"], "error", "job body: {job}");
    assert_eq!(job["error"]["code"], "config");
}
