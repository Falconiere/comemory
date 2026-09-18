#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/learning/evaluation/dataset_rows.rs` and the
//! selection rules it drives, against a real migrated SQLite file written
//! through the production `candidate_observations` and `candidate_judgments`
//! writers and read back through `store::candidate_dataset::snapshot`.

use comemory::config::Config;
use comemory::domains::learning::evaluation::candidate_identity::CandidateDomain;
use comemory::domains::learning::evaluation::candidate_observation::{
    CorpusSnapshot, EffectiveFilters, RetrievalKnobs, RetrievalVersion, VectorScenario,
};
use comemory::domains::learning::evaluation::dataset_rows::{
    BuiltRows, ProvenanceFilter, RowInput, build,
};
use comemory::store::candidate_dataset::snapshot;
use comemory::store::candidate_judgments::{NewJudgment, upsert_all};
use comemory::store::candidate_observations::{NewCandidate, NewObservation, insert};
use comemory::store::connection;
use rusqlite::Connection;

const MEMORY_REF: &str = "memory:5a9f19bc:aaaa";
const MEMORY_REF_V2: &str = "memory:5a9f19bc:bbbb";
const CODE_REF: &str = "code:demo:src/ranking.rs:activation_boost:1111";
const DOC_REF: &str = "document:0f1e2d3c:guides/chunking.md:rev1:0";
const QUERY: &str = "activation decay";
const AT: &str = "2026-09-18T10:00:00.000000000Z";

fn db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

fn filters_json() -> String {
    serde_json::to_string(&EffectiveFilters {
        domains: vec!["code".into(), "document".into(), "memory".into()],
        repo: None,
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

struct Header {
    id: String,
    query: String,
    at: String,
    version: i64,
    filters: String,
    retrieval: String,
}

impl Header {
    fn new(id: &str) -> Header {
        Header {
            id: id.to_string(),
            query: QUERY.to_string(),
            at: AT.to_string(),
            version: 1,
            filters: filters_json(),
            retrieval: retrieval_json(),
        }
    }

    fn write(&self, conn: &Connection, candidates: &[NewCandidate<'_>]) {
        insert(
            conn,
            &NewObservation {
                observation_id: &self.id,
                observation_version: self.version,
                query_id: Some("q-20260918-aaaaaaaa"),
                query: &self.query,
                source: "find",
                filters_json: &self.filters,
                retrieval_json: &self.retrieval,
                knobs_hash: "k0",
                corpus_digest: "c0",
                decay_frozen: true,
                pool_size: 24,
                page_limit: 12,
                page_offset: 0,
                truncated: false,
                at: &self.at,
            },
            candidates,
        )
        .expect("insert observation");
    }
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
        locator_json: r#"{"title":"a display title","repo":null,"path":null,"line_range":null,
            "heading_path":null,"symbol_id":null}"#,
    }
}

fn judge(conn: &Connection, observation: &str, reference: &str, relevance: i64, provenance: &str) {
    let domain = reference.split(':').next().unwrap_or("memory");
    upsert_all(
        conn,
        &[NewJudgment {
            observation_id: observation,
            candidate_ref: reference,
            domain,
            relevance,
            provenance,
            at: "2026-09-18T11:00:00.000000000Z",
        }],
    )
    .expect("record a verdict");
}

fn built(conn: &Connection, provenance: ProvenanceFilter, unjudged: bool) -> BuiltRows {
    built_in(conn, provenance, unjudged, &CandidateDomain::all())
}

fn built_in(
    conn: &Connection,
    provenance: ProvenanceFilter,
    unjudged: bool,
    domains: &[CandidateDomain],
) -> BuiltRows {
    let captured = snapshot(conn, None, None).expect("snapshot");
    build(RowInput {
        snapshot: &captured,
        provenance,
        domains,
        include_unjudged: unjudged,
    })
    .expect("build rows")
}

#[test]
fn an_unknown_contract_version_is_refused_and_counted_without_failing_the_run() {
    let (_dir, conn) = db();
    let mut future = Header::new("o-20260918-00000001");
    future.version = 999;
    future.write(&conn, &[candidate(1, "memory", MEMORY_REF)]);
    Header::new("o-20260918-00000002").write(&conn, &[candidate(1, "code", CODE_REF)]);
    judge(&conn, "o-20260918-00000002", CODE_REF, 3, "manual");

    let rows = built(&conn, ProvenanceFilter::Manual, false);

    assert_eq!(rows.stats.observations_scanned, 2);
    assert_eq!(rows.stats.observations_refused_version, 1);
    assert_eq!(rows.contexts.len(), 1);
    assert_eq!(rows.rows.len(), 1);
    assert_eq!(rows.rows[0].candidate_ref, CODE_REF);
    assert_eq!(
        rows.stats.candidates_scanned, 1,
        "a refused observation's candidates are never read at all"
    );
}

#[test]
fn an_unresolved_candidate_is_never_exported_even_when_it_carries_a_verdict() {
    let (_dir, conn) = db();
    let mut redacted = candidate(1, "memory", MEMORY_REF);
    redacted.unresolved = true;
    redacted.content_version = "";
    redacted.text = "";
    Header::new("o-20260918-00000001").write(&conn, &[redacted, candidate(2, "code", CODE_REF)]);
    judge(&conn, "o-20260918-00000001", MEMORY_REF, 3, "manual");
    judge(&conn, "o-20260918-00000001", CODE_REF, 0, "manual");

    let rows = built(&conn, ProvenanceFilter::Manual, true);

    assert_eq!(rows.stats.candidates_unresolved, 1);
    assert_eq!(rows.stats.judgments_on_unresolved_candidates, 1);
    assert!(
        rows.rows.iter().all(|r| r.candidate_ref != MEMORY_REF),
        "a candidate with no content snapshot can never be training data"
    );
    let kept = rows
        .rows
        .iter()
        .find(|r| r.candidate_ref == CODE_REF)
        .expect("the resolved candidate survives");
    assert_eq!(kept.label.as_ref().expect("a verdict").relevance, 0);
}

#[test]
fn a_changed_version_is_stale_and_an_absent_candidate_is_a_pool_miss() {
    let (_dir, conn) = db();
    Header::new("o-20260918-00000001").write(&conn, &[candidate(1, "memory", MEMORY_REF)]);
    judge(&conn, "o-20260918-00000001", MEMORY_REF_V2, 3, "manual");
    judge(&conn, "o-20260918-00000001", DOC_REF, 2, "manual");

    let rows = built(&conn, ProvenanceFilter::Manual, true);

    assert_eq!(rows.stats.judgments_scanned, 2);
    assert_eq!(rows.stats.judgments_stale, 1);
    assert_eq!(rows.stats.judgments_recall_miss, 1);
    assert_eq!(rows.rows.len(), 1);
    assert!(
        rows.rows[0].label.is_none(),
        "neither refused verdict may be attached to the candidate that WAS returned"
    );
}

#[test]
fn silence_is_reported_as_unjudged_and_never_becomes_a_zero() {
    let (_dir, conn) = db();
    Header::new("o-20260918-00000001").write(
        &conn,
        &[
            candidate(1, "memory", MEMORY_REF),
            candidate(2, "code", CODE_REF),
            candidate(3, "document", DOC_REF),
        ],
    );
    judge(&conn, "o-20260918-00000001", MEMORY_REF, 0, "manual");

    let reviewed = built(&conn, ProvenanceFilter::Manual, false);
    assert_eq!(
        reviewed.rows.len(),
        1,
        "only the reviewed candidate is a row"
    );
    assert_eq!(
        reviewed.rows[0]
            .label
            .as_ref()
            .expect("a verdict")
            .relevance,
        0,
        "a reviewed irrelevant candidate is a hard negative"
    );
    assert_eq!(reviewed.stats.candidates_unjudged, 2);

    let with_pool = built(&conn, ProvenanceFilter::Manual, true);
    assert_eq!(with_pool.rows.len(), 3);
    let unlabelled: Vec<_> = with_pool
        .rows
        .iter()
        .filter(|r| r.label.is_none())
        .collect();
    assert_eq!(unlabelled.len(), 2);
    assert!(
        unlabelled.iter().all(|r| r.label_class.is_none()),
        "an unjudged record carries no evidence class, so it cannot reach an implicit file"
    );
}

#[test]
fn an_implicit_verdict_never_reaches_the_reviewed_default() {
    let (_dir, conn) = db();
    Header::new("o-20260918-00000001").write(
        &conn,
        &[
            candidate(1, "memory", MEMORY_REF),
            candidate(2, "code", CODE_REF),
        ],
    );
    judge(&conn, "o-20260918-00000001", MEMORY_REF, 3, "manual");
    judge(&conn, "o-20260918-00000001", CODE_REF, 2, "implicit");

    let reviewed = built(&conn, ProvenanceFilter::Manual, false);
    assert_eq!(reviewed.rows.len(), 1);
    assert_eq!(reviewed.rows[0].candidate_ref, MEMORY_REF);

    let everything = built(&conn, ProvenanceFilter::All, false);
    assert_eq!(everything.rows.len(), 2);
    let implicit = everything
        .rows
        .iter()
        .find(|r| r.candidate_ref == CODE_REF)
        .expect("the implicit row is present under --provenance all");
    assert_eq!(
        implicit.label.as_ref().expect("a verdict").provenance,
        "implicit",
        "an implicit label must stay explicitly identified"
    );

    let only_implicit = built(&conn, ProvenanceFilter::Implicit, false);
    assert_eq!(only_implicit.rows.len(), 1);
    assert_eq!(only_implicit.rows[0].candidate_ref, CODE_REF);
}

#[test]
fn the_domain_filter_excludes_a_candidate_and_counts_it_without_a_false_pool_miss() {
    let (_dir, conn) = db();
    Header::new("o-20260918-00000001").write(
        &conn,
        &[
            candidate(1, "memory", MEMORY_REF),
            candidate(2, "code", CODE_REF),
        ],
    );
    judge(&conn, "o-20260918-00000001", CODE_REF, 3, "manual");

    let rows = built_in(
        &conn,
        ProvenanceFilter::Manual,
        true,
        &[CandidateDomain::Memory],
    );

    assert_eq!(rows.stats.candidates_filtered_by_domain, 1);
    assert_eq!(rows.rows.len(), 1);
    assert_eq!(rows.rows[0].candidate_ref, MEMORY_REF);
    assert_eq!(
        rows.stats.judgments_recall_miss, 0,
        "a verdict on a candidate this run FILTERED OUT is a filter, not a pool miss"
    );
}

#[test]
fn an_unreadable_contract_column_fails_the_export_rather_than_being_dropped() {
    let (_dir, conn) = db();
    let mut broken = Header::new("o-20260918-00000001");
    broken.filters = "{\"not\":\"an EffectiveFilters\"}".to_string();
    broken.write(&conn, &[candidate(1, "memory", MEMORY_REF)]);

    let captured = snapshot(&conn, None, None).expect("snapshot");
    let outcome = build(RowInput {
        snapshot: &captured,
        provenance: ProvenanceFilter::Manual,
        domains: &CandidateDomain::all(),
        include_unjudged: true,
    });

    let message = match outcome {
        Ok(_) => panic!("an unreadable contract column must fail the export"),
        Err(e) => e.to_string(),
    };
    assert!(message.contains("o-20260918-00000001"), "{message}");
    assert!(message.contains("filters_json"), "{message}");
}

#[test]
fn an_unknown_provenance_word_is_refused_naming_what_is_accepted() {
    let failure = ProvenanceFilter::parse("explicit").expect_err("refused");
    let message = failure.to_string();
    assert!(message.contains("explicit"), "{message}");
    assert!(message.contains("manual, implicit, all"), "{message}");
    assert_eq!(
        ProvenanceFilter::parse("manual").expect("accepted"),
        ProvenanceFilter::Manual
    );
}
