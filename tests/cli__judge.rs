#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Candidate observation capture driven through the real `comemory` binary
//! against a real mixed-domain corpus — part 1: what `comemory find` records,
//! what it records when capture is off, what happens when capture fails, and
//! how the observation tables behave under `gc` and `rebuild`.
//!
//! The judgment half is `tests/cli__judge_2.rs`.

#[path = "common/observation_corpus.rs"]
mod corpus;

use comemory::domains::learning::evaluation::candidate_identity::{CandidateIdentity, parse_ref};
use corpus::{CAPTURE_ON, QUERY, candidate_scalar, count, observation_of, ref_in, refs, seeded};

#[test]
fn capture_records_every_domain_in_one_observation() {
    let (home, _repo) = seeded();
    let found = home.capture_json(&["find", QUERY, "--k", "20"]);
    let id = observation_of(&found);

    let conn = home.db();
    assert_eq!(
        count(&conn, "candidate_query_observations"),
        1,
        "one run writes exactly one observation"
    );
    let stored = refs(&conn, &id);
    assert_eq!(
        stored.len() as i64,
        count(&conn, "candidate_observations"),
        "every candidate row belongs to this observation"
    );

    let mut domains: Vec<&'static str> = Vec::new();
    for reference in &stored {
        let identity = parse_ref(reference)
            .unwrap_or_else(|e| panic!("stored reference `{reference}` must parse: {e}"));
        assert_eq!(
            identity.candidate_ref(),
            *reference,
            "the stored reference must be exactly what the identity encodes"
        );
        let domain = identity.domain().as_str();
        if !domains.contains(&domain) {
            domains.push(domain);
        }
        match &identity {
            CandidateIdentity::Memory(m) => assert_eq!(m.content_hash.len(), 64),
            CandidateIdentity::Code(c) => {
                assert!(
                    !c.blob_oid.is_empty(),
                    "a code candidate carries its blob OID"
                );
                assert_eq!(c.repo, "demo");
                assert_eq!(c.path, "src/ranking.rs");
            }
            CandidateIdentity::Document(d) => {
                assert!(!d.revision_hash.is_empty());
                assert_eq!(d.path, "chunking.md");
                assert!(d.chunk_ordinal >= 0);
            }
        }
    }
    domains.sort_unstable();
    assert_eq!(
        domains,
        vec!["code", "document", "memory"],
        "the whole point of the contract is that all three corpora reach one pool"
    );
}

#[test]
fn captured_text_is_bounded_and_digested_before_truncation() {
    let (home, _repo) = seeded();
    let found = home
        .capturing_bin()
        .env("COMEMORY_OBSERVATIONS_MAX_TEXT_BYTES", "24")
        .args(["--json", "find", QUERY, "--k", "20"])
        .output()
        .expect("run find");
    assert!(found.status.success());
    let body: serde_json::Value =
        serde_json::from_slice(&found.stdout).expect("find --json is JSON");
    let id = observation_of(&body);

    let conn = home.db();
    let memory_ref = ref_in(&conn, &id, "memory");
    let text: String = candidate_scalar(&conn, &id, &memory_ref, "text");
    let sha: String = candidate_scalar(&conn, &id, &memory_ref, "text_sha256");
    let full: i64 = candidate_scalar(&conn, &id, &memory_ref, "text_full_bytes");
    let truncated: i64 = candidate_scalar(&conn, &id, &memory_ref, "text_truncated");

    assert!(
        text.len() <= 24,
        "text is bounded at the configured ceiling"
    );
    assert_eq!(truncated, 1, "a memory body exceeds a 24-byte bound");
    assert!(
        full > text.len() as i64,
        "full_bytes is the untruncated length: {full} vs {}",
        text.len()
    );
    assert_eq!(sha.len(), 64);

    // The digest covers the FULL body, not the bounded text: an unbounded
    // capture of the same corpus must produce the same digest.
    let (home2, _repo2) = seeded();
    let id2 = observation_of(&home2.capture_json(&["find", QUERY, "--k", "20"]));
    let conn2 = home2.db();
    let sha2: String = candidate_scalar(&conn2, &id2, &memory_ref, "text_sha256");
    assert_eq!(
        sha, sha2,
        "equal digests mean the same passage was seen under different bounds"
    );
    let truncated2: i64 = candidate_scalar(&conn2, &id2, &memory_ref, "text_truncated");
    assert_eq!(
        truncated2, 0,
        "the default bound does not truncate a fixture body"
    );
}

#[test]
fn an_observation_separates_the_pool_from_the_page() {
    let (home, _repo) = seeded();
    let page_one = observation_of(&home.capture_json(&["find", QUERY, "--k", "2"]));
    let conn = home.db();
    let positions: Vec<(i64, Option<i64>)> = pool_positions(&conn, &page_one);
    assert!(
        positions.len() > 2,
        "the pool must be deeper than the page, or this proves nothing: {positions:?}"
    );
    assert_eq!(positions[0], (1, Some(1)));
    assert_eq!(positions[1], (2, Some(2)));
    for (pool, returned) in positions.iter().skip(2) {
        assert_eq!(
            *returned, None,
            "pool position {pool} is below the page cut and was not returned"
        );
    }

    let page_two =
        observation_of(&home.capture_json(&["find", QUERY, "--k", "2", "--offset", "2"]));
    let second: Vec<(i64, Option<i64>)> = pool_positions(&conn, &page_two);
    assert_eq!(second[0], (1, None), "the offset skipped pool position 1");
    assert_eq!(second[1], (2, None));
    assert_eq!(
        second[2],
        (3, Some(1)),
        "the second page numbers its own returned positions from 1"
    );

    let (pool_size, limit, offset): (i64, i64, i64) = conn
        .query_row(
            "SELECT pool_size, page_limit, page_offset FROM candidate_query_observations \
              WHERE observation_id = ?1",
            [&page_two],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("read window");
    assert!(pool_size >= second.len() as i64);
    assert_eq!((limit, offset), (2, 2));
}

fn pool_positions(conn: &rusqlite::Connection, id: &str) -> Vec<(i64, Option<i64>)> {
    let mut stmt = conn
        .prepare(
            "SELECT pool_position, returned_position FROM candidate_observations \
              WHERE observation_id = ?1 ORDER BY pool_position",
        )
        .expect("prepare");
    stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows")
}

#[test]
fn an_observation_records_the_complete_effective_filters() {
    let (home, _repo) = seeded();
    let id = observation_of(&home.capture_json(&[
        "find",
        QUERY,
        "--domain",
        "memory",
        "--repo",
        "demo",
        "--kind",
        "decision",
        "--since",
        "2020-01-01",
    ]));
    let conn = home.db();
    let (filters_json, knobs_hash, corpus_digest): (String, String, String) = conn
        .query_row(
            "SELECT filters_json, knobs_hash, corpus_digest FROM candidate_query_observations \
              WHERE observation_id = ?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("read header");
    let filters: serde_json::Value = serde_json::from_str(&filters_json).expect("filters are JSON");

    assert_eq!(filters["domains"], serde_json::json!(["memory"]));
    assert_eq!(filters["repo"], "demo");
    assert_eq!(filters["kind"], "decision");
    assert!(
        filters["lang"].is_null(),
        "an unset filter is null, not omitted"
    );
    assert_eq!(filters["path_globs"], serde_json::json!([]));
    assert!(
        !filters["since"].is_null(),
        "the normalized --since bound is recorded"
    );
    assert!(filters["until"].is_null());
    assert!(filters["as_of"].is_null());
    assert_eq!(filters["vector"]["kind"], "lexical");

    assert_eq!(knobs_hash.len(), 64, "the ranking configuration digest");
    assert_eq!(corpus_digest.len(), 64, "the corpus snapshot digest");

    let retrieval_json: String = conn
        .query_row(
            "SELECT retrieval_json FROM candidate_query_observations WHERE observation_id = ?1",
            [&id],
            |r| r.get(0),
        )
        .expect("read retrieval");
    let retrieval: serde_json::Value =
        serde_json::from_str(&retrieval_json).expect("retrieval is JSON");
    assert_eq!(retrieval["knobs_hash"], knobs_hash);
    assert_eq!(retrieval["corpus"]["digest"], corpus_digest);
    assert_eq!(retrieval["schema_version"], "28");
    assert!(
        retrieval["corpus"]["repos"]
            .as_array()
            .is_some_and(|r| !r.is_empty()),
        "the pinned corpus snapshot names the indexed repo"
    );
}

#[test]
fn capture_is_off_by_default() {
    let (home, _repo) = seeded();
    let found = home.run_json(&["find", QUERY]);
    assert!(
        found["observation_id"].is_null(),
        "the default build captures nothing: {found}"
    );
    assert!(
        !found["hits"].as_array().expect("hits").is_empty(),
        "the search itself is unaffected"
    );
    let conn = home.db();
    assert_eq!(count(&conn, "candidate_query_observations"), 0);
    assert_eq!(count(&conn, "candidate_observations"), 0);
}

#[test]
fn a_disabled_access_tracker_also_disables_capture() {
    let (home, _repo) = seeded();
    let out = home
        .capturing_bin()
        .env("COMEMORY_DISABLE_ACCESS_TRACKING", "true")
        .args(["--json", "find", QUERY])
        .output()
        .expect("run find");
    assert!(out.status.success());
    let body: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON");
    assert!(
        body["observation_id"].is_null(),
        "a run that may not write telemetry must not write an observation: {body}"
    );
    assert_eq!(count(&home.db(), "candidate_query_observations"), 0);
}

#[test]
fn a_failing_capture_never_fails_the_search() {
    let (home, _repo) = seeded();
    // Arm capture, then take the candidate table away underneath it. The
    // schema-meta markers are untouched, so the migration runner does not put
    // it back: every later capture attempt fails at the write.
    home.db()
        .execute_batch("DROP TABLE candidate_observations;")
        .expect("drop the candidate table");

    let out = home
        .capturing_bin()
        .args(["--json", "find", QUERY])
        .output()
        .expect("run find");
    assert!(
        out.status.success(),
        "an optional capture must never fail a search (exit {:?}): {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let body: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON");
    assert!(
        body["observation_id"].is_null(),
        "a failed capture reports no observation: {body}"
    );
    assert!(
        !body["hits"].as_array().expect("hits").is_empty(),
        "the ranked hits are unaffected"
    );
    assert_eq!(
        count(&home.db(), "candidate_query_observations"),
        0,
        "the header must not survive a failed candidate write"
    );
}

#[test]
fn gc_evicts_unjudged_observations_and_keeps_judged_ones() {
    let (home, _repo) = seeded();
    let judged = observation_of(&home.capture_json(&["find", QUERY, "--k", "5"]));
    let unjudged = observation_of(&home.capture_json(&["find", "chunking splitter", "--k", "5"]));
    let conn = home.db();
    let reference = ref_in(&conn, &judged, "memory");
    home.run_ok(&["judge", &judged, "--ref", &format!("{reference}=3")]);

    // Age both past the retention window by rewriting the stored instant.
    conn.execute(
        "UPDATE candidate_query_observations SET at = '2020-01-01T00:00:00.000000000Z'",
        [],
    )
    .expect("age the observations");

    let report = home.run_json(&["gc"]);
    assert_eq!(
        report["observation_rows"], 1,
        "exactly the unjudged observation is evicted: {report}"
    );

    let survivors = refs(&conn, &judged);
    assert!(
        !survivors.is_empty(),
        "a judged observation keeps its candidates"
    );
    assert!(
        refs(&conn, &unjudged).is_empty(),
        "an unjudged observation's candidates go with it"
    );
    let remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM candidate_query_observations WHERE observation_id = ?1",
            [&unjudged],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(remaining, 0);
    assert_eq!(count(&conn, "candidate_judgments"), 1);
}

#[test]
fn purging_a_memory_redacts_its_captured_text_and_keeps_the_pool_shape() {
    let (home, _repo) = seeded();
    let id = observation_of(&home.capture_json(&["find", QUERY, "--k", "20"]));
    let conn = home.db();
    let before = refs(&conn, &id);
    let memory_ref = ref_in(&conn, &id, "memory");
    let memory_id = memory_ref
        .split(':')
        .nth(1)
        .expect("a memory reference carries its id")
        .to_string();
    let text_before: String = candidate_scalar(&conn, &id, &memory_ref, "text");
    assert!(!text_before.is_empty());

    home.run_ok(&["delete", &memory_id]);
    // Reach the purge the way `gc` itself does for a zombie row: the trash
    // file is gone and `deleted_at` is past the retention window. A zero
    // window is not an option — config validation refuses it.
    let trash = home.data_dir().join("memories/.trash");
    for entry in std::fs::read_dir(&trash).expect("read trash").flatten() {
        if entry.file_name().to_string_lossy().starts_with(&memory_id) {
            std::fs::remove_file(entry.path()).expect("unlink the trashed file");
        }
    }
    conn.execute(
        "UPDATE memories SET deleted_at = '2020-01-01T00:00:00.000000000Z' WHERE id = ?1",
        [&memory_id],
    )
    .expect("age the soft delete");
    let swept = home.run_json(&["gc"]);
    assert_eq!(
        swept["purged_rows"], 1,
        "gc must purge the memory row: {swept}"
    );

    let after = refs(&conn, &id);
    assert_eq!(
        after, before,
        "redaction must keep every pool position and every reference"
    );
    let text_after: String = candidate_scalar(&conn, &id, &memory_ref, "text");
    let unresolved: i64 = candidate_scalar(&conn, &id, &memory_ref, "unresolved");
    let full: i64 = candidate_scalar(&conn, &id, &memory_ref, "text_full_bytes");
    assert!(text_after.is_empty(), "the purged body must not survive");
    assert_eq!(
        unresolved, 1,
        "the candidate is explicitly marked unresolvable"
    );
    assert_eq!(full, 0);

    let other = before
        .iter()
        .find(|r| r.starts_with("code:") || r.starts_with("document:"))
        .expect("the pool holds another domain");
    let other_text: String = candidate_scalar(&conn, &id, other, "text");
    assert!(
        !other_text.is_empty(),
        "another domain's candidate is untouched by a memory purge"
    );

    let (code, stderr) = home.run_err(&["judge", &id, "--ref", &format!("{memory_ref}=3")]);
    assert_eq!(code, Some(78));
    assert!(
        stderr.contains("no content snapshot"),
        "a redacted candidate must be refused as unjudgeable: {stderr}"
    );
}

#[test]
fn rebuild_preserves_observations_and_judgments() {
    let (home, _repo) = seeded();
    let id = observation_of(&home.capture_json(&["find", QUERY, "--k", "20"]));
    let conn = home.db();
    let before = refs(&conn, &id);
    let reference = ref_in(&conn, &id, "code");
    home.run_ok(&["judge", &id, "--ref", &format!("{reference}=2")]);
    drop(conn);

    home.run_ok(&["rebuild"]);

    let conn = home.db();
    assert_eq!(
        refs(&conn, &id),
        before,
        "a rebuild copies every candidate row, in order"
    );
    let relevance: i64 = conn
        .query_row(
            "SELECT relevance FROM candidate_judgments \
              WHERE observation_id = ?1 AND candidate_ref = ?2",
            rusqlite::params![&id, &reference],
            |r| r.get(0),
        )
        .expect("the judgment survived the rebuild");
    assert_eq!(relevance, 2);
    let report = home.run_json(&["judge", &id]);
    assert_eq!(report["observation_version"], 1);
    assert_eq!(report["candidate_count"], before.len() as i64);
}

#[test]
fn the_tty_view_of_a_capturing_find_prints_the_observation_handle() {
    let (home, _repo) = seeded();
    let out = home
        .capturing_bin()
        .args(["find", QUERY])
        .output()
        .expect("run find");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("observation: o-"),
        "a captured run prints the handle `comemory judge` takes: {stdout}"
    );
    assert!(
        stdout.contains("comemory judge --observation"),
        "and says what to do with it: {stdout}"
    );

    let plain = home.run_ok(&["find", QUERY]);
    assert!(
        !plain.contains("observation:"),
        "a non-capturing run prints nothing extra: {plain}"
    );
    assert_eq!(CAPTURE_ON.0, "COMEMORY_OBSERVATIONS_ENABLED");
}
