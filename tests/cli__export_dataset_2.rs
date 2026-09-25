#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory export-dataset` driven through the real binary — part 2: that the
//! qualification split is unreachable from a training export, that selection
//! runs inside a split, that a repeated export is byte-identical, that no
//! display field becomes a training key, and what the manifest reports.
//!
//! The record-shape half is `tests/cli__export_dataset.rs`.

#[path = "common/export_dataset_corpus.rs"]
mod fixture;

use fixture::corpus::refs;
use fixture::{ISOLATED_QUERY, collect_keys, export, field, out_of, prepared, records};
use serde_json::Value;

#[test]
fn the_holdout_split_is_unreadable_from_a_training_export() {
    let p = prepared();
    let training = out_of(&p.home, "training");
    let planted = training.join("holdout.jsonl");
    std::fs::create_dir_all(&training).expect("create out");
    std::fs::write(&planted, b"{\"leaked\":true}\n").expect("plant a stale holdout");

    let withheld = export(
        &p.home,
        &training,
        &["--holdout-repo", "demo", "--include-unjudged"],
    );

    assert!(
        !planted.exists(),
        "a qualification file an earlier run wrote must not survive a withholding run"
    );
    assert_eq!(withheld["split"]["holdout_repo"], "demo");
    let groups = withheld["split"]["groups"].as_u64().expect("group count");
    assert!(
        groups >= 2,
        "this corpus must form more than one component, or every assertion below is vacuous"
    );
    let withheld_entry = withheld["withheld"]
        .as_array()
        .expect("withheld list")
        .iter()
        .find(|f| f["path"] == "holdout.jsonl")
        .expect("the holdout file is reported as withheld");
    let reserved_rows = withheld_entry["rows"].as_u64().expect("withheld rows");
    assert!(reserved_rows > 0, "the reserved repo must hold rows");

    let released = out_of(&p.home, "qualify");
    export(
        &p.home,
        &released,
        &[
            "--holdout-repo",
            "demo",
            "--include-unjudged",
            "--include-holdout",
        ],
    );
    let holdout = records(&released.join("holdout.jsonl"));
    assert_eq!(holdout.len() as u64, reserved_rows);

    let mut trainable = records(&training.join("train.jsonl"));
    trainable.extend(records(&training.join("validation.jsonl")));
    assert!(
        !trainable.is_empty(),
        "the training side must hold rows, or disjointness proves nothing"
    );
    for key in ["candidate_ref", "text_sha256", "query", "group_id"] {
        let reachable = field(&trainable, key);
        for value in field(&holdout, key) {
            assert!(
                !reachable.contains(&value),
                "`{key}` value `{value}` reached a training file from the qualification split"
            );
        }
    }
}

#[test]
fn negative_selection_runs_inside_a_split() {
    let p = prepared();
    // A second reviewed negative on the same query, so the cap of one has
    // something to drop. Without it the dedup assertion below would hold
    // whether or not any capping happened.
    let extra: Vec<String> = refs(&p.home.db(), &p.first)
        .into_iter()
        .filter(|r| *r != p.memory_ref && *r != p.code_ref && *r != p.document_ref)
        .take(1)
        .collect();
    assert_eq!(
        extra.len(),
        1,
        "the captured pool must hold a spare candidate"
    );
    let mut verdicts = vec!["judge".to_string(), p.first.clone()];
    for reference in &extra {
        verdicts.push("--ref".to_string());
        verdicts.push(format!("{reference}=0"));
    }
    p.home
        .run_ok(&verdicts.iter().map(String::as_str).collect::<Vec<_>>());

    let capped = [
        "--holdout-repo",
        "demo",
        "--max-negatives-per-query",
        "1",
        "--include-unjudged",
    ];
    let withholding = out_of(&p.home, "withholding");
    let releasing = out_of(&p.home, "releasing");

    let withheld_manifest = export(&p.home, &withholding, &capped);
    assert!(
        withheld_manifest["counts"]["negatives_capped"]
            .as_u64()
            .expect("the cap report")
            >= 1,
        "the cap must have dropped something, or this test measures nothing"
    );
    let mut released_args = capped.to_vec();
    released_args.push("--include-holdout");
    export(&p.home, &releasing, &released_args);

    let trainable: usize = ["train.jsonl", "validation.jsonl"]
        .iter()
        .map(|name| records(&withholding.join(name)).len())
        .sum();
    assert!(
        trainable > 0,
        "the training side must hold rows, or the byte comparison below is vacuous"
    );
    for name in ["train.jsonl", "validation.jsonl"] {
        assert_eq!(
            std::fs::read(withholding.join(name)).expect("read"),
            std::fs::read(releasing.join(name)).expect("read"),
            "`{name}` must not change when the qualification split is released, so the \
             selection that produced it could not have read a holdout row"
        );
    }

    let mut everything = records(&releasing.join("train.jsonl"));
    everything.extend(records(&releasing.join("validation.jsonl")));
    everything.extend(records(&releasing.join("holdout.jsonl")));
    let mut negatives: Vec<String> = everything
        .iter()
        .filter(|r| r["label"]["relevance"].as_i64() == Some(0))
        .filter_map(|r| r["query_group"].as_str().map(str::to_string))
        .collect();
    let before = negatives.len();
    negatives.sort();
    negatives.dedup();
    assert_eq!(
        before,
        negatives.len(),
        "with a cap of one, no query group may contribute two reviewed negatives"
    );
}

#[test]
fn two_exports_of_one_snapshot_are_byte_identical() {
    let p = prepared();
    let out = out_of(&p.home, "dataset");

    let first = export(&p.home, &out, &["--include-unjudged"]);
    let bytes: Vec<Vec<u8>> = ["train.jsonl", "validation.jsonl", "manifest.json"]
        .iter()
        .map(|name| std::fs::read(out.join(name)).expect("read"))
        .collect();

    let second = export(&p.home, &out, &["--include-unjudged"]);

    for (name, before) in ["train.jsonl", "validation.jsonl", "manifest.json"]
        .iter()
        .zip(&bytes)
    {
        assert_eq!(
            *before,
            std::fs::read(out.join(name)).expect("read"),
            "`{name}` must be byte-identical across two exports of one snapshot"
        );
    }
    assert_eq!(first["dataset_id"], second["dataset_id"]);
    assert_eq!(first["snapshot_digest"], second["snapshot_digest"]);
    assert!(
        first["dataset_id"]
            .as_str()
            .expect("dataset id")
            .starts_with("d-")
    );

    // A further capture changes the snapshot, and therefore the id.
    p.home.capture_json(&["find", ISOLATED_QUERY, "--k", "5"]);
    let third = export(&p.home, &out, &["--include-unjudged"]);
    assert_ne!(first["snapshot_digest"], third["snapshot_digest"]);
    assert_ne!(first["dataset_id"], third["dataset_id"]);
}

#[test]
fn no_display_field_reaches_a_training_record() {
    let p = prepared();
    let out = out_of(&p.home, "dataset");
    export(&p.home, &out, &["--split", "1,0,0", "--include-unjudged"]);

    // A scan for the title STRING would prove nothing here: a memory's
    // headline is the first line of the body a record legitimately carries,
    // and a code candidate's headline is its repo-relative path, which is part
    // of its identity. What is checkable is that no locator VALUE and no
    // locator FIELD rides along, and both are read back out of the database
    // rather than hard-coded.
    let conn = p.home.db();
    let mut stmt = conn
        .prepare("SELECT locator_json FROM candidate_observations")
        .expect("prepare");
    let locators: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .expect("query")
        .filter_map(std::result::Result::ok)
        .filter(|body| {
            serde_json::from_str::<Value>(body)
                .is_ok_and(|v| v["title"].as_str().is_some_and(|t| !t.is_empty()))
        })
        .collect();
    assert!(
        !locators.is_empty(),
        "the captured pool must carry locators, or this proves nothing"
    );

    for name in ["train.jsonl", "validation.jsonl", "manifest.json"] {
        let body = std::fs::read_to_string(out.join(name)).expect("read");
        for locator in &locators {
            assert!(
                !body.contains(locator.as_str()),
                "a stored locator reached `{name}`; the locator is display-only by contract"
            );
        }
    }

    // The structural statement: no record carries a `CandidateLocator` field at
    // any depth, so re-adding one fails here even when its value coincides with
    // something a record already legitimately holds.
    let mut keys = Vec::new();
    for record in records(&out.join("train.jsonl")) {
        collect_keys(&record, &mut keys);
    }
    assert!(!keys.is_empty());
    for banned in [
        "title",
        "heading_path",
        "symbol_id",
        "line_range",
        "locator",
    ] {
        assert!(
            !keys.iter().any(|k| k == banned),
            "a record must carry no `{banned}`: the locator is display-only by contract"
        );
    }
    assert!(
        keys.iter().any(|k| k == "candidate_ref"),
        "the key scan must have seen the real records"
    );
}

#[test]
fn the_manifest_reports_the_filtering_configuration_and_the_split_plan() {
    let p = prepared();
    let out = out_of(&p.home, "dataset");

    let manifest = export(
        &p.home,
        &out,
        &[
            "--domain",
            "memory",
            "--domain",
            "code",
            "--split",
            "0.5,0.25,0.25",
            "--split-seed",
            "pinned",
        ],
    );

    assert_eq!(manifest["manifest_version"], 1);
    assert_eq!(manifest["record_version"], 1);
    assert_eq!(manifest["filters"]["provenance"], "manual");
    assert_eq!(manifest["filters"]["include_unjudged"], false);
    assert_eq!(manifest["filters"]["include_holdout"], false);
    assert_eq!(
        manifest["filters"]["domains"],
        serde_json::json!(["memory", "code"])
    );
    assert_eq!(manifest["split"]["policy"], "grouped-hash");
    assert_eq!(manifest["split"]["seed"], "pinned");
    assert_eq!(manifest["split"]["ratios"]["train"], 0.5);
    assert!(manifest["split"]["assignments"].is_array());
    assert!(manifest["counts"]["candidates_filtered_by_domain"].as_u64() >= Some(1));
    assert!(manifest["by_label"].is_array());
    assert_eq!(
        manifest["schema_version"], "28",
        "the manifest pins the schema the observations were captured under"
    );
    let on_disk: Value =
        serde_json::from_slice(&std::fs::read(out.join("manifest.json")).expect("read"))
            .expect("the written manifest is JSON");
    assert_eq!(on_disk["dataset_id"], manifest["dataset_id"]);

    // The window really narrows: nothing was captured in 2020, and everything
    // was captured before tomorrow.
    let empty = export(
        &p.home,
        &out_of(&p.home, "empty"),
        &["--since", "2030-01-01", "--until", "2030-01-02"],
    );
    assert_eq!(empty["counts"]["observations_scanned"], 0);
    assert_eq!(empty["filters"]["since"], "2030-01-01T00:00:00.000000000Z");
    let everything = export(
        &p.home,
        &out_of(&p.home, "wide"),
        &["--since", "2000-01-01"],
    );
    assert!(
        everything["counts"]["observations_scanned"]
            .as_u64()
            .expect("scanned")
            >= 2,
        "both capturing runs fall inside a window that starts in 2000"
    );

    let refused = p.home.run_err(&[
        "export-dataset",
        "--out",
        out.to_str().expect("utf8"),
        "--split",
        "1,1,1",
    ]);
    assert!(
        refused.1.contains("sum to 1.0"),
        "a malformed ratio must be refused naming the rule: {}",
        refused.1
    );
}
