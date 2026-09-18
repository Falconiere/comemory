#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory export-dataset` driven through the real binary over a real mixed
//! corpus — part 1: what a record carries, which evidence class reaches which
//! file, what a reviewer's silence produces, and what a purge leaves behind.
//!
//! The leakage, determinism and manifest half is
//! `tests/cli__export_dataset_2.rs`. The corpus both drive is
//! `tests/common/export_dataset_corpus.rs`.

#[path = "common/export_dataset_corpus.rs"]
mod fixture;

use comemory::store::candidate_judgments::{NewJudgment, upsert_all};
use fixture::corpus::{QUERY, refs};
use fixture::{export, field, out_of, prepared, records};
use serde_json::Value;

#[test]
fn export_writes_a_reviewed_dataset_over_every_domain() {
    let p = prepared();
    let out = out_of(&p.home, "dataset");

    // Everything into train, so the shape assertions below cannot depend on
    // which split the hash happened to choose for this corpus.
    let manifest = export(&p.home, &out, &["--split", "1,0,0"]);

    assert!(out.join("train.jsonl").exists());
    assert!(out.join("validation.jsonl").exists());
    assert!(out.join("manifest.json").exists());
    assert!(!out.join("holdout.jsonl").exists());

    let rows = records(&out.join("train.jsonl"));
    assert_eq!(rows.len(), 3, "one record per reviewed verdict: {rows:?}");
    let mut domains = field(&rows, "domain");
    domains.sort();
    assert_eq!(domains, vec!["code", "document", "memory"]);

    for row in &rows {
        assert_eq!(row["record_version"], 1);
        assert_eq!(row["observation_version"], 1);
        assert_eq!(row["observation_id"], p.first.as_str());
        assert_eq!(row["query"], QUERY);
        assert_eq!(row["label"]["provenance"], "manual");
        assert!(row["identity"]["domain"].is_string());
        assert!(!row["text"].as_str().expect("text").is_empty());
        assert_eq!(row["text_sha256"].as_str().expect("digest").len(), 64);
        assert!(!row["content_version"].as_str().expect("version").is_empty());
        assert!(row["filters"]["domains"].is_array());
        assert!(
            !row["retrieval_revision"]["knobs_hash"]
                .as_str()
                .expect("knobs hash")
                .is_empty()
        );
        assert!(
            !row["retrieval_revision"]["corpus_digest"]
                .as_str()
                .expect("corpus digest")
                .is_empty()
        );
        assert_eq!(
            row["retrieval_revision"]["binary_version"],
            env!("CARGO_PKG_VERSION")
        );
        assert!(row["reference_time"].is_string());
        assert!(row["pool_position"].as_i64().expect("pool position") >= 1);
    }

    let graded = records(&out.join("train.jsonl"))
        .iter()
        .filter_map(|r| r["label"]["relevance"].as_i64())
        .collect::<Vec<_>>();
    assert_eq!(graded.len(), 3);
    assert!(graded.contains(&0) && graded.contains(&2) && graded.contains(&3));
    assert_eq!(manifest["counts"]["rows_emitted"], 3);
    assert_eq!(manifest["by_domain"]["memory"], 1);
    assert!(
        !manifest["retrieval_revisions"]
            .as_array()
            .expect("revisions")
            .is_empty()
    );
}

#[test]
fn implicit_labels_stay_out_of_the_reviewed_files() {
    let p = prepared();
    // `comemory judge` cannot mint an implicit verdict by design — it has no
    // --source flag — so the row is written through the production store
    // writer against the live database the binary just captured into, naming a
    // candidate that run really observed.
    let unjudged = refs(&p.home.db(), &p.first)
        .into_iter()
        .find(|r| *r != p.memory_ref && *r != p.code_ref && *r != p.document_ref)
        .expect("the captured pool is larger than the three judged candidates");
    let domain = unjudged.split(':').next().expect("a domain prefix");
    upsert_all(
        &p.home.db(),
        &[NewJudgment {
            observation_id: &p.first,
            candidate_ref: &unjudged,
            domain,
            relevance: 3,
            provenance: "implicit",
            at: "2026-09-18T12:00:00.000000000Z",
        }],
    )
    .expect("record an implicit signal");

    let reviewed = out_of(&p.home, "reviewed");
    export(&p.home, &reviewed, &["--split", "1,0,0"]);
    assert!(
        !reviewed.join("train.implicit.jsonl").exists(),
        "the default export writes no implicit file at all"
    );
    let default_rows = records(&reviewed.join("train.jsonl"));
    assert!(
        !field(&default_rows, "candidate_ref").contains(&unjudged),
        "an implicit signal must never reach the reviewed dataset"
    );

    let both = out_of(&p.home, "both");
    export(&p.home, &both, &["--split", "1,0,0", "--provenance", "all"]);
    let manual = records(&both.join("train.jsonl"));
    let implicit = records(&both.join("train.implicit.jsonl"));
    assert_eq!(implicit.len(), 1);
    assert_eq!(implicit[0]["candidate_ref"], unjudged.as_str());
    assert_eq!(implicit[0]["label"]["provenance"], "implicit");
    assert!(
        !field(&manual, "candidate_ref").contains(&unjudged),
        "the two evidence classes are written to separate files and never mixed"
    );
    assert_eq!(manual.len(), 3);
}

#[test]
fn a_reviewed_zero_is_a_hard_negative_and_silence_is_not() {
    let p = prepared();
    let reviewed = out_of(&p.home, "reviewed");
    let manifest = export(&p.home, &reviewed, &["--split", "1,0,0"]);

    let judged = records(&reviewed.join("train.jsonl"));
    let negative = judged
        .iter()
        .find(|r| r["candidate_ref"] == p.document_ref.as_str())
        .expect("the reviewed-irrelevant document is a record");
    assert_eq!(
        negative["label"]["relevance"], 0,
        "a reviewed irrelevant candidate is the hard negative a reranker needs"
    );
    let unjudged_count = manifest["counts"]["candidates_unjudged"]
        .as_u64()
        .expect("the missing-label report");
    assert!(
        unjudged_count > 0,
        "the fixture pool is larger than three, or this assertion proves nothing"
    );
    assert_eq!(judged.len(), 3, "silence produces no record by default");

    let with_pool = out_of(&p.home, "with-pool");
    export(
        &p.home,
        &with_pool,
        &["--split", "1,0,0", "--include-unjudged"],
    );
    let all_rows = records(&with_pool.join("train.jsonl"));
    assert!(all_rows.len() > judged.len());
    let silent: Vec<&Value> = all_rows.iter().filter(|r| r["label"].is_null()).collect();
    assert_eq!(silent.len() as u64, unjudged_count);
    assert!(
        silent.iter().all(|r| r["label"].is_null()),
        "an unjudged candidate is reported as unjudged, never as a relevance of 0"
    );
}

#[test]
fn a_purged_memory_is_reported_not_exported() {
    let p = prepared();
    let memory_id = p
        .memory_ref
        .split(':')
        .nth(1)
        .expect("a memory reference carries its id")
        .to_string();

    p.home.run_ok(&["delete", &memory_id]);
    let trash = p.home.data_dir().join("memories/.trash");
    for entry in std::fs::read_dir(&trash).expect("read trash").flatten() {
        if entry.file_name().to_string_lossy().starts_with(&memory_id) {
            std::fs::remove_file(entry.path()).expect("unlink the trashed file");
        }
    }
    p.home
        .db()
        .execute(
            "UPDATE memories SET deleted_at = '2020-01-01T00:00:00.000000000Z' WHERE id = ?1",
            [&memory_id],
        )
        .expect("age the soft delete");
    let swept = p.home.run_json(&["gc"]);
    assert_eq!(swept["purged_rows"], 1, "gc must purge the memory: {swept}");

    let out = out_of(&p.home, "dataset");
    let manifest = export(&p.home, &out, &["--split", "1,0,0", "--include-unjudged"]);

    let rows = records(&out.join("train.jsonl"));
    assert!(
        !field(&rows, "candidate_ref").contains(&p.memory_ref),
        "a redacted candidate can never be exported, labelled or not"
    );
    assert!(
        manifest["counts"]["candidates_unresolved"]
            .as_u64()
            .expect("unresolved count")
            >= 1
    );
    assert_eq!(
        manifest["counts"]["judgments_on_unresolved_candidates"], 1,
        "the verdict recorded before the purge is reported, not exported"
    );
    assert!(
        field(&rows, "candidate_ref").contains(&p.code_ref),
        "the rest of the pool is unaffected"
    );
}
