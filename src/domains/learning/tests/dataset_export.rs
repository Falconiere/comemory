#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/learning/dataset_export.rs`, against a real
//! migrated SQLite file written through the production
//! `candidate_observations` and `candidate_judgments` writers.
//!
//! The real-binary journey over a real captured pool is
//! `tests/cli__export_dataset.rs`; what this suite pins is the pipeline's own
//! arithmetic and the file rules the journey would only observe indirectly.

use std::path::{Path, PathBuf};

use comemory::config::Config;
use comemory::config::paths::Paths;
use comemory::domains::learning::dataset_export::{ExportReport, Request, run};
use comemory::domains::learning::evaluation::candidate_observation::{
    CorpusSnapshot, EffectiveFilters, RetrievalKnobs, RetrievalVersion, VectorScenario,
};
use comemory::store::candidate_judgments::{NewJudgment, upsert_all};
use comemory::store::candidate_observations::{NewCandidate, NewObservation, insert};
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use rusqlite::Connection;
use serde_json::Value;

const MEMORY_REF: &str = "memory:5a9f19bc:aaaa";
const CODE_REF: &str = "code:demo:src/ranking.rs:activation_boost:1111";
const DOC_REF: &str = "document:0f1e2d3c:guides/chunking.md:rev1:0";
const AT: &str = "2026-09-18T10:00:00.000000000Z";

struct Home {
    root: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
}

impl Home {
    fn new() -> Home {
        let root = tempfile::tempdir().expect("tempdir");
        let paths = Paths::new(root.path().join("data"));
        paths.ensure_dirs().expect("ensure dirs");
        Home {
            root,
            paths,
            cfg: Config::defaults(),
        }
    }

    fn db(&self) -> Connection {
        connection::open(self.paths.db_path()).expect("open")
    }

    fn out(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    fn export(&self, req: Request) -> comemory::prelude::Result<ExportReport> {
        let mut conn = self.db();
        let mut ctx = Ctx::borrowed(&self.paths, &self.cfg, &mut conn);
        run(&mut ctx, &req)
    }
}

fn request(out: PathBuf) -> Request {
    Request {
        out,
        provenance: None,
        include_unjudged: false,
        include_holdout: false,
        domains: Vec::new(),
        since: None,
        until: None,
        split: None,
        split_seed: None,
        holdout_repo: None,
        holdout_since: None,
        max_negatives_per_query: 0,
    }
}

fn filters_json(repo: Option<&str>) -> String {
    serde_json::to_string(&EffectiveFilters {
        domains: vec!["code".into(), "document".into(), "memory".into()],
        repo: repo.map(str::to_string),
        kind: None,
        lang: None,
        path_globs: Vec::new(),
        since: None,
        until: None,
        as_of: None,
        vector: VectorScenario::Lexical,
    })
    .expect("serialize filters")
}

fn retrieval_json() -> String {
    serde_json::to_string(&RetrievalVersion {
        binary_version: "0.36.0".into(),
        schema_version: "20".into(),
        knobs: RetrievalKnobs::of(&Config::defaults()),
        knobs_hash: "k0".into(),
        corpus: CorpusSnapshot {
            memories: 3,
            code_symbols: 1,
            documents: 1,
            document_chunks: 2,
            repos: Vec::new(),
            digest: "c0".into(),
        },
    })
    .expect("serialize retrieval")
}

fn candidate<'a>(position: i64, domain: &'a str, reference: &'a str) -> NewCandidate<'a> {
    NewCandidate {
        pool_position: position,
        domain,
        candidate_ref: reference,
        content_version: reference.rsplit(':').next().unwrap_or_default(),
        unresolved: false,
        returned_position: Some(position),
        retrieval_score: 0.5,
        rank_in_domain: position,
        tier: None,
        text: "the recorded passage",
        text_sha256: "d0",
        text_full_bytes: 20,
        text_truncated: false,
        locator_json: "{}",
    }
}

struct Observation<'a> {
    id: &'a str,
    query: &'a str,
    at: &'a str,
    repo: Option<&'a str>,
}

fn write(conn: &Connection, o: Observation<'_>, candidates: &[NewCandidate<'_>]) {
    insert(
        conn,
        &NewObservation {
            observation_id: o.id,
            observation_version: 1,
            query_id: None,
            query: o.query,
            source: "find",
            filters_json: &filters_json(o.repo),
            retrieval_json: &retrieval_json(),
            knobs_hash: "k0",
            corpus_digest: "c0",
            decay_frozen: true,
            pool_size: 24,
            page_limit: 12,
            page_offset: 0,
            truncated: false,
            at: o.at,
        },
        candidates,
    )
    .expect("insert observation");
}

fn judge(conn: &Connection, observation: &str, reference: &str, relevance: i64, at: &str) {
    let domain = reference.split(':').next().unwrap_or("memory");
    upsert_all(
        conn,
        &[NewJudgment {
            observation_id: observation,
            candidate_ref: reference,
            domain,
            relevance,
            provenance: "manual",
            at,
        }],
    )
    .expect("record a verdict");
}

fn lines(path: &Path) -> Vec<Value> {
    let body = std::fs::read_to_string(path).expect("read a jsonl file");
    body.lines()
        .map(|line| serde_json::from_str(line).expect("each line is one JSON record"))
        .collect()
}

/// One code candidate in `demo` and one memory candidate, on two queries whose
/// token bags are disjoint, so the export sees two components.
fn two_components(conn: &Connection) {
    write(
        conn,
        Observation {
            id: "o-20260918-00000001",
            query: "activation decay",
            at: AT,
            repo: None,
        },
        &[candidate(1, "code", CODE_REF)],
    );
    judge(conn, "o-20260918-00000001", CODE_REF, 3, AT);
    write(
        conn,
        Observation {
            id: "o-20260918-00000002",
            query: "zebra widget calibration",
            at: AT,
            repo: None,
        },
        &[candidate(1, "memory", MEMORY_REF)],
    );
    judge(conn, "o-20260918-00000002", MEMORY_REF, 0, AT);
}

#[test]
fn an_export_writes_the_training_files_and_withholds_the_qualification_one() {
    let home = Home::new();
    two_components(&home.db());
    let out = home.out("dataset");

    let report = home
        .export(Request {
            holdout_repo: Some("demo".into()),
            ..request(out.clone())
        })
        .expect("export");

    assert!(out.join("train.jsonl").exists());
    assert!(out.join("validation.jsonl").exists());
    assert!(out.join("manifest.json").exists());
    assert!(
        !out.join("holdout.jsonl").exists(),
        "the qualification split must have no file for training-time mining to read"
    );
    assert!(
        !out.join("train.implicit.jsonl").exists(),
        "an implicit file is written only when implicit labels are asked for"
    );

    let withheld = &report.manifest.withheld;
    assert_eq!(withheld.len(), 1);
    assert_eq!(withheld[0].path, "holdout.jsonl");
    assert_eq!(withheld[0].rows, 1, "the code row was reserved by its repo");
    assert_eq!(withheld[0].sha256.len(), 64);
    assert!(
        withheld[0]
            .reason
            .as_deref()
            .is_some_and(|r| r.contains("--include-holdout"))
    );

    let train = lines(&out.join("train.jsonl"));
    assert_eq!(train.len(), 1);
    assert_eq!(train[0]["candidate_ref"], MEMORY_REF);
    assert_eq!(train[0]["record_version"], 1);
    assert_eq!(train[0]["observation_version"], 1);
    assert_eq!(train[0]["label"]["relevance"], 0);
    assert_eq!(train[0]["label"]["provenance"], "manual");
    assert_eq!(train[0]["identity"]["domain"], "memory");
    assert_eq!(train[0]["identity"]["memory_id"], "5a9f19bc");
    assert_eq!(train[0]["retrieval_revision"]["knobs_hash"], "k0");
    assert!(train[0]["filters"].is_object());
    assert!(
        train.iter().all(|r| r["candidate_ref"] != CODE_REF),
        "no reserved candidate may appear in a training file"
    );
}

#[test]
fn a_released_holdout_matches_the_digest_the_withholding_run_published() {
    let home = Home::new();
    two_components(&home.db());
    let withholding = home
        .export(Request {
            holdout_repo: Some("demo".into()),
            ..request(home.out("train-only"))
        })
        .expect("export");

    let released = home
        .export(Request {
            holdout_repo: Some("demo".into()),
            include_holdout: true,
            ..request(home.out("qualify"))
        })
        .expect("export");

    let published = &withholding.manifest.withheld[0];
    let written = released
        .manifest
        .files
        .iter()
        .find(|f| f.path == "holdout.jsonl")
        .expect("the released run writes the holdout file");
    assert_eq!(
        published.sha256, written.sha256,
        "publishing a digest lets a qualification run prove it scored the same rows"
    );
    assert_eq!(published.rows, written.rows);
    assert_eq!(
        std::fs::read(home.out("train-only").join("train.jsonl")).expect("read"),
        std::fs::read(home.out("qualify").join("train.jsonl")).expect("read"),
        "releasing the holdout must not change one byte of the training file"
    );
}

#[test]
fn two_exports_of_one_snapshot_are_byte_identical() {
    let home = Home::new();
    two_components(&home.db());
    let out = home.out("dataset");

    let first = home.export(request(out.clone())).expect("export");
    let train_before = std::fs::read(out.join("train.jsonl")).expect("read");
    let manifest_before = std::fs::read(out.join("manifest.json")).expect("read");

    let second = home.export(request(out.clone())).expect("export");

    assert_eq!(
        train_before,
        std::fs::read(out.join("train.jsonl")).expect("read")
    );
    assert_eq!(
        manifest_before,
        std::fs::read(out.join("manifest.json")).expect("read")
    );
    assert_eq!(first.manifest.dataset_id, second.manifest.dataset_id);
    assert_eq!(
        first.manifest.snapshot_digest,
        second.manifest.snapshot_digest
    );

    judge(&home.db(), "o-20260918-00000001", CODE_REF, 1, AT);
    let third = home.export(request(out)).expect("export");
    assert_ne!(
        first.manifest.snapshot_digest, third.manifest.snapshot_digest,
        "a changed verdict is a changed snapshot"
    );
    assert_ne!(first.manifest.dataset_id, third.manifest.dataset_id);
}

#[test]
fn a_stale_holdout_file_is_removed_before_a_withholding_run_writes() {
    let home = Home::new();
    two_components(&home.db());
    let out = home.out("dataset");
    home.export(Request {
        holdout_repo: Some("demo".into()),
        include_holdout: true,
        ..request(out.clone())
    })
    .expect("export");
    assert!(out.join("holdout.jsonl").exists());
    let bystander = out.join("notes.txt");
    std::fs::write(&bystander, b"not this command's file").expect("write");

    home.export(Request {
        holdout_repo: Some("demo".into()),
        ..request(out.clone())
    })
    .expect("export");

    assert!(
        !out.join("holdout.jsonl").exists(),
        "a qualification file an earlier run wrote must not survive a withholding run"
    );
    assert!(
        bystander.exists(),
        "only the command's own closed name set is swept"
    );
}

#[test]
fn a_malformed_request_is_refused_before_the_output_directory_exists() {
    let home = Home::new();
    two_components(&home.db());

    for (field, req) in [
        (
            "ratios that do not sum to one",
            Request {
                split: Some("0.5,0.2,0.2".into()),
                ..request(home.out("a"))
            },
        ),
        (
            "a negative ratio",
            Request {
                split: Some("1.2,-0.2,0.0".into()),
                ..request(home.out("b"))
            },
        ),
        (
            "the wrong number of ratios",
            Request {
                split: Some("0.8,0.2".into()),
                ..request(home.out("c"))
            },
        ),
        (
            "an unknown provenance word",
            Request {
                provenance: Some("explicit".into()),
                ..request(home.out("d"))
            },
        ),
        (
            "an unknown domain word",
            Request {
                domains: vec!["memories".into()],
                ..request(home.out("e"))
            },
        ),
        (
            "an unparsable date",
            Request {
                since: Some("last tuesday".into()),
                ..request(home.out("f"))
            },
        ),
    ] {
        let out = req.out.clone();
        home.export(req)
            .err()
            .unwrap_or_else(|| panic!("{field} must be refused"));
        assert!(
            !out.exists(),
            "{field}: a refused invocation must write nothing at all"
        );
    }
}

#[test]
fn duplicate_observations_collapse_and_a_revised_verdict_wins() {
    let home = Home::new();
    let conn = home.db();
    for id in ["o-20260918-00000001", "o-20260918-00000002"] {
        write(
            &conn,
            Observation {
                id,
                query: "activation decay",
                at: AT,
                repo: None,
            },
            &[candidate(1, "memory", MEMORY_REF)],
        );
    }
    judge(&conn, "o-20260918-00000001", MEMORY_REF, 3, AT);

    let collapsed = home.export(request(home.out("a"))).expect("export");

    assert_eq!(collapsed.manifest.counts.duplicate_observations_dropped, 1);
    let train = lines(&home.out("a").join("train.jsonl"));
    assert_eq!(train.len(), 1);
    assert_eq!(
        train[0]["observation_id"], "o-20260918-00000001",
        "the judged duplicate is the one worth keeping"
    );

    // A second query spelling under a different filter is NOT a duplicate, so
    // both observations survive and their disagreeing verdicts contend.
    write(
        &conn,
        Observation {
            id: "o-20260918-00000003",
            query: "decay activation",
            at: AT,
            repo: Some("demo"),
        },
        &[candidate(1, "memory", MEMORY_REF)],
    );
    judge(
        &conn,
        "o-20260918-00000003",
        MEMORY_REF,
        0,
        "2026-09-19T10:00:00.000000000Z",
    );

    let revised = home.export(request(home.out("b"))).expect("export");

    assert_eq!(revised.manifest.counts.judgments_contradictory_dropped, 1);
    let rows = lines(&home.out("b").join("train.jsonl"));
    let labelled: Vec<&Value> = rows.iter().filter(|r| !r["label"].is_null()).collect();
    assert_eq!(labelled.len(), 1);
    assert_eq!(
        labelled[0]["label"]["relevance"], 0,
        "the later verdict revises the earlier one"
    );
    assert_eq!(labelled[0]["observation_id"], "o-20260918-00000003");
}

#[test]
fn the_reviewed_negative_cap_runs_inside_one_split() {
    let home = Home::new();
    let conn = home.db();
    write(
        &conn,
        Observation {
            id: "o-20260918-00000001",
            query: "activation decay",
            at: AT,
            repo: None,
        },
        &[
            candidate(1, "memory", MEMORY_REF),
            candidate(2, "code", CODE_REF),
            candidate(3, "document", DOC_REF),
        ],
    );
    for reference in [MEMORY_REF, CODE_REF, DOC_REF] {
        judge(&conn, "o-20260918-00000001", reference, 0, AT);
    }

    let uncapped = home.export(request(home.out("all"))).expect("export");
    assert_eq!(uncapped.manifest.counts.negatives_capped, 0);

    let capped = home
        .export(Request {
            max_negatives_per_query: 1,
            ..request(home.out("capped"))
        })
        .expect("export");

    assert_eq!(capped.manifest.counts.negatives_capped, 2);
    assert_eq!(capped.manifest.counts.rows_emitted, 1);
    let kept = lines(&home.out("capped").join("train.jsonl"));
    assert_eq!(kept.len(), 1);
    assert_eq!(
        kept[0]["pool_position"], 1,
        "the cap keeps the negatives retrieval ranked highest"
    );
}

#[test]
fn an_empty_database_exports_an_empty_dataset_rather_than_failing() {
    let home = Home::new();
    let out = home.out("dataset");

    let report = home.export(request(out.clone())).expect("export");

    assert_eq!(report.manifest.counts.observations_scanned, 0);
    assert_eq!(report.manifest.counts.rows_emitted, 0);
    assert_eq!(report.manifest.split.groups, 0);
    assert_eq!(
        std::fs::read_to_string(out.join("train.jsonl")).expect("read"),
        ""
    );
    assert!(report.manifest.retrieval_revisions.is_empty());
    assert_eq!(report.manifest.files.len(), 2, "train and validation");
}
