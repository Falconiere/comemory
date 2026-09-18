#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end tests for the optional learned ordering stage (#213), driving the
//! real `comemory` binary over a real SQLite store.
//!
//! The applied path runs the shipped
//! `integrations/reranker/comemory_rerank.py score --scoring lexical-overlap`
//! as a real child process over real pipes, so the `[rerank]` configuration,
//! the #211 wire protocol, the #212 identity gate and comemory's own fallback
//! ladder are all exercised together. `python3` is a prerequisite; an absent
//! interpreter FAILS these tests rather than skipping them, exactly as it does
//! for `src/utilities/tests/rerank_runner_2.rs`.
//!
//! Every ordering assertion pins `[rank] decay = 0.0` and sets
//! `COMEMORY_DISABLE_ACCESS_TRACKING` where the run must not reinforce itself:
//! activation reads the wall clock and `access_count`, and without both levers
//! an exact comparison across two runs measures the clock, not the ranking.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// The shipped reference backend in its deterministic mode, as a TOML array.
fn backend_command() -> String {
    format!(
        "[\"python3\", {:?}, \"score\", \"--scoring\", \"lexical-overlap\"]",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/integrations/reranker/comemory_rerank.py"
        )
    )
}

/// A throwaway data directory plus the config file the run loads.
struct Home {
    root: TempDir,
}

impl Home {
    fn new() -> Self {
        Self {
            root: TempDir::new().expect("tempdir"),
        }
    }

    fn data_dir(&self) -> PathBuf {
        self.root.path().join(".comemory")
    }

    fn workspace(&self) -> &Path {
        self.root.path()
    }

    /// Write `<data_dir>/config.toml`. Always pins `decay = 0.0` so activation
    /// reduces to `ln(max(access_count, 1))` and two runs are comparable.
    fn write_config(&self, rerank: &str) {
        let dir = self.data_dir();
        std::fs::create_dir_all(&dir).expect("create data dir");
        std::fs::write(
            dir.join("config.toml"),
            format!("[rank]\ndecay = 0.0\n\n{rerank}"),
        )
        .expect("write config.toml");
    }

    fn bin(&self) -> Command {
        let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
        c.env("COMEMORY_DATA_DIR", self.data_dir());
        c
    }

    /// Run `comemory <args>`, returning `(success, stdout, stderr)`.
    fn run(&self, args: &[&str]) -> (bool, String, String) {
        let out = self.bin().args(args).output().expect("run comemory");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn run_ok(&self, args: &[&str]) -> String {
        let (ok, stdout, stderr) = self.run(args);
        assert!(ok, "comemory {args:?} failed: {stderr}");
        stdout
    }

    /// Run `comemory --json <args>` with access tracking disabled and parse it.
    fn json(&self, args: &[&str]) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let out = self
            .bin()
            .env("COMEMORY_DISABLE_ACCESS_TRACKING", "true")
            .args(&all)
            .output()
            .expect("run comemory");
        assert!(
            out.status.success(),
            "comemory {all:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        serde_json::from_str(stdout.trim())
            .unwrap_or_else(|e| panic!("--json {args:?} is not JSON: {e}\n{stdout}"))
    }

    /// Save one memory at `quality` and return its id.
    fn save_with(&self, body: &str, quality: u8) -> String {
        let quality = quality.to_string();
        let v: Value = serde_json::from_str(
            self.run_ok(&[
                "--json",
                "save",
                body,
                "--kind",
                "note",
                "--quality",
                &quality,
            ])
            .trim(),
        )
        .expect("save envelope");
        v["id"].as_str().expect("saved id").to_string()
    }
}

/// The query every ordering test runs. Every `sqlite` body below contains all
/// three terms, so the lexical ladder's strict tier matches the whole family.
const QUERY: &str = "sqlite busy timeout";

/// The corpus every ordering test uses, as `(quality, body)`.
///
/// It is built so the deterministic order and the lexical-overlap order
/// disagree, and the disagreement is asserted rather than assumed
/// (`applies_the_backend_order`). BM25 with length normalization rewards a
/// short body while Jaccard similarity rewards covering the query's token set
/// without adding to it, and `quality` — a real multiplicative rerank prior —
/// pushes the longest body up the deterministic list. Measured on this corpus:
/// the deterministic order swaps positions 1/2 and 4/5 relative to Jaccard.
fn corpus() -> Vec<(u8, String)> {
    vec![
        (1, "sqlite busy timeout".to_string()),
        (
            5,
            "sqlite busy timeout pool retry backoff wal mode".to_string(),
        ),
        (3, "sqlite busy timeout pool".to_string()),
        (3, "sqlite busy timeout pool retry".to_string()),
        (3, "sqlite busy timeout pool retry backoff wal".to_string()),
        (
            3,
            "postgres connection pool exhaustion under load".to_string(),
        ),
        (
            3,
            "rust borrow checker lifetimes in async blocks".to_string(),
        ),
    ]
}

/// Seed `home` with [`corpus`] and return the saved ids in order. A memory id
/// is the digest of its body, so the same corpus yields the same ids in every
/// store and two `Home`s are directly comparable.
fn seed(home: &Home) -> Vec<String> {
    corpus()
        .iter()
        .map(|(q, b)| home.save_with(b, *q))
        .collect()
}

/// Insert `n` distinct memories straight through the migrated store.
///
/// The pool tests need a corpus larger than `router::CANDIDATE_POOL` (50) for
/// the page-proportional bound to bite at all, and sixty `comemory save`
/// subprocesses would dominate this suite's runtime. The rows are the shape
/// `save` writes — `memories` plus `memory_fts`, with a spread SimHash so
/// near-duplicate collapse cannot silently shrink the list — and every
/// assertion reads them back through the real binary.
fn seed_many(home: &Home, n: usize) {
    let conn = comemory::store::connection::open(home.data_dir().join("comemory.db"))
        .expect("open comemory.db");
    for i in 0..n {
        let id = format!("{:08x}", 0x1000_0000_u32 + i as u32);
        let body = format!("sqlite busy timeout variant {i} of the pool retry family");
        let simhash = (i as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15) as i64;
        conn.execute(
            "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                                  body, created_at, updated_at, md_path, simhash)
             VALUES (?1, ?1, 'note', 'd', 'f', 3, 1, ?1, ?2,
                     '2026-06-09T00:00:00Z', '2026-06-09T00:00:00Z', ?1, ?3)",
            rusqlite::params![id, body, simhash],
        )
        .expect("seed memory");
        conn.execute(
            "INSERT INTO memory_fts(memory_id, body, tags) VALUES (?1, ?2, '')",
            rusqlite::params![id, body],
        )
        .expect("seed fts");
    }
}

/// The body behind a saved id, for the independently computed expectation.
fn body_of(ids: &[String], id: &str) -> String {
    let idx = ids.iter().position(|s| s == id).expect("a seeded id");
    corpus()[idx].1.clone()
}

/// Jaccard similarity over lowercased alphanumeric token sets — the pinned
/// definition of the backend's `lexical-overlap` mode, restated here so the
/// expected order is computed independently of the thing under test.
fn jaccard(query: &str, text: &str) -> f64 {
    let tokens = |s: &str| -> HashSet<String> {
        let mut out = HashSet::new();
        let mut current = String::new();
        for ch in s.to_lowercase().chars() {
            if ch.is_ascii_alphanumeric() {
                current.push(ch);
            } else if !current.is_empty() {
                out.insert(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            out.insert(current);
        }
        out
    };
    let (q, t) = (tokens(query), tokens(text));
    let union = q.union(&t).count();
    if union == 0 {
        return 0.0;
    }
    q.intersection(&t).count() as f64 / union as f64
}

/// The memory ids in a `search` / `find` / `context` JSON payload.
fn hit_ids(v: &Value, key: &str) -> Vec<String> {
    v["hits"]
        .as_array()
        .unwrap_or_else(|| panic!("hits array in {v}"))
        .iter()
        .map(|h| h[key].as_str().expect("hit id").to_string())
        .collect()
}

/// The `[rerank]` block pointing at the shipped backend.
fn enabled_block(prefix: usize) -> String {
    format!(
        "[rerank]\nenabled = true\ncommand = {}\nmodel = \"lexical-overlap@1\"\nprefix = {prefix}\n",
        backend_command()
    )
}

// ---------------------------------------------------------------- AC-1, AC-18

#[test]
fn disabled_by_default_changes_nothing() {
    let home = Home::new();
    home.write_config("");
    seed(&home);

    // `search`, `search-code` and `context` build their payload from a shared
    // `Envelope` whose `learned` field is skipped when absent, so the key is
    // not there at all.
    for args in [vec!["search", QUERY], vec!["search-code", "sqlite"]] {
        let v = home.json(&args);
        assert!(
            v.get("learned").is_none(),
            "{args:?} must emit no learned key at all: {v}"
        );
        assert!(v.get("hits").is_some(), "and still emit its hits: {v}");
    }
    let ctx = home.json(&["context", QUERY]);
    assert!(
        ctx.get("learned").is_none(),
        "context must emit no learned key at all: {ctx}"
    );

    // `find` assembles its object by hand and spells every absent field as an
    // explicit null — `query_id` and `observation_id` already do — so `learned`
    // is present and null rather than missing. Pinned, because the two shapes
    // are a contract a consumer reads.
    let find = home.json(&["find", QUERY]);
    assert_eq!(
        find["learned"],
        Value::Null,
        "find spells an absent learned object as null, like its siblings: {find}"
    );
    assert_eq!(find["query_id"], Value::Null);
    assert_eq!(find["observation_id"], Value::Null);
    assert!(!hit_ids(&find, "id").is_empty(), "the corpus must match");

    // The deterministic page is what it always was: the disabled run's ranked
    // window is the page-proportional pool, not the fixed one.
    let search = home.json(&["search", QUERY]);
    assert_eq!(
        hit_ids(&search, "memory_id"),
        hit_ids(&find, "id"),
        "and search still orders identically to find --domain all's memory leg"
    );
}

#[test]
fn fixed_window_reports_its_own_total() {
    let disabled = Home::new();
    disabled.write_config("");
    seed_many(&disabled, 60);
    let enabled = Home::new();
    enabled.write_config(&enabled_block(50));
    seed_many(&enabled, 60);

    let total = |v: &Value| v["total"].as_u64().expect("total");
    let off_head = disabled.json(&["search", QUERY, "--k", "2"]);
    let off_deep = disabled.json(&["search", QUERY, "--k", "2", "--offset", "120"]);
    assert_ne!(
        total(&off_head),
        total(&off_deep),
        "without the stage the ranked window really is page-proportional, \
         which is what the fixed universe replaces"
    );

    let on_head = enabled.json(&["search", QUERY, "--k", "2"]);
    let on_deep = enabled.json(&["search", QUERY, "--k", "2", "--offset", "120"]);
    assert_eq!(
        total(&on_head),
        total(&on_deep),
        "with the stage on, the ranked window must not depend on the page"
    );
    assert_eq!(
        on_head["learned"]["pool"].as_u64(),
        on_deep["learned"]["pool"].as_u64(),
        "and neither must the candidate universe the scorer was offered"
    );
    assert_eq!(
        on_head["learned"]["pool"].as_u64(),
        Some(total(&on_head)),
        "the reported pool is the ranked list the page was sliced from"
    );
    assert!(
        total(&on_head) > total(&off_head),
        "the fixed window really is larger than the page-proportional pool"
    );
}

// ------------------------------------------------------------------ AC-2

#[test]
fn disabled_launches_no_process() {
    let home = Home::new();
    let sentinel = home.workspace().join("scorer-ran");
    let script = home.workspace().join("record.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\ntouch {}\nexit 1\n", sentinel.display()),
    )
    .expect("write script");
    make_executable(&script);
    home.write_config(&format!(
        "[rerank]\nenabled = false\ncommand = [{:?}]\nmodel = \"lexical-overlap@1\"\n",
        script.to_string_lossy()
    ));
    seed(&home);
    for args in [
        vec!["search", "sqlite"],
        vec!["search-code", "sqlite"],
        vec!["find", "sqlite"],
        vec!["context", "sqlite"],
    ] {
        home.json(&args);
    }
    assert!(
        !sentinel.exists(),
        "a disabled stage must launch no process at all"
    );
}

// ------------------------------------------------------------------ AC-3

#[test]
fn invalid_rerank_config_is_refused() {
    for (block, needle) in [
        ("[rerank]\nprefix = 0\n", "rerank.prefix=0"),
        ("[rerank]\ntimeout_ms = 0\n", "rerank.timeout_ms=0"),
        (
            "[rerank]\nmax_candidate_text_bytes = 0\n",
            "rerank.max_candidate_text_bytes=0",
        ),
        (
            "[rerank]\nenabled = true\nmodel = \"m@1\"\ncommand = []\n",
            "rerank.command",
        ),
        (
            "[rerank]\nenabled = true\ncommand = [\"true\"]\nmodel = \"\"\n",
            "rerank.model",
        ),
        (
            "[rerank]\nprefix = 4096\nmax_candidate_text_bytes = 4096\n",
            "8388608-byte scorer request limit",
        ),
    ] {
        let home = Home::new();
        home.write_config(block);
        let (ok, _, stderr) = home.run(&["search", "anything"]);
        assert!(!ok, "config {block:?} must be refused");
        assert!(
            stderr.contains(needle),
            "stderr must name {needle:?}, got: {stderr}"
        );
    }
}

// ------------------------------------------------------------------ AC-4

#[test]
fn applies_the_backend_order() {
    let plain = Home::new();
    plain.write_config("");
    let ids = seed(&plain);
    let deterministic = hit_ids(&plain.json(&["search", QUERY, "--k", "0"]), "memory_id");

    let home = Home::new();
    home.write_config(&enabled_block(50));
    seed(&home);
    let v = home.json(&["search", QUERY, "--k", "0"]);
    assert_eq!(v["learned"]["applied"], serde_json::json!(true));
    assert_eq!(
        v["learned"]["model"],
        serde_json::json!("lexical-overlap@1")
    );
    assert_eq!(v["learned"]["adapter"], Value::Null);
    let returned = hit_ids(&v, "memory_id");
    assert_eq!(
        returned.len(),
        deterministic.len(),
        "reranking must not change which candidates are returned"
    );

    // The expectation is computed here, from the bodies this test saved: the
    // deterministic order sorted by Jaccard similarity descending, ties broken
    // by the deterministic position — which is exactly the protocol's rule.
    let mut expected: Vec<(usize, f64, String)> = deterministic
        .iter()
        .enumerate()
        .map(|(pos, id)| (pos, jaccard(QUERY, &body_of(&ids, id)), id.clone()))
        .collect();
    expected.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let expected_ids: Vec<String> = expected.into_iter().map(|(_, _, id)| id).collect();
    assert_eq!(
        returned, expected_ids,
        "the returned order must be the backend's own Jaccard order"
    );
    assert_ne!(
        returned, deterministic,
        "the fixture must make the learned order differ from the deterministic one, \
         or this test asserts nothing"
    );

    let scores = v["learned"]["scores"].as_array().expect("scores");
    assert_eq!(scores.len(), returned.len());
    let mut moved = false;
    for (i, s) in scores.iter().enumerate() {
        assert_eq!(s["rank"].as_u64(), Some(i as u64 + 1));
        assert!(s["score"].as_f64().is_some_and(f64::is_finite));
        if s["rank"] != s["deterministic_rank"] {
            moved = true;
        }
    }
    assert!(
        moved,
        "deterministic_rank must be reported distinctly from the effective rank"
    );
}

/// `search-code` with the stage enabled, over a code index whose observation
/// references collide. Covers the code surface's whole paused path: `code_keys`,
/// code-domain materialization, the envelope's `learned` key, and the reason the
/// wire id is the pool key rather than the observation reference.
#[test]
fn search_code_reranks_over_colliding_symbol_references() {
    let home = Home::new();
    home.write_config(&enabled_block(50));
    home.save_with("a memory so the store exists", 3);
    seed_colliding_symbols(&home);

    let v = home.json(&["search-code", "new"]);
    assert_eq!(
        v["learned"]["applied"],
        serde_json::json!(true),
        "two same-named functions in one file must not decline the stage: {v}"
    );
    let hits = v["hits"].as_array().expect("hits");
    assert_eq!(hits.len(), 2, "both symbols are returned: {v}");
    let scores = v["learned"]["scores"].as_array().expect("scores");
    assert_eq!(scores.len(), 2);
    assert_ne!(
        scores[0]["candidate_id"], scores[1]["candidate_id"],
        "the wire ids must differ, which is what keeps the request valid"
    );
    assert_eq!(
        scores[0]["candidate_ref"], scores[1]["candidate_ref"],
        "while the observation references really do collide"
    );

    // And the dedicated command still orders identically to the single-domain
    // unified one, with the stage on.
    let code_ids: Vec<String> = hits.iter().map(|h| h["symbol_id"].to_string()).collect();
    let find_ids = hit_ids(&home.json(&["find", "new", "--domain", "code"]), "id");
    assert_eq!(
        code_ids, find_ids,
        "search-code and find --domain code must agree with the stage on"
    );
}

// ------------------------------------------------------------------ AC-5

#[test]
fn one_scorer_process_per_search() {
    let home = Home::new();
    let log = home.workspace().join("invocations.log");
    let script = recording_proxy(home.workspace(), &log, None);
    home.write_config(&format!(
        "[rerank]\nenabled = true\ncommand = [{:?}]\nmodel = \"lexical-overlap@1\"\n",
        script.to_string_lossy()
    ));
    seed(&home);
    let v = home.json(&["find", QUERY, "--domain", "all"]);
    assert_eq!(v["learned"]["applied"], serde_json::json!(true));
    let lines = std::fs::read_to_string(&log).expect("log");
    assert_eq!(
        lines.lines().count(),
        1,
        "one requested search must launch exactly one scorer, got:\n{lines}"
    );
}

// ------------------------------------------------------------------ AC-6

#[test]
fn paging_matches_the_full_window() {
    let home = Home::new();
    home.write_config(&enabled_block(20));
    seed_many(&home, 60);
    let full = hit_ids(&home.json(&["search", QUERY, "--k", "0"]), "memory_id");
    assert!(
        full.len() > 20,
        "the corpus must cross the rerank prefix boundary, got {}",
        full.len()
    );
    for size in [1usize, 5, 12] {
        let mut walked: Vec<String> = Vec::new();
        let mut offset = 0usize;
        while walked.len() < full.len() {
            let page = home.json(&[
                "search",
                QUERY,
                "--k",
                &size.to_string(),
                "--offset",
                &offset.to_string(),
            ]);
            let ids = hit_ids(&page, "memory_id");
            if ids.is_empty() {
                break;
            }
            assert!(ids.len() <= size, "a page never exceeds its limit");
            walked.extend(ids);
            offset += size;
        }
        assert_eq!(
            walked, full,
            "pages of {size} must concatenate to the full-window run"
        );
    }
    // A page that straddles the prefix/tail boundary sees the same rows the
    // full-window run put there.
    let straddle = home.json(&["search", QUERY, "--k", "4", "--offset", "18"]);
    assert_eq!(hit_ids(&straddle, "memory_id"), full[18..22].to_vec());

    let empty = home.json(&["search", QUERY, "--offset", "500"]);
    assert!(
        hit_ids(&empty, "memory_id").is_empty(),
        "an offset past the window returns nothing"
    );
    let nothing = home.json(&["search", "zzzzzzzz-no-such-token"]);
    assert!(hit_ids(&nothing, "memory_id").is_empty());
}

// ------------------------------------------------------------------ AC-7

#[test]
fn tail_is_preserved() {
    let home = Home::new();
    home.write_config(&enabled_block(2));
    seed(&home);
    let query = QUERY;
    let reranked = home.json(&["search", query, "--k", "0"]);

    let plain = Home::new();
    plain.write_config("");
    seed(&plain);
    let deterministic = plain.json(&["search", query, "--k", "0"]);

    let a = reranked["hits"].as_array().expect("hits");
    let b = deterministic["hits"].as_array().expect("hits");
    assert!(a.len() > 2 && a.len() == b.len(), "same ranked window");
    for i in 2..a.len() {
        assert_eq!(
            a[i]["memory_id"], b[i]["memory_id"],
            "position {i} is below the prefix and must not move"
        );
        assert_eq!(
            a[i]["score_parts"], b[i]["score_parts"],
            "the deterministic breakdown at position {i} must be untouched"
        );
    }
    assert_eq!(reranked["learned"]["prefix"], serde_json::json!(2));
}

// ------------------------------------------------------------------ AC-8

#[test]
fn every_refusal_restores_the_order() {
    let plain = Home::new();
    plain.write_config("");
    seed(&plain);
    let query = QUERY;
    let baseline = plain.json(&["search", query, "--k", "0"]);
    let baseline_hits = baseline["hits"].as_array().expect("hits").clone();

    for (name, command, extra) in refusals() {
        let home = Home::new();
        home.write_config(&format!(
            "[rerank]\nenabled = true\ncommand = {command}\nmodel = \"lexical-overlap@1\"\n{extra}"
        ));
        seed(&home);
        let v = home.json(&["search", query, "--k", "0"]);
        assert_eq!(
            v["learned"]["applied"],
            serde_json::json!(false),
            "{name} must decline"
        );
        assert!(
            v["learned"]["fallback"].as_str().is_some(),
            "{name} must name its fallback: {v}"
        );
        assert!(
            v["learned"]["scores"]
                .as_array()
                .is_some_and(std::vec::Vec::is_empty),
            "{name} scored nothing"
        );
        let hits = v["hits"].as_array().expect("hits");
        assert_eq!(hits.len(), baseline_hits.len(), "{name} kept every hit");
        for (i, (a, b)) in hits.iter().zip(baseline_hits.iter()).enumerate() {
            assert_eq!(a["memory_id"], b["memory_id"], "{name} position {i}");
            assert_eq!(a["score_parts"], b["score_parts"], "{name} parts at {i}");
        }
    }
}

/// The five refusal shapes, each a real child process.
fn refusals() -> Vec<(&'static str, String, &'static str)> {
    vec![
        (
            "non-zero exit (the #212 identity gate's shape)",
            "[\"python3\", \"-c\", \"import sys; sys.stdin.read(); sys.exit(65)\"]".to_string(),
            "",
        ),
        (
            "malformed response",
            "[\"python3\", \"-c\", \"import sys; sys.stdin.read(); print('not json')\"]"
                .to_string(),
            "",
        ),
        (
            "wrong model echo",
            "[\"python3\", {}, \"score\", \"--scoring\", \"lexical-overlap\", \"--model-label\", \"someone-elses-model@9\"]"
                .replace("{}", &format!("{:?}", backend_path()))
                ,
            "",
        ),
        (
            "a missing score",
            "[\"python3\", \"-c\", \"import sys,json; r=json.load(sys.stdin); print(json.dumps({'protocol_version':1,'request_id':r['request_id'],'model':r['model'],'adapter':r['adapter'],'score_direction':'higher_is_better','scores':[]}))\"]"
                .to_string(),
            "",
        ),
        (
            "an over-budget scorer",
            "[\"python3\", \"-c\", \"import sys,time; sys.stdin.read(); time.sleep(5)\"]"
                .to_string(),
            "timeout_ms = 300\n",
        ),
    ]
}

// ------------------------------------------------------------------ AC-9

#[test]
fn dedicated_and_single_domain_find_agree() {
    let home = Home::new();
    home.write_config(&enabled_block(50));
    seed(&home);
    let query = QUERY;
    let search = hit_ids(&home.json(&["search", query]), "memory_id");
    let find = hit_ids(&home.json(&["find", query, "--domain", "memory"]), "id");
    assert_eq!(
        search, find,
        "search and find --domain memory must order identically with the stage on"
    );
    let none = hit_ids(
        &home.json(&["search", "zzzzzzzz-no-such-token"]),
        "memory_id",
    );
    let none_find = hit_ids(
        &home.json(&["find", "zzzzzzzz-no-such-token", "--domain", "memory"]),
        "id",
    );
    assert_eq!(none, none_find, "a zero-hit query agrees too");
}

// ------------------------------------------------------------------ AC-11

#[test]
fn access_tracking_policy_is_preserved() {
    let home = Home::new();
    home.write_config(&enabled_block(50));
    let ids = seed(&home);
    let query = QUERY;

    // A head window bumps exactly the returned ids and nothing else.
    let before = access_counts(&home, &ids);
    let head = tracked_json(&home, &["search", query, "--k", "2"]);
    let returned = hit_ids(&head, "memory_id");
    assert_eq!(returned.len(), 2);
    let after = access_counts(&home, &ids);
    for id in &ids {
        let delta = after[id] - before[id];
        let expected = i64::from(returned.contains(id));
        assert_eq!(
            delta, expected,
            "only the returned head page may be reinforced; {id} moved by {delta}"
        );
    }

    // A deep page bumps nothing and still writes a query log row.
    let before = access_counts(&home, &ids);
    let deep = tracked_json(&home, &["search", query, "--k", "2", "--offset", "2"]);
    assert!(
        deep["query_id"].as_str().is_some(),
        "a deep page still logs: {deep}"
    );
    let after = access_counts(&home, &ids);
    for id in &ids {
        assert_eq!(
            after[id] - before[id],
            0,
            "a deep page must reinforce nothing; {id} moved"
        );
    }
}

/// `context`'s own code-reference bump is gated the same way. Memory-only
/// corpora have no code refs, so this asserts the memory half plus the absence
/// of any bump on a deep page, which is the shared gate (#201).
#[test]
fn context_reinforces_only_a_head_window() {
    let home = Home::new();
    home.write_config(&enabled_block(50));
    let ids = seed(&home);
    let before = access_counts(&home, &ids);
    tracked_json(&home, &["context", QUERY, "--k", "2"]);
    let mid = access_counts(&home, &ids);
    let bumped: usize = ids.iter().filter(|id| mid[*id] > before[*id]).count();
    assert_eq!(bumped, 2, "the head page of the bundle is reinforced");
    tracked_json(&home, &["context", QUERY, "--k", "2", "--offset", "2"]);
    let after = access_counts(&home, &ids);
    for id in &ids {
        assert_eq!(after[id], mid[id], "a deep context page reinforces nothing");
    }
}

/// `context`'s own code-reference bump is a SEPARATE write from the memory one
/// — it fires after the bundle is assembled, over refs derived from the page
/// that was returned — and #201 gates it on a head window too. Reranking
/// changes which memories the head contains, so this asserts the gate survives
/// with a learned stage active.
///
/// `tests/api__context.rs::a_deep_context_page_does_not_bump_code_ref_access_counts`
/// covers the same gate with the stage off.
#[test]
fn context_code_reference_bumps_survive_reranking() {
    let home = Home::new();
    home.write_config(&enabled_block(50));
    let first = home.save_with("advisory lock guards the migration runner", 3);
    let second = home.save_with("advisory lock protects every schema rebuild", 3);
    seed_code_ref(&home, &first, "alpha.rs", "alpha_run");
    seed_code_ref(&home, &second, "bravo.rs", "bravo_run");

    let before = bumped_symbols(&home);
    assert_eq!(before, 0, "a fresh code index carries no access writes");
    let head = tracked_json(&home, &["context", "advisory lock", "--k", "1"]);
    assert_eq!(
        head["learned"]["applied"],
        serde_json::json!(true),
        "the stage really ran: {head}"
    );
    let after_head = bumped_symbols(&home);
    assert!(
        after_head > 0,
        "the head page must reinforce the code refs it surfaced, \
         or the negative half below is vacuous"
    );

    tracked_json(
        &home,
        &["context", "advisory lock", "--k", "1", "--offset", "1"],
    );
    assert_eq!(
        bumped_symbols(&home),
        after_head,
        "a deep context page must reinforce no code reference"
    );
}

// ------------------------------------------------------------------ AC-13

#[test]
fn capture_is_suppressed_while_reranking() {
    let home = Home::new();
    home.write_config(&format!(
        "{}\n[observations]\nenabled = true\n",
        enabled_block(50)
    ));
    seed(&home);
    let v = tracked_json(&home, &["find", QUERY]);
    assert_eq!(
        v["observation_id"],
        Value::Null,
        "capture must be suppressed while a learned stage is active: {v}"
    );
    assert_eq!(v["learned"]["applied"], serde_json::json!(true));
    assert_eq!(
        observation_rows(&home),
        0,
        "no candidate observation row may be written"
    );

    // The same configuration without the stage does capture.
    let plain = Home::new();
    plain.write_config("[observations]\nenabled = true\n");
    seed(&plain);
    let w = tracked_json(&plain, &["find", QUERY]);
    assert!(
        w["observation_id"].as_str().is_some(),
        "capture works when nothing reorders the pool: {w}"
    );
    assert_eq!(observation_rows(&plain), 1);
}

// ------------------------------------------------------------------ AC-14

#[test]
fn descendant_holding_the_pipe_times_out() {
    let home = Home::new();
    // The direct child exits immediately, but its descendant inherits stdout and
    // holds the pipe open. #211's Non-Goal 6 does not reap descendants; the
    // deadline is what bounds the run.
    home.write_config(
        "[rerank]\nenabled = true\ncommand = [\"python3\", \"-c\", \"import subprocess,sys; sys.stdin.read(); subprocess.Popen(['sleep','5']); sys.exit(0)\"]\nmodel = \"lexical-overlap@1\"\ntimeout_ms = 400\n",
    );
    seed(&home);
    let started = std::time::Instant::now();
    let v = home.json(&["search", QUERY]);
    let elapsed = started.elapsed();
    assert_eq!(v["learned"]["applied"], serde_json::json!(false));
    assert!(
        v["learned"]["fallback"]
            .as_str()
            .is_some_and(|f| f.contains("timed out")),
        "the run must be bounded by its deadline: {v}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(4),
        "a descendant holding the pipe must not extend the request: {elapsed:?}"
    );
    assert!(
        !hit_ids(&v, "memory_id").is_empty(),
        "the deterministic ranking still answers"
    );
}

// ------------------------------------------------------------------ AC-15

#[test]
fn filters_apply_before_materialization() {
    let home = Home::new();
    let log = home.workspace().join("requests.log");
    let script = recording_proxy(home.workspace(), &log, Some("stdin"));
    home.write_config(&format!(
        "[rerank]\nenabled = true\ncommand = [{:?}]\nmodel = \"lexical-overlap@1\"\n",
        script.to_string_lossy()
    ));
    // Two kinds; the filter must exclude one of them before anything is
    // materialized for the scorer.
    home.run_ok(&[
        "--json",
        "save",
        "sqlite busy timeout pool retry",
        "--kind",
        "bug",
    ]);
    home.run_ok(&[
        "--json",
        "save",
        "sqlite busy timeout pool decision record",
        "--kind",
        "decision",
    ]);
    let v = home.json(&["find", "sqlite busy timeout pool", "--kind", "decision"]);
    assert_eq!(v["learned"]["applied"], serde_json::json!(true));
    let body = std::fs::read_to_string(&log).expect("recorded request");
    let request: Value = serde_json::from_str(body.trim()).expect("request json");
    let candidates = request["candidates"].as_array().expect("candidates");
    assert_eq!(
        candidates.len(),
        1,
        "only the in-filter candidate may reach the scorer: {body}"
    );
    assert!(
        candidates[0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("decision record")),
        "the surviving candidate is the one the filter kept: {body}"
    );
}

// ------------------------------------------------------------------ AC-17

#[test]
fn records_the_extra_retrieval_cost() {
    let disabled = Home::new();
    disabled.write_config("");
    seed(&disabled);
    let enabled = Home::new();
    enabled.write_config(&enabled_block(50));
    seed(&enabled);

    // Wall-clock is measured but not asserted on: comparing two separate
    // process invocations is a CI timing race, and the real measurement lives
    // in the design doc's "Measured cost" section.
    let off = measure(&disabled, &["search", QUERY]);
    let on = measure(&enabled, &["search", QUERY]);
    let pool = on["learned"]["pool"].as_u64().expect("pool");
    let scored = on["learned"]["prefix"].as_u64().expect("prefix");
    // The measurement itself is recorded in the design doc's "Measured cost"
    // section; what is asserted here is that both paths answer, that the
    // enabled run really did rank the bounded universe it reports, and that
    // reranking cost something rather than being silently skipped.
    assert!(!hit_ids(&off, "memory_id").is_empty());
    assert!(!hit_ids(&on, "memory_id").is_empty());
    assert_eq!(
        scored,
        pool.min(50),
        "an enabled run scores the whole prefix it configured, over the pool it ranked"
    );
    assert!(
        on["learned"]["elapsed_ms"]
            .as_u64()
            .is_some_and(|ms| ms > 0),
        "and reports a measured, non-zero inference cost: {}",
        on["learned"]
    );
}

// ------------------------------------------------------------------ helpers
fn backend_path() -> &'static str {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/integrations/reranker/comemory_rerank.py"
    )
}

/// A recording proxy: appends one line per invocation to `log` (or the whole
/// request body when `mode` is `Some("stdin")`) and then EXECUTES the real
/// shipped backend, so the search under test stays a genuine end-to-end run.
fn recording_proxy(workspace: &Path, log: &Path, mode: Option<&str>) -> PathBuf {
    let script = workspace.join(format!("proxy{}.py", mode.unwrap_or("count")));
    let record = if mode == Some("stdin") {
        "log.write(body.decode())"
    } else {
        "log.write('invoked\\n')"
    };
    std::fs::write(
        &script,
        format!(
            "#!/usr/bin/env python3\n\
             import subprocess, sys\n\
             body = sys.stdin.buffer.read()\n\
             with open({log:?}, 'a') as log:\n    {record}\n\
             p = subprocess.run([sys.executable, {backend:?}, 'score', '--scoring', 'lexical-overlap'],\n\
             \x20               input=body, capture_output=True)\n\
             sys.stdout.buffer.write(p.stdout)\n\
             sys.stderr.buffer.write(p.stderr)\n\
             sys.exit(p.returncode)\n",
            log = log.to_string_lossy(),
            record = record,
            backend = backend_path(),
        ),
    )
    .expect("write proxy");
    make_executable(&script);
    script
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut perms = std::fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod");
    }
}

/// A `--json` run with access tracking left ON, for the telemetry tests —
/// `Home::json` sets `COMEMORY_DISABLE_ACCESS_TRACKING`, and these must not.
fn tracked_json(home: &Home, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let out = home.bin().args(&all).output().expect("run comemory");
    assert!(
        out.status.success(),
        "comemory {all:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("not JSON: {e}\n{stdout}"))
}

/// `memories.access_count` for each id, read straight out of the real database.
fn access_counts(home: &Home, ids: &[String]) -> std::collections::HashMap<String, i64> {
    let conn = comemory::store::connection::open(home.data_dir().join("comemory.db"))
        .expect("open comemory.db");
    ids.iter()
        .map(|id| {
            let n: i64 = conn
                .query_row(
                    "SELECT access_count FROM memories WHERE id = ?1",
                    [id],
                    |r| r.get(0),
                )
                .expect("access_count");
            (id.clone(), n)
        })
        .collect()
}

/// Insert one `code_symbols` row through the production writer and point
/// `memory_id` at it with the `references_symbol` edge `bundle::assemble`
/// resolves, so `context` really surfaces a code reference.
fn seed_code_ref(home: &Home, memory_id: &str, path: &str, symbol: &str) {
    let conn = comemory::store::connection::open(home.data_dir().join("comemory.db"))
        .expect("open comemory.db");
    comemory::store::code_row::insert(
        &conn,
        &comemory::store::code_row::CodeSymbolRow {
            repo: "demo",
            path,
            blob_oid: "oid",
            symbol,
            kind: "function",
            lang: "rust",
            line_start: 1,
            line_end: 10,
            snippet: "fn body() {}",
            simhash: 0,
            parent_id: None,
        },
    )
    .expect("insert code symbol");
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
         VALUES('memory',?1,'symbol',?2,'references_symbol','t')",
        rusqlite::params![memory_id, format!("demo:{path}:{symbol}")],
    )
    .expect("seed references_symbol edge");
}

/// Index two same-named functions in one file. Their candidate observation
/// references are byte-identical — the code form is `(repo, path, symbol,
/// blob_oid)` while `code_symbols` is unique on `(repo, path, symbol,
/// line_start)` — so this is the fixture that would decline the whole stage if
/// the wire id were the reference string.
fn seed_colliding_symbols(home: &Home) {
    let conn = comemory::store::connection::open(home.data_dir().join("comemory.db"))
        .expect("open comemory.db");
    for line in [10i64, 40] {
        let id = comemory::store::code_row::insert(
            &conn,
            &comemory::store::code_row::CodeSymbolRow {
                repo: "demo",
                path: "alpha.rs",
                blob_oid: "oid",
                symbol: "new",
                kind: "function",
                lang: "rust",
                line_start: line,
                line_end: line + 5,
                snippet: "fn new() { sqlite busy timeout pool }",
                simhash: 0,
                parent_id: None,
            },
        )
        .expect("insert code symbol");
        comemory::store::fts::index_code(
            &conn,
            id,
            "new",
            "fn new() { sqlite busy timeout pool }",
            "alpha.rs",
        )
        .expect("index code fts");
    }
}

/// Symbols carrying any access-tracking write at all — either column.
fn bumped_symbols(home: &Home) -> i64 {
    let conn = comemory::store::connection::open(home.data_dir().join("comemory.db"))
        .expect("open comemory.db");
    conn.query_row(
        "SELECT COUNT(*) FROM code_symbols WHERE access_count <> 0 OR last_accessed IS NOT NULL",
        [],
        |r| r.get(0),
    )
    .expect("count bumped symbols")
}

/// How many candidate observations the store holds.
fn observation_rows(home: &Home) -> i64 {
    let conn = comemory::store::connection::open(home.data_dir().join("comemory.db"))
        .expect("open comemory.db");
    conn.query_row(
        "SELECT COUNT(*) FROM candidate_query_observations",
        [],
        |r| r.get(0),
    )
    .expect("count observations")
}

/// One `--json` run, with its wall clock discarded: the measurement that
/// matters is recorded in the design doc, and a cross-process timing
/// comparison is not something to assert on in CI.
fn measure(home: &Home, args: &[&str]) -> Value {
    home.json(args)
}
