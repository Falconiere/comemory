#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The LoRA training recipe over a real `comemory export-dataset` directory —
//! part 1: what `comemory_train.py plan` resolves, what it refuses, and the
//! reviewed benchmark set `comemory_qualify.py build-set` generates from the
//! withheld holdout.
//!
//! The qualification half — scoring arms through real child processes and
//! recording the outcome — is `tests/lora_qualification_2.rs`. The corpus and
//! the `python3` plumbing both drive are `tests/common/lora_harness.rs`.
//!
//! `python3` is a prerequisite, and its absence FAILS rather than skips: a
//! skipped test that reads as a pass turns "we have not checked this" into "we
//! checked this and it was fine".

#[path = "common/lora_harness.rs"]
mod harness;

use harness::{
    build_set, export_filling, out_of, python, python_ok, read_json, three_judged, train_py,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[test]
fn plan_resolves_the_recipe_and_never_opens_the_holdout() {
    let home = three_judged();
    let out = out_of(&home, "dataset");
    export_filling(
        &home,
        &out,
        "0.34,0.33,0.33",
        &["train", "validation", "holdout"],
    );
    let directory = out.to_str().expect("utf8");

    let first = python_ok(&[&train_py(), "plan", "--dataset", directory, "--json"]);
    let plan: Value = serde_json::from_str(&first).expect("plan --json is JSON");

    assert_eq!(plan["recipe"]["lora"]["r"], 16);
    assert_eq!(plan["recipe"]["lora"]["modules_to_save"][0], "classifier");
    assert_eq!(plan["recipe"]["lora"]["task_type"], "SEQ_CLS");
    assert_eq!(plan["recipe"]["seed"], 20_260_918);
    assert_eq!(plan["recipe"]["precision"]["dtype"], "float32");
    assert_eq!(
        plan["recipe"]["selection"]["splits_consulted"][0],
        "validation"
    );
    assert_eq!(
        plan["recipe"]["base"]["model_revision"],
        "233902d25c440f23af6f7d6e94d2946bac0bee0a"
    );
    assert!(
        plan["dataset"]["dataset_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("d-")),
        "the plan carries the export's own dataset id: {plan}"
    );
    assert!(plan["dataset"]["rows_by_split"]["train"].as_i64().unwrap() > 0);
    assert!(
        plan["dataset"]["rows_by_split"]["validation"]
            .as_i64()
            .unwrap()
            > 0
    );
    assert!(
        plan["dataset"]["rows_by_split"].get("holdout").is_none(),
        "the trainer never counts held-out rows: {plan}"
    );
    assert_eq!(plan["dataset"]["holdout"]["read"], false);
    assert_eq!(
        plan["dataset"]["readable_files"].as_array().unwrap().len(),
        3
    );
    assert!(
        plan["dataset"]["holdout"]["sha256"]
            .as_str()
            .is_some_and(|d| d.len() == 64),
        "the withheld holdout's digest is recorded: {plan}"
    );

    // The proof that the allowlist is structural rather than conventional: the
    // held-out file is replaced with bytes that are not JSON, and the resolved
    // plan does not move. Anything that parsed it would fail here.
    std::fs::write(out.join("holdout.jsonl"), b"not json at all\n").expect("corrupt the holdout");
    let second = python_ok(&[&train_py(), "plan", "--dataset", directory, "--json"]);
    assert_eq!(
        first, second,
        "a corrupt holdout changed the plan, so something read it"
    );
}

#[test]
fn plan_refuses_a_tampered_or_leaking_export() {
    let home = three_judged();
    let out = out_of(&home, "dataset");
    export_filling(&home, &out, "0.5,0.5,0", &["train", "validation"]);
    let directory = out.to_str().expect("utf8").to_string();

    let good = python(&[&train_py(), "plan", "--dataset", &directory]);
    assert!(good.status.success(), "the untampered export plans cleanly");

    let train = out.join("train.jsonl");
    let original = std::fs::read_to_string(&train).expect("read train.jsonl");
    std::fs::write(&train, original.replacen("\"query\"", "\"QUERY\"", 1)).expect("tamper");
    let tampered = python(&[&train_py(), "plan", "--dataset", &directory]);
    assert_eq!(tampered.status.code(), Some(65));
    let message = String::from_utf8_lossy(&tampered.stderr);
    assert!(
        message.contains("train.jsonl does not match the manifest"),
        "a changed byte is caught by the manifest digest: {message}"
    );
    std::fs::write(&train, &original).expect("restore");

    let manifest_path = out.join("manifest.json");
    let mut manifest = read_json(&manifest_path);
    manifest["manifest_version"] = Value::from(2);
    std::fs::write(
        &manifest_path,
        serde_json::to_string(&manifest).expect("serialize"),
    )
    .expect("write manifest");
    let future = python(&[&train_py(), "plan", "--dataset", &directory]);
    assert_eq!(future.status.code(), Some(65));
    let message = String::from_utf8_lossy(&future.stderr);
    assert!(
        message.contains("manifest_version 2") && message.contains("must refuse, not guess"),
        "an unknown contract version is refused, not guessed at: {message}"
    );
}

#[test]
fn a_group_reaching_both_readable_splits_is_refused() {
    let home = three_judged();
    let out = out_of(&home, "dataset");
    export_filling(&home, &out, "0.5,0.5,0", &["train", "validation"]);
    let directory = out.to_str().expect("utf8").to_string();

    let train_group = {
        let first = std::fs::read_to_string(out.join("train.jsonl")).expect("read train");
        let row: Value = serde_json::from_str(first.lines().next().expect("a train row"))
            .expect("a JSON record");
        row["group_id"].as_str().expect("group_id").to_string()
    };
    let validation = out.join("validation.jsonl");
    let body = std::fs::read_to_string(&validation).expect("read validation");
    let leaked: String = body
        .lines()
        .map(|line| {
            let mut row: Value = serde_json::from_str(line).expect("a JSON record");
            row["group_id"] = Value::from(train_group.clone());
            serde_json::to_string(&row).expect("serialize") + "\n"
        })
        .collect();
    std::fs::write(&validation, &leaked).expect("write the leaking split");
    // The manifest digest would otherwise catch this first; re-stamping it is
    // what makes the leakage check itself the thing under test.
    let manifest_path = out.join("manifest.json");
    let mut manifest = read_json(&manifest_path);
    let digest = Sha256::digest(leaked.as_bytes())
        .iter()
        .fold(String::new(), |mut acc, byte| {
            use std::fmt::Write;
            let _ = write!(acc, "{byte:02x}");
            acc
        });
    for entry in manifest["files"].as_array_mut().expect("files") {
        if entry["path"] == "validation.jsonl" {
            entry["sha256"] = Value::from(digest.clone());
        }
    }
    std::fs::write(
        &manifest_path,
        serde_json::to_string(&manifest).expect("serialize"),
    )
    .expect("write manifest");

    let refused = python(&[&train_py(), "plan", "--dataset", &directory]);
    assert_eq!(refused.status.code(), Some(65));
    let message = String::from_utf8_lossy(&refused.stderr);
    assert!(
        message.contains(&train_group) && message.contains("both train.jsonl and validation.jsonl"),
        "a group on both sides of the split boundary is refused by name: {message}"
    );
}

#[test]
fn build_set_produces_a_reviewed_set_the_binary_runs_and_reproduces() {
    let home = three_judged();
    let out = out_of(&home, "dataset");
    let manifest = export_filling(&home, &out, "0,0,1", &["holdout"]);
    let workspace = home.workspace().to_path_buf();
    let (set, provenance) = build_set(&workspace, &out, "1");

    let recorded = read_json(&provenance);
    assert_eq!(recorded["dataset_id"], manifest["dataset_id"]);
    assert_eq!(recorded["snapshot_digest"], manifest["snapshot_digest"]);
    assert_eq!(recorded["set_ranking"]["decay"], 0.0);
    assert_eq!(recorded["budgets"]["min_tasks"], 1);
    assert!(recorded["tasks"].as_i64().unwrap() >= 2);
    assert!(recorded["judgments"].as_i64().unwrap() >= 3);
    assert_eq!(recorded["pin_content_version"], true);
    assert_eq!(
        recorded["holdout"]["sha256"].as_str().map(str::len),
        Some(64)
    );

    // The generated set is a real benchmark set: the real binary loads it,
    // re-runs retrieval for every task and scores the deterministic baseline.
    let report = workspace.join("baseline.json");
    let tty = home.run_ok(&[
        "benchmark",
        "--set",
        set.to_str().expect("utf8"),
        "--report",
        report.to_str().expect("utf8"),
    ]);
    assert!(
        tty.contains("deterministic [Baseline]"),
        "the generated set scored a baseline arm: {tty}"
    );
    let artifact = read_json(&report);
    assert_eq!(artifact["artifact_version"], 1);
    assert_eq!(
        artifact["set_size"]["tasks"].as_i64(),
        recorded["tasks"].as_i64()
    );
    assert_eq!(
        artifact["set_size"]["judgments"].as_i64(),
        recorded["judgments"].as_i64()
    );
    assert_eq!(artifact["decay_frozen"], true);
    let stale: i64 = artifact["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .map(|t| t["judgments_stale"].as_i64().unwrap_or(0))
        .sum();
    assert_eq!(stale, 0, "every pinned content version still holds");

    // A qualification whose own inputs are not reproducible cannot support a
    // reproducible verdict, so two runs must write the same bytes.
    let twice = workspace.join("twice");
    std::fs::create_dir_all(&twice).expect("create the second output directory");
    let (again, again_provenance) = build_set(&twice, &out, "1");
    assert_eq!(
        std::fs::read(&set).expect("read set"),
        std::fs::read(&again).expect("read set again"),
        "build-set is not byte-reproducible over one export"
    );
    assert_eq!(
        std::fs::read(&provenance).expect("read provenance"),
        std::fs::read(&again_provenance).expect("read provenance again")
    );
}
