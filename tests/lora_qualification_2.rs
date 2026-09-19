#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The LoRA qualification harness end to end — part 2: three arms scored over
//! one captured candidate pool by real `python3` children speaking the real
//! #211 wire protocol, the operational cost the binary cannot see, and the
//! decision record that must never read `go` from evidence it does not have.
//!
//! The recipe half is `tests/lora_qualification.rs`; the shared corpus and
//! plumbing are `tests/common/lora_harness.rs`.
//!
//! The scorer here is the reference backend's deterministic `lexical-overlap`
//! mode, which measures no relevance quality at all and is refused a `go` by
//! name. So this suite proves the plumbing, the refusals and the decision
//! table, and says nothing about whether an adapter helps. That question needs
//! weights, and `integrations/training/run-training-tests.sh` is where it is
//! asked.

#[path = "common/lora_harness.rs"]
mod harness;

use harness::{
    backend_py, benchmark, build_set, export_filling, out_of, python, python_ok, qualify_py,
    read_json, repo_root, three_judged,
};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

/// The whole qualification pipeline, plus the outcomes it must refuse to reach.
#[test]
fn the_qualification_records_an_honest_outcome() {
    let home = three_judged();
    let out = out_of(&home, "dataset");
    export_filling(&home, &out, "0,0,1", &["holdout"]);
    let workspace = home.workspace().to_path_buf();
    let (set, provenance) = build_set(&workspace, &out, "1");
    let baseline = workspace.join("baseline.json");
    benchmark(&home, &set, &baseline, &[]);

    let mut score_files = Vec::new();
    for arm in ["base", "lora"] {
        let scores = workspace.join(format!("{arm}.scores.json"));
        let sidecar = workspace.join(format!("{arm}.scoring.json"));
        python_ok(&[
            &qualify_py(),
            "score",
            "--report",
            baseline.to_str().expect("utf8"),
            "--arm",
            arm,
            "--out",
            scores.to_str().expect("utf8"),
            "--sidecar",
            sidecar.to_str().expect("utf8"),
            "--model",
            "lexical-overlap@1",
            "--",
            "python3",
            &backend_py(),
            "score",
            "--scoring",
            "lexical-overlap",
        ]);
        let recorded = read_json(&sidecar);
        assert_eq!(recorded["sidecar_version"], 1);
        assert_eq!(recorded["failures"], 0);
        assert_eq!(recorded["fallback_rate"], 0.0);
        assert_eq!(recorded["ambiguous_refs"], 0);
        assert!(recorded["invocations"].as_i64().unwrap() >= 2);
        assert_eq!(
            recorded["invocations"].as_i64(),
            recorded["tasks"].as_i64(),
            "every task was scored"
        );
        assert!(recorded["candidates_scored"].as_i64().unwrap() > 0);
        let latency = &recorded["latency_ms"];
        assert!(latency["p50"].as_f64().unwrap() <= latency["p95"].as_f64().unwrap());
        assert!(latency["p95"].as_f64().unwrap() <= latency["max"].as_f64().unwrap());
        assert!(recorded["peak_child_rss"]["value"].as_i64().unwrap() > 0);
        assert!(recorded["peak_child_rss"]["unit"].is_string());
        assert!(recorded["peak_child_rss"]["platform"].is_string());
        score_files.push(scores);
    }

    let qualification = workspace.join("qualification.json");
    let borrowed: Vec<&std::path::Path> = score_files.iter().map(PathBuf::as_path).collect();
    benchmark(&home, &set, &qualification, &borrowed);
    let artifact = read_json(&qualification);
    let arms = artifact["arms"].as_array().expect("arms");
    assert_eq!(arms.len(), 3, "deterministic, base and lora: {artifact}");
    for arm in arms {
        assert!(
            arm["scored_fraction"].as_f64().unwrap() >= 1.0 - 1e-9,
            "every arm scored the whole pool: {arm}"
        );
    }

    let decision_path = workspace.join("decision.json");
    let markdown = workspace.join("decision.md");
    let printed = python_ok(&[
        &qualify_py(),
        "decide",
        "--report",
        qualification.to_str().expect("utf8"),
        "--provenance",
        provenance.to_str().expect("utf8"),
        "--scoring",
        &format!("base={}", workspace.join("base.scoring.json").display()),
        "--scoring",
        &format!("lora={}", workspace.join("lora.scoring.json").display()),
        "--out",
        decision_path.to_str().expect("utf8"),
        "--markdown",
        markdown.to_str().expect("utf8"),
    ]);
    assert!(printed.starts_with("insufficient-evidence"), "{printed}");

    let decision = read_json(&decision_path);
    assert_eq!(decision["outcome"], "insufficient-evidence");
    assert_ne!(decision["outcome"], "go");
    let named: Vec<String> = decision["reasons"]
        .as_array()
        .expect("reasons")
        .iter()
        .map(|r| r.as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        named.iter().any(|r| r.contains("scorers_are_neural")),
        "a deterministic non-neural scorer can never earn a go: {named:?}"
    );
    let status = |name: &str| -> String {
        decision["checks"]
            .as_array()
            .expect("checks")
            .iter()
            .chain(decision["budgets"].as_array().expect("budgets"))
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("no check named {name}: {decision}"))["status"]
            .as_str()
            .expect("status")
            .to_string()
    };
    assert_eq!(status("artifact_versions"), "pass");
    assert_eq!(status("no_stale_judgments"), "pass");
    assert_eq!(status("scored_the_whole_pool[lora]"), "pass");
    assert_eq!(status("scored_the_reported_run[base]"), "pass");
    assert_eq!(status("pinned_ranking_applied"), "pass");
    assert_eq!(status("fallback_rate"), "pass");
    assert_eq!(status("scorers_are_neural"), "fail");
    assert_eq!(
        status("adapter_provenance"),
        "skipped",
        "a check that could not run is never recorded as a pass"
    );
    assert!(decision["arms"]["lora"]["per_domain"].is_array());
    assert!(decision["scoring"]["lora"]["scoring_latency_ms"]["p95"].is_number());
    let rendered = std::fs::read_to_string(&markdown).expect("read the markdown");
    assert!(rendered.contains("**Outcome: `insufficient-evidence`**"));
    assert!(rendered.contains("skipped (did not run)"));
    assert!(rendered.contains("## Operational cost"));
}

#[test]
fn a_failing_scorer_and_a_moved_corpus_are_recorded_not_hidden() {
    let home = three_judged();
    let out = out_of(&home, "dataset");
    export_filling(&home, &out, "0,0,1", &["holdout"]);
    let workspace = home.workspace().to_path_buf();
    let (set, provenance) = build_set(&workspace, &out, "1");
    let baseline = workspace.join("baseline.json");
    benchmark(&home, &set, &baseline, &[]);

    // A scorer that refuses every request is the production fallback path:
    // comemory's own ranking stands, and the qualification records how often
    // that happened rather than quietly comparing a partial arm.
    let scores = workspace.join("broken.scores.json");
    let sidecar = workspace.join("broken.scoring.json");
    python_ok(&[
        &qualify_py(),
        "score",
        "--report",
        baseline.to_str().expect("utf8"),
        "--arm",
        "broken",
        "--out",
        scores.to_str().expect("utf8"),
        "--sidecar",
        sidecar.to_str().expect("utf8"),
        "--model",
        "lexical-overlap@1",
        "--",
        "python3",
        "-c",
        "raise SystemExit(65)",
    ]);
    let recorded = read_json(&sidecar);
    assert!(recorded["failures"].as_i64().unwrap() >= 2);
    assert_eq!(
        recorded["failures"].as_i64(),
        recorded["invocations"].as_i64()
    );
    assert_eq!(recorded["fallback_rate"], 1.0);
    assert_eq!(recorded["candidates_scored"], 0);
    let reasons = recorded["failure_reasons"].as_array().expect("reasons");
    assert_eq!(reasons[0]["kind"], "non_zero_exit");
    assert_eq!(reasons[0]["code"], 65);
    assert!(reasons[0]["task_id"].is_string());

    // A scores file that scored nothing still reorders nothing, so the arm
    // reproduces the baseline exactly. The harness reports that rather than
    // presenting it as a neutral model result.
    let qualification = workspace.join("qualification.json");
    benchmark(&home, &set, &qualification, &[&scores]);

    // And a corpus that moved between the two passes is caught by the digests
    // each sidecar copied from the artifact it actually scored.
    let drifted = workspace.join("drifted.scoring.json");
    let mut moved = read_json(&sidecar);
    moved["corpus_digest"] = Value::from("0".repeat(64));
    std::fs::write(&drifted, serde_json::to_string(&moved).expect("serialize")).expect("write");
    let decision_path = workspace.join("decision.json");
    let printed = python_ok(&[
        &qualify_py(),
        "decide",
        "--report",
        qualification.to_str().expect("utf8"),
        "--provenance",
        provenance.to_str().expect("utf8"),
        "--scoring",
        &format!("broken={}", drifted.display()),
        "--lora-arm",
        "broken",
        "--base-arm",
        "broken",
        "--out",
        decision_path.to_str().expect("utf8"),
    ]);
    assert!(printed.starts_with("insufficient-evidence"), "{printed}");
    let decision = read_json(&decision_path);
    let failed: Vec<String> = decision["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .filter(|c| c["status"] == "fail")
        .map(|c| c["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        failed
            .iter()
            .any(|n| n == "scored_the_reported_run[broken]"),
        "a corpus that moved between the passes is caught: {failed:?}"
    );
    let detail = decision["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .find(|c| c["name"] == "scored_the_reported_run[broken]")
        .expect("the check")["detail"]
        .as_str()
        .expect("detail")
        .to_string();
    assert!(
        detail.contains("000000000000"),
        "the record names both digests: {detail}"
    );
}

#[test]
fn the_offline_python_suite_runs_on_every_test_run() {
    let out = python(&[
        "-m",
        "unittest",
        "discover",
        "--start-directory",
        "integrations/training/tests",
        "--pattern",
        "test_offline_*.py",
        "--verbose",
    ]);
    let report = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the offline recipe suite failed:\n{report}"
    );
    assert!(
        report.contains("test_the_saved_shape_is_accepted"),
        "the adapter round trip ran: {report}"
    );
    assert!(
        report.contains("test_the_base_identity_is_imported_not_restated"),
        "the pin agreement ran: {report}"
    );
    assert!(
        report.contains("test_the_readable_allowlist_excludes_every_held_out_file"),
        "the split-boundary check ran: {report}"
    );
}

#[test]
fn the_opt_in_training_suite_refuses_to_report_a_pass_it_did_not_earn() {
    let out = Command::new("bash")
        .arg("integrations/training/run-training-tests.sh")
        .current_dir(repo_root())
        .output()
        .expect("run the opt-in script");
    let report = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        // Only reachable on a machine that installed the pinned libraries AND
        // fetched the pinned snapshot, which CI deliberately does not.
        assert!(
            report.contains("prerequisites satisfied"),
            "a successful run says it actually ran: {report}"
        );
        return;
    }
    assert_eq!(
        out.status.code(),
        Some(69),
        "a missing prerequisite exits EX_UNAVAILABLE: {report}"
    );
    assert!(
        report.contains("the training suite did NOT run"),
        "a skipped model suite says so out loud: {report}"
    );
    assert!(
        report.contains("pip install -r"),
        "and names the command that fixes it: {report}"
    );
}
