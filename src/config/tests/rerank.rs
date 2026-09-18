#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirrors `src/config/rerank.rs` — the `[rerank]` file overlay and every
//! invariant, loaded through the real `Config::with_file` path over a real
//! `config.toml` on disk.

use std::path::PathBuf;

use comemory::config::Config;
use comemory::config::rerank::MAX_REQUEST_BYTES;

/// Write `body` as a `config.toml` in a fresh temp dir and return its path,
/// keeping the dir alive for the caller.
fn config_file(body: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, body).expect("write config.toml");
    (dir, path)
}

/// Load `body` through the real file overlay.
fn load(body: &str) -> Result<Config, comemory::errors::Error> {
    let (_dir, path) = config_file(body);
    Config::defaults().with_file(&path)
}

#[test]
fn defaults_are_off_with_no_command_and_no_model() {
    let c = Config::defaults();
    assert!(
        !c.rerank.enabled,
        "the learned stage must be off by default"
    );
    assert!(c.rerank.command.is_empty());
    assert!(c.rerank.model.is_empty());
    assert_eq!(c.rerank.adapter(), None);
    assert_eq!(c.rerank.prefix, 50);
    assert_eq!(c.rerank.timeout_ms, 20_000);
    assert_eq!(c.rerank.max_candidate_text_bytes, 4096);
}

#[test]
fn an_absent_section_leaves_every_default() {
    let c = load("[retrieval]\ntop_k = 7\n").expect("load");
    assert!(!c.rerank.enabled);
    assert_eq!(c.rerank.prefix, 50);
}

#[test]
fn the_file_overlay_sets_every_key() {
    let c = load(
        r#"
[rerank]
enabled = true
command = ["python3", "score.py", "--scoring", "lexical-overlap"]
model = "lexical-overlap@1"
adapter = "lora-v1"
prefix = 25
timeout_ms = 1500
max_candidate_text_bytes = 2048
"#,
    )
    .expect("load");
    assert!(c.rerank.enabled);
    assert_eq!(
        c.rerank.command,
        vec!["python3", "score.py", "--scoring", "lexical-overlap"]
    );
    assert_eq!(c.rerank.model, "lexical-overlap@1");
    assert_eq!(c.rerank.adapter(), Some("lora-v1"));
    assert_eq!(c.rerank.prefix, 25);
    assert_eq!(c.rerank.timeout_ms, 1500);
    assert_eq!(c.rerank.max_candidate_text_bytes, 2048);
}

#[test]
fn a_blank_adapter_reads_as_the_base_model() {
    let c = load(
        r#"
[rerank]
enabled = true
command = ["true"]
model = "m@1"
adapter = "   "
"#,
    )
    .expect("load");
    assert_eq!(
        c.rerank.adapter(),
        None,
        "a blank adapter is the base model, never an adapter named \"\""
    );
}

#[test]
fn an_unknown_key_is_refused() {
    let err = load("[rerank]\nenabled = true\nscoring = \"cross-encoder\"\n")
        .expect_err("unknown [rerank] key must fail the load");
    assert!(
        err.to_string().contains("scoring"),
        "the message must name the offending key, got: {err}"
    );
}

#[test]
fn every_bound_is_enforced_and_names_its_key() {
    for (body, needle) in [
        ("[rerank]\nprefix = 0\n", "rerank.prefix=0"),
        ("[rerank]\ntimeout_ms = 0\n", "rerank.timeout_ms=0"),
        (
            "[rerank]\nmax_candidate_text_bytes = 0\n",
            "rerank.max_candidate_text_bytes=0",
        ),
    ] {
        let err = load(body).expect_err("bound must be enforced");
        let text = err.to_string();
        assert!(
            text.contains(needle),
            "message must contain {needle:?}, got: {text}"
        );
        assert!(
            text.contains("file-only [rerank] key"),
            "message must say the section is file-only, got: {text}"
        );
    }
}

#[test]
fn bounds_are_enforced_even_while_the_stage_is_disabled() {
    let err = load("[rerank]\nenabled = false\nprefix = 0\n")
        .expect_err("a typo must be reported when it is written");
    assert!(err.to_string().contains("rerank.prefix=0"));
}

#[test]
fn enabling_without_a_command_or_a_model_is_refused() {
    let no_command = load("[rerank]\nenabled = true\nmodel = \"m@1\"\ncommand = []\n")
        .expect_err("an enabled stage needs a program");
    assert!(
        no_command.to_string().contains("rerank.command"),
        "got: {no_command}"
    );
    let blank_command = load("[rerank]\nenabled = true\nmodel = \"m@1\"\ncommand = [\"  \"]\n")
        .expect_err("a blank program is no program");
    assert!(blank_command.to_string().contains("rerank.command"));
    // The FIRST entry is the program: a blank one must fail even when a later
    // argument is non-blank, rather than deferring to a spawn failure.
    let blank_program =
        load("[rerank]\nenabled = true\nmodel = \"m@1\"\ncommand = [\"\", \"score.py\"]\n")
            .expect_err("a blank program with arguments is still no program");
    assert!(
        blank_program.to_string().contains("rerank.command"),
        "got: {blank_program}"
    );
    let no_model = load("[rerank]\nenabled = true\ncommand = [\"true\"]\nmodel = \"\"\n")
        .expect_err("an enabled stage needs a model identity");
    assert!(
        no_model.to_string().contains("rerank.model"),
        "got: {no_model}"
    );
}

#[test]
fn a_disabled_stage_may_leave_the_identity_keys_empty() {
    load("[rerank]\nenabled = false\n").expect("a disabled stage needs no identity");
}

#[test]
fn the_request_budget_bounds_prefix_times_text() {
    let over = MAX_REQUEST_BYTES / 4096 + 1;
    let err = load(&format!(
        "[rerank]\nprefix = {over}\nmax_candidate_text_bytes = 4096\n"
    ))
    .expect_err("the product must be bounded");
    let text = err.to_string();
    assert!(text.contains("rerank.prefix"), "got: {text}");
    assert!(
        text.contains(&MAX_REQUEST_BYTES.to_string()),
        "the message must name the byte limit, got: {text}"
    );
    let exactly = MAX_REQUEST_BYTES / 4096;
    load(&format!(
        "[rerank]\nprefix = {exactly}\nmax_candidate_text_bytes = 4096\n"
    ))
    .expect("exactly at the limit is valid");
}

#[test]
fn the_declared_request_ceiling_matches_the_runners_own() {
    assert_eq!(
        MAX_REQUEST_BYTES,
        comemory::utilities::rerank_runner::RerankLimits::default().max_request_bytes,
        "config restates the runner's ceiling; the two must never drift"
    );
}

#[test]
fn the_section_round_trips_through_toml() {
    let mut c = Config::defaults();
    c.rerank.enabled = true;
    c.rerank.command = vec!["python3".into(), "score.py".into()];
    c.rerank.model = "m@1".into();
    let text = toml::to_string(&c).expect("serialize");
    let back = toml::from_str::<Config>(&text).expect("deserialize");
    assert!(back.rerank.enabled);
    assert_eq!(back.rerank.command, c.rerank.command);
}
