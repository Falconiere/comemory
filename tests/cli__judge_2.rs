#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory judge` driven through the real binary — part 2: what a verdict
//! resolves against, and what happens to it when the corpus moves underneath.
//!
//! The five cases issue #209 names are all here and all real: re-indexing that
//! reuses code rowids, a changed memory, a deleted memory, a replaced document
//! chunk, and a verdict that arrives long after the capture.
//!
//! The capture half is `tests/cli__judge.rs`.

#[path = "common/observation_corpus.rs"]
mod corpus;

use corpus::{
    CHUNKING_MD_EDITED, QUERY, RANKING_RS_EDITED, candidate_scalar, count, git_commit,
    observation_of, ref_in, refs, seeded,
};

/// Capture one pool over `home` and return its observation id.
fn capture(home: &corpus::Home) -> String {
    observation_of(&home.capture_json(&["find", QUERY, "--k", "20"]))
}

#[test]
fn judgments_record_across_every_domain_under_the_stated_provenance() {
    let (home, _repo) = seeded();
    let id = capture(&home);
    let conn = home.db();
    let memory = ref_in(&conn, &id, "memory");
    let code = ref_in(&conn, &id, "code");
    let document = ref_in(&conn, &id, "document");

    let ack = home.run_json(&[
        "judge",
        &id,
        "--ref",
        &format!("{memory}=3"),
        "--ref",
        &format!("{code}=2"),
        "--ref",
        &format!("{document}=0"),
    ]);
    assert_eq!(ack["recorded"], 3);
    assert_eq!(
        ack["provenance"], "manual",
        "a typed verdict is a human one — the #130 explicit -> manual mapping"
    );
    assert_eq!(ack["observation_id"], id);

    let mut stmt = conn
        .prepare(
            "SELECT domain, relevance, provenance FROM candidate_judgments \
              WHERE observation_id = ?1 ORDER BY domain",
        )
        .expect("prepare");
    let rows: Vec<(String, i64, String)> = stmt
        .query_map([&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows");
    assert_eq!(
        rows,
        vec![
            ("code".to_string(), 2, "manual".to_string()),
            ("document".to_string(), 0, "manual".to_string()),
            ("memory".to_string(), 3, "manual".to_string()),
        ],
        "a document verdict has no other path in comemory; this is it"
    );

    // The legacy counters are a separate record and must be untouched.
    assert_eq!(count(&conn, "feedback"), 0);
    assert_eq!(count(&conn, "code_feedback"), 0);
    assert_eq!(count(&conn, "feedback_events"), 0);
}

#[test]
fn the_legacy_feedback_path_is_unchanged_by_a_judgment() {
    let (home, _repo) = seeded();
    let id = capture(&home);
    let conn = home.db();
    let memory = ref_in(&conn, &id, "memory");
    home.run_ok(&["judge", &id, "--ref", &format!("{memory}=3")]);

    let query_id: String = conn
        .query_row(
            "SELECT query_id FROM retrieval_log ORDER BY at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("the capturing find also wrote its retrieval_log row");
    let memory_id = memory.split(':').nth(1).expect("id").to_string();
    let ack = home.run_json(&["feedback", &query_id, "--used", &memory_id]);
    assert_eq!(ack["ok"], true);
    assert_eq!(ack["used"], 1);
    assert_eq!(
        ack["known_query"], true,
        "the capture must not have disturbed the retrieval log"
    );
    assert_eq!(count(&conn, "feedback"), 1);
    assert_eq!(count(&conn, "candidate_judgments"), 1);
}

#[test]
fn a_judgment_retrieval_never_returned_is_refused_and_nothing_is_written() {
    let (home, _repo) = seeded();
    let id = capture(&home);
    let conn = home.db();
    let present = ref_in(&conn, &id, "memory");
    let absent = format!("memory:ffffffff:{}", "0".repeat(64));

    let (code, stderr) = home.run_err(&[
        "judge",
        &id,
        "--ref",
        &format!("{present}=3"),
        "--ref",
        &format!("{absent}=3"),
    ]);
    assert_eq!(code, Some(78), "a refusal exits EX_CONFIG: {stderr}");
    assert!(
        stderr.contains("candidate-pool recall miss"),
        "the refusal must name the reason: {stderr}"
    );
    assert!(stderr.contains(&absent), "and the reference: {stderr}");
    assert_eq!(
        count(&conn, "candidate_judgments"),
        0,
        "all-or-nothing: the reference that DID match must not be written either"
    );
}

#[test]
fn a_changed_memory_makes_a_pinned_judgment_stale_and_a_deleted_one_a_recall_miss() {
    let (home, _repo) = seeded();
    let first = capture(&home);
    let conn = home.db();
    let original = ref_in(&conn, &first, "memory");
    let memory_id = original.split(':').nth(1).expect("id").to_string();
    home.run_ok(&["judge", &first, "--ref", &format!("{original}=3")]);

    // Markdown is the source of truth: editing the body on disk and rebuilding
    // re-mirrors the row under the SAME id with different content. That is how
    // a memory's content version moves — a re-save cannot do it, because the
    // id is derived from the body.
    edit_memory_body(
        &home,
        &memory_id,
        "Activation decay must be pinned for measurement, and the exponent is now documented here.",
    );
    drop(conn);
    home.run_ok(&["rebuild"]);
    // A rebuild renames a fresh file over `comemory.db`, so a connection held
    // across it keeps reading the unlinked pre-rebuild inode.
    let conn = home.db();

    let second = capture(&home);
    let observed_now = refs(&conn, &second);
    assert!(
        !observed_now.is_empty(),
        "the re-captured pool must not be empty, or the assertions below pass vacuously"
    );
    assert!(
        !observed_now.contains(&original),
        "the edited body must present a different content version: {observed_now:?}"
    );
    assert!(
        observed_now
            .iter()
            .any(|r| r.starts_with(&format!("memory:{memory_id}:"))),
        "the memory itself is still returned — only its content version moved"
    );

    let (code, stderr) = home.run_err(&["judge", &second, "--ref", &format!("{original}=3")]);
    assert_eq!(code, Some(78));
    assert!(
        stderr.contains("is stale"),
        "a judgment pinned to a version this run did not see is stale, not a match: {stderr}"
    );
    assert_eq!(
        count(&conn, "candidate_judgments"),
        1,
        "the refused call writes nothing"
    );

    // The verdict made against the ORIGINAL observation still stands: it
    // describes the content that observation actually showed.
    let report = home.run_json(&["judge", &first]);
    let judged = report["candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .find(|c| c["candidate_ref"] == original.as_str())
        .expect("the original candidate is still recorded");
    assert_eq!(judged["relevance"], 3);

    // Now delete the memory outright: the identity itself stops being
    // returned, which is a pool-recall miss rather than staleness.
    home.run_ok(&["delete", &memory_id]);
    let third = capture(&home);
    let (code, stderr) = home.run_err(&["judge", &third, "--ref", &format!("{original}=3")]);
    assert_eq!(code, Some(78));
    assert!(
        stderr.contains("candidate-pool recall miss"),
        "a deleted memory is a miss, not a match: {stderr}"
    );
}

/// Rewrite the markdown body of `memory_id`, keeping its frontmatter intact.
fn edit_memory_body(home: &corpus::Home, memory_id: &str, body: &str) {
    let dir = home.data_dir().join("memories");
    let path = std::fs::read_dir(&dir)
        .expect("read memories dir")
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(memory_id))
        })
        .unwrap_or_else(|| panic!("no markdown file for {memory_id} in {}", dir.display()));
    let raw = std::fs::read_to_string(&path).expect("read the memory file");
    let (frontmatter, _) = raw
        .rsplit_once("\n---\n")
        .unwrap_or_else(|| panic!("no frontmatter terminator in {}", path.display()));
    std::fs::write(&path, format!("{frontmatter}\n---\n\n{body}\n")).expect("rewrite the body");
}

#[test]
fn reindexing_that_reuses_code_rowids_never_moves_a_judgment() {
    let (home, repo) = seeded();
    let first = capture(&home);
    let conn = home.db();
    let original = ref_in(&conn, &first, "code");
    home.run_ok(&["judge", &first, "--ref", &format!("{original}=3")]);

    let rowid_before: i64 = conn
        .query_row(
            "SELECT id FROM code_symbols WHERE repo = 'demo' AND path = 'src/ranking.rs'",
            [],
            |r| r.get(0),
        )
        .expect("the indexed symbol has a rowid");

    // Edit and re-index: `index_code` purges the file's `code_symbols` rows
    // and reinserts them, freeing rowids SQLite may recycle.
    git_commit::commit_files(&repo, &[("src/ranking.rs", RANKING_RS_EDITED)], "retune");
    home.run_ok(&[
        "index-code",
        "--path",
        repo.to_str().expect("utf8"),
        "--repo",
        "demo",
    ]);
    let rowid_after: i64 = conn
        .query_row(
            "SELECT id FROM code_symbols WHERE repo = 'demo' AND path = 'src/ranking.rs'",
            [],
            |r| r.get(0),
        )
        .expect("the symbol is indexed again");

    // Whatever SQLite did with the rowid, no stored identity contains one.
    for table in ["candidate_observations", "candidate_judgments"] {
        let hits: i64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM {table} \
                      WHERE candidate_ref LIKE '%:' || ?1 || ':%' OR candidate_ref LIKE '%:' || ?1"
                ),
                [rowid_before.to_string()],
                |r| r.get(0),
            )
            .expect("scan");
        assert_eq!(
            hits, 0,
            "{table} must not key anything on a recyclable code_symbols rowid \
             ({rowid_before} before, {rowid_after} after)"
        );
    }

    // The judgment still names the same (repo, path, symbol) it was made for.
    let stored: String = conn
        .query_row(
            "SELECT candidate_ref FROM candidate_judgments WHERE observation_id = ?1",
            [&first],
            |r| r.get(0),
        )
        .expect("the judgment survived the re-index");
    assert_eq!(stored, original);
    let parts: Vec<&str> = stored.split(':').collect();
    assert_eq!(
        &parts[..4],
        &["code", "demo", "src/ranking.rs", "activation_boost"]
    );

    // And a reference carrying the pre-edit blob OID is stale against a fresh
    // observation of the same symbol — never a silent match.
    let second = capture(&home);
    let current = ref_in(&conn, &second, "code");
    assert_ne!(current, original, "the edit changed the file's blob OID");
    let (code, stderr) = home.run_err(&["judge", &second, "--ref", &format!("{original}=3")]);
    assert_eq!(code, Some(78));
    assert!(stderr.contains("is stale"), "{stderr}");
}

#[test]
fn a_replaced_document_chunk_makes_the_old_ref_stale() {
    let (home, _repo) = seeded();
    let first = capture(&home);
    let conn = home.db();
    let original = ref_in(&conn, &first, "document");
    home.run_ok(&["judge", &first, "--ref", &format!("{original}=2")]);
    let passage: String = candidate_scalar(&conn, &first, &original, "text");
    assert!(passage.contains("paragraph boundary"));

    let docs = home.workspace().join("guides");
    std::fs::write(docs.join("chunking.md"), CHUNKING_MD_EDITED).expect("rewrite the document");
    home.run_ok(&["index", docs.to_str().expect("utf8")]);

    let second = capture(&home);
    let current = ref_in(&conn, &second, "document");
    assert_ne!(
        current, original,
        "a rewritten document has a new revision hash"
    );
    let (code, stderr) = home.run_err(&["judge", &second, "--ref", &format!("{original}=2")]);
    assert_eq!(code, Some(78));
    assert!(stderr.contains("is stale"), "{stderr}");

    // The first observation still holds the passage it actually showed.
    let kept: String = candidate_scalar(&conn, &first, &original, "text");
    assert_eq!(kept, passage, "a captured passage is never rewritten");
}

#[test]
fn a_delayed_verdict_resolves_against_the_corpus_it_observed() {
    let (home, repo) = seeded();
    let observed = capture(&home);
    let conn = home.db();
    let memory = ref_in(&conn, &observed, "memory");
    let code = ref_in(&conn, &observed, "code");

    // The corpus moves on before anyone reviews: a new memory, and an edited
    // and re-indexed source file.
    home.run_ok(&[
        "save",
        "A later decision about activation that nobody has judged yet.",
        "--kind",
        "decision",
        "--repo",
        "demo",
    ]);
    git_commit::commit_files(&repo, &[("src/ranking.rs", RANKING_RS_EDITED)], "retune");
    home.run_ok(&[
        "index-code",
        "--path",
        repo.to_str().expect("utf8"),
        "--repo",
        "demo",
    ]);

    // Only now is the verdict recorded, and it resolves against what that
    // observation saw — not against the corpus as it now stands.
    let ack = home.run_json(&[
        "judge",
        &observed,
        "--ref",
        &format!("{memory}=3"),
        "--ref",
        &format!("{code}=1"),
    ]);
    assert_eq!(ack["recorded"], 2);

    let stored: String = conn
        .query_row(
            "SELECT candidate_ref FROM candidate_judgments \
              WHERE observation_id = ?1 AND domain = 'code'",
            [&observed],
            |r| r.get(0),
        )
        .expect("the code verdict");
    assert_eq!(
        stored, code,
        "a late verdict stores the content version that was observed"
    );

    // The same reference against a fresh observation is stale: the delay is
    // visible, never silently re-attributed to today's content.
    let fresh = capture(&home);
    let (exit, stderr) = home.run_err(&["judge", &fresh, "--ref", &format!("{code}=1")]);
    assert_eq!(exit, Some(78));
    assert!(stderr.contains("is stale"), "{stderr}");
}

#[test]
fn an_unknown_observation_version_is_refused() {
    let (home, _repo) = seeded();
    let id = capture(&home);
    let conn = home.db();
    let reference = ref_in(&conn, &id, "memory");
    conn.execute(
        "UPDATE candidate_query_observations SET observation_version = 999 \
          WHERE observation_id = ?1",
        [&id],
    )
    .expect("bump the contract version");

    let (code, stderr) = home.run_err(&["judge", &id, "--ref", &format!("{reference}=3")]);
    assert_eq!(code, Some(78));
    assert!(
        stderr.contains("999"),
        "the error names the stored version: {stderr}"
    );
    assert!(
        stderr.contains('1'),
        "and the one this build reads: {stderr}"
    );
    assert!(stderr.contains(&id), "and the observation: {stderr}");
    assert_eq!(count(&conn, "candidate_judgments"), 0);
}

#[test]
fn an_unknown_or_malformed_observation_id_is_refused_with_its_own_code() {
    let (home, _repo) = seeded();
    let (code, stderr) = home.run_err(&["judge", "o-20260918-deadbeef"]);
    assert_eq!(code, Some(69), "an absent observation is EX_UNAVAILABLE");
    assert!(stderr.contains("o-20260918-deadbeef"), "{stderr}");

    let (code, stderr) = home.run_err(&["judge", "not-an-observation"]);
    assert_eq!(code, Some(78), "a malformed id is EX_CONFIG");
    assert!(stderr.contains("o-<yyyymmdd>-<8hex>"), "{stderr}");
}

#[test]
fn judge_without_a_verdict_reports_the_observation() {
    let (home, _repo) = seeded();
    let id = capture(&home);
    let conn = home.db();
    let memory = ref_in(&conn, &id, "memory");
    home.run_ok(&["judge", &id, "--ref", &format!("{memory}=3")]);

    let report = home.run_json(&["judge", &id]);
    assert_eq!(report["observation_id"], id);
    assert_eq!(report["observation_version"], 1);
    assert_eq!(report["query"], QUERY);
    assert_eq!(report["truncated"], false);
    let candidates = report["candidates"].as_array().expect("candidates");
    assert_eq!(candidates.len(), refs(&conn, &id).len());
    let judged = candidates
        .iter()
        .find(|c| c["candidate_ref"] == memory.as_str())
        .expect("the judged candidate is reported");
    assert_eq!(judged["relevance"], 3);
    let expected_position: i64 = candidate_scalar(&conn, &id, &memory, "pool_position");
    assert_eq!(
        judged["pool_position"], expected_position,
        "the report echoes retrieval's own pool order"
    );
    assert!(!judged["title"].as_str().expect("title").is_empty());
    assert!(
        candidates.iter().any(|c| c["relevance"].is_null()),
        "an unjudged candidate reports no relevance: {report}"
    );

    // A report writes nothing.
    assert_eq!(count(&conn, "candidate_judgments"), 1);

    let tty = home.run_ok(&["judge", &id]);
    assert!(
        tty.contains(&id),
        "the TTY view names the observation: {tty}"
    );
    assert!(tty.contains("candidate(s)"), "{tty}");
    assert!(
        tty.contains(&memory),
        "and every candidate reference: {tty}"
    );
}
