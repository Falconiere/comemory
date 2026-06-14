//! Mirror tests for `src/cli/lazy_reindex.rs`.
//!
//! Two layers, no mocks:
//!   1. The pure [`should_reindex`] decision is exercised exhaustively —
//!      Lazy never-indexed / HEAD-moved / HEAD-unchanged / already-triggered
//!      (debounce) / time-debounce, plus Off and Hook always-false. The
//!      trigger-marker encode/decode round-trip is asserted too.
//!   2. Real-binary integration: a `search-code` run is driven against a real
//!      git fixture repo indexed by the real `comemory index-code`, with the
//!      process CWD set inside the repo so the lazy probe discovers it. We
//!      assert the search still returns correct results AND that the
//!      `schema_meta` debounce marker reflects the DECISION (absent for
//!      `off`/fresh, present after HEAD moves) — never the spawned process,
//!      which is non-deterministic.

use assert_cmd::Command;
use comemory::cli::lazy_reindex::{LastTrigger, encode_trigger, parse_trigger, should_reindex};
use comemory::config::AutoReindexMode;
use serde_json::Value;

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

const HEAD_A: &str = "1111111111111111111111111111111111111111";
const HEAD_B: &str = "2222222222222222222222222222222222222222";

fn trigger(head: &str, at_millis: u128) -> LastTrigger {
    LastTrigger {
        head: head.to_string(),
        at_millis,
    }
}

// ── pure decision: Lazy ────────────────────────────────────────────────────

#[test]
fn lazy_never_indexed_is_stale() {
    // No last-indexed head (never indexed) and no prior trigger → fire.
    assert!(should_reindex(
        &AutoReindexMode::Lazy,
        HEAD_A,
        None,
        None,
        1_000,
        200,
    ));
}

#[test]
fn lazy_head_moved_is_stale() {
    // Indexed at HEAD_A, now on HEAD_B, no prior trigger → fire.
    assert!(should_reindex(
        &AutoReindexMode::Lazy,
        HEAD_B,
        Some(HEAD_A),
        None,
        1_000,
        200,
    ));
}

#[test]
fn lazy_head_unchanged_is_fresh() {
    // Indexed at the current HEAD → no work.
    assert!(!should_reindex(
        &AutoReindexMode::Lazy,
        HEAD_A,
        Some(HEAD_A),
        None,
        1_000,
        200,
    ));
}

#[test]
fn lazy_already_triggered_for_this_head_is_debounced() {
    // HEAD moved to B and a trigger already fired for B (even long ago) →
    // suppress: the in-flight/just-spawned index-code will advance the cursor.
    assert!(!should_reindex(
        &AutoReindexMode::Lazy,
        HEAD_B,
        Some(HEAD_A),
        Some(&trigger(HEAD_B, 0)),
        10_000_000,
        200,
    ));
}

#[test]
fn lazy_time_debounce_suppresses_within_window() {
    // Stale and the prior trigger fired for a DIFFERENT head, but only 50ms
    // ago against a 200ms window → suppress (herd guard).
    assert!(!should_reindex(
        &AutoReindexMode::Lazy,
        HEAD_B,
        Some(HEAD_A),
        Some(&trigger("3333333333333333333333333333333333333333", 950)),
        1_000,
        200,
    ));
}

#[test]
fn lazy_time_debounce_allows_after_window() {
    // Same as above but the prior trigger is 300ms old (> 200ms window) → fire.
    assert!(should_reindex(
        &AutoReindexMode::Lazy,
        HEAD_B,
        Some(HEAD_A),
        Some(&trigger("3333333333333333333333333333333333333333", 700)),
        1_000,
        200,
    ));
}

#[test]
fn lazy_zero_threshold_only_debounces_on_head() {
    // threshold_ms = 0 disables the time window: a stale repo whose prior
    // trigger was for a different head fires immediately.
    assert!(should_reindex(
        &AutoReindexMode::Lazy,
        HEAD_B,
        Some(HEAD_A),
        Some(&trigger(HEAD_A, 1_000)),
        1_000,
        0,
    ));
}

// ── pure decision: Off / Hook never fire ───────────────────────────────────

#[test]
fn off_never_triggers_even_when_stale() {
    assert!(!should_reindex(
        &AutoReindexMode::Off,
        HEAD_B,
        Some(HEAD_A),
        None,
        1_000,
        200,
    ));
    assert!(!should_reindex(
        &AutoReindexMode::Off,
        HEAD_A,
        None,
        None,
        1_000,
        200,
    ));
}

#[test]
fn hook_never_triggers_in_process_even_when_stale() {
    assert!(!should_reindex(
        &AutoReindexMode::Hook,
        HEAD_B,
        Some(HEAD_A),
        None,
        1_000,
        200,
    ));
    assert!(!should_reindex(
        &AutoReindexMode::Hook,
        HEAD_A,
        None,
        None,
        1_000,
        200,
    ));
}

// ── trigger-marker round-trip ──────────────────────────────────────────────

#[test]
fn trigger_marker_round_trips() {
    let raw = encode_trigger(HEAD_A, 1_700_000_000_123);
    let parsed = parse_trigger(&raw).expect("round-trip parses");
    assert_eq!(parsed.head, HEAD_A);
    assert_eq!(parsed.at_millis, 1_700_000_000_123);
}

#[test]
fn malformed_trigger_markers_are_none() {
    assert!(parse_trigger("no-separator").is_none());
    assert!(parse_trigger("|123").is_none()); // empty head
    assert!(parse_trigger(&format!("{HEAD_A}|not-a-number")).is_none());
}

// ── real-binary integration ────────────────────────────────────────────────

/// Build a single-file git fixture repo and return its path.
fn build_repo(root: &std::path::Path) -> std::path::PathBuf {
    let repo = root.join("lazyrepo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[("alpha.rs", "fn alpha_router() {}\nfn beta_helper() {}\n")],
        "init",
    );
    repo
}

/// Run `comemory index-code` for `repo` into the data dir at `home`.
fn index(home: &std::path::Path, repo: &std::path::Path) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home)
        .args(["index-code", "--repo", "lazyrepo", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();
}

/// Run `comemory search-code` from inside `repo` (so the lazy probe
/// discovers it) with `mode`, returning parsed `--json` stdout.
fn search_code(home: &std::path::Path, repo: &std::path::Path, mode: &str, query: &str) -> Value {
    let assert = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home)
        .env("COMEMORY_INDEXING_AUTO_REINDEX", mode)
        .current_dir(repo)
        .args(["search-code", query, "--repo", "lazyrepo", "--json"])
        .assert()
        .success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("json parse ({e}): {out}"))
}

/// The persisted `lazy_reindex_head:<repo>` marker, or `None` when absent.
fn trigger_marker(home: &std::path::Path) -> Option<String> {
    let db = home.join("comemory.db");
    let conn = rusqlite::Connection::open(db).expect("open db");
    conn.query_row(
        "SELECT value FROM schema_meta WHERE key = 'lazy_reindex_head:lazyrepo'",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

fn hit_symbols(v: &Value) -> Vec<String> {
    v["hits"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|h| h["symbol"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn off_mode_does_not_trigger_and_returns_results() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join(".comemory");
    let repo = build_repo(tmp.path());
    index(&home, &repo);

    let out = search_code(&home, &repo, "off", "alpha router");
    // Search behaves exactly as before: the indexed symbol is found.
    assert!(
        hit_symbols(&out).iter().any(|s| s == "alpha_router"),
        "off-mode search must still return alpha_router: {out}"
    );
    // No lazy trigger was recorded.
    assert!(
        trigger_marker(&home).is_none(),
        "off mode must never write a lazy reindex trigger marker"
    );
}

#[test]
fn lazy_fresh_and_current_index_does_not_trigger() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join(".comemory");
    let repo = build_repo(tmp.path());
    index(&home, &repo);

    let out = search_code(&home, &repo, "lazy", "alpha router");
    assert!(
        hit_symbols(&out).iter().any(|s| s == "alpha_router"),
        "lazy-mode search must still return alpha_router: {out}"
    );
    // Index is current (HEAD == last_mined_commit) → decision is false →
    // no trigger marker written.
    assert!(
        trigger_marker(&home).is_none(),
        "lazy must not trigger when the index is fresh for the current HEAD"
    );
}

#[test]
fn lazy_after_head_moves_records_a_trigger() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join(".comemory");
    let repo = build_repo(tmp.path());
    index(&home, &repo);

    // Move HEAD with a new commit AFTER indexing so the cheap probe sees a
    // HEAD that differs from repo_marker.last_mined_commit.
    git_commit::commit_files(&repo, &[("gamma.rs", "fn gamma_router() {}\n")], "second");

    let out = search_code(&home, &repo, "lazy", "alpha router");
    // The search still returns against the current (pre-reindex) index.
    assert!(
        hit_symbols(&out).iter().any(|s| s == "alpha_router"),
        "search must return immediately against the current index: {out}"
    );
    // The DECISION fired: a trigger marker keyed to the NEW head exists.
    let head_new = comemory::git_utils::current_head(&repo).expect("head");
    let marker = trigger_marker(&home).expect("a trigger marker after HEAD moved");
    let parsed = parse_trigger(&marker).expect("marker parses");
    assert_eq!(
        parsed.head, head_new,
        "trigger marker must record the current HEAD"
    );
}

#[test]
fn lazy_skips_when_cwd_is_a_different_checkout_for_the_label() {
    // The repo is indexed under label `lazyrepo` from `repo`, but the search
    // is run with `--repo lazyrepo` from a SEPARATE git checkout `other` whose
    // HEAD differs. The HEADs disagree, but the CWD is not the indexed
    // checkout (its root differs from repo_marker.root_path), so the
    // label-collision guard must SKIP — reindexing `other` under `lazyrepo`
    // would corrupt the indexed repo's rows.
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join(".comemory");
    let repo = build_repo(tmp.path());
    index(&home, &repo);

    // A second, unrelated checkout with its own (different) HEAD.
    let other = tmp.path().join("other");
    git_repo::init_repo(&other);
    git_commit::commit_files(&other, &[("zeta.rs", "fn zeta() {}\n")], "init-other");

    let out = search_code(&home, &other, "lazy", "alpha router");
    // Search still returns the indexed repo's symbol.
    assert!(
        hit_symbols(&out).iter().any(|s| s == "alpha_router"),
        "search must still return alpha_router from the indexed repo: {out}"
    );
    // Guard tripped: no trigger marker written despite divergent HEADs.
    assert!(
        trigger_marker(&home).is_none(),
        "lazy must not trigger when the CWD is a different checkout for the label"
    );
}
