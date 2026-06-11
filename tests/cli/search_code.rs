//! Task 13: `comemory search-code` — ranked code search over the indexed
//! `code_symbols` table, with telemetry (access bump + `retrieval_log`
//! row tagged `source='search-code'`) and a feedback-ready `query_id`.
//!
//! Every test runs the real binary against a real git fixture repo that
//! was indexed with the real `comemory index-code` — no mock data.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::tempdir;

use super::git_setup;

/// Build a two-language fixture repo: a Rust file with two functions and
/// a Python file whose function shares the `alpha`/`router` subtokens, so
/// the same query reaches both languages until `--lang` narrows it.
fn build_code_repo(root: &std::path::Path) -> std::path::PathBuf {
    let repo = root.join("code-repo");
    git_setup::init_repo(&repo);
    git_setup::commit_files(
        &repo,
        &[
            (
                "alpha.rs",
                "fn alpha_router() {}\nfn unrelated_helper() {}\n",
            ),
            ("beta.py", "def alpha_router_py():\n    pass\n"),
        ],
        "init",
    );
    repo
}

/// Index `repo` into the comemory data dir rooted at `home`.
fn index_repo(home: &tempfile::TempDir, repo: &std::path::Path) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "r", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();
}

/// Run `comemory search-code` with `args`, return parsed `--json` stdout.
fn search_code_json(home: &tempfile::TempDir, args: &[&str]) -> Value {
    let mut cmd = Command::cargo_bin("comemory").expect("bin");
    cmd.env("COMEMORY_DATA_DIR", home.path())
        .args(["search-code"])
        .args(args)
        .arg("--json");
    let assert = cmd.assert().success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("json parse ({e}): {out}"))
}

fn open_db(home: &tempfile::TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open_with_flags(
        home.path().join("comemory.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open db read-only")
}

#[test]
fn search_code_json_contract_telemetry_and_access_bump() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = build_code_repo(workspace.path());
    index_repo(&home, &repo);

    // Fresh index: no symbol has been accessed yet.
    {
        let db = open_db(&home);
        let touched: i64 = db
            .query_row(
                "SELECT count(*) FROM code_symbols WHERE access_count > 0",
                [],
                |r| r.get(0),
            )
            .expect("count accessed");
        assert_eq!(touched, 0, "fresh index must have zero accessed symbols");
    }

    let v = search_code_json(&home, &["alpha_router"]);
    let hits = v.get("hits").and_then(Value::as_array).expect("hits array");
    assert!(!hits.is_empty(), "expected hits for alpha_router: {v}");
    for hit in hits {
        assert!(hit["symbol_id"].is_i64(), "symbol_id: {hit}");
        assert_eq!(hit["repo"].as_str(), Some("r"), "repo: {hit}");
        assert!(hit["path"].is_string(), "path: {hit}");
        assert!(hit["symbol"].is_string(), "symbol: {hit}");
        assert!(hit["kind"].is_string(), "kind: {hit}");
        assert!(hit["lang"].is_string(), "lang: {hit}");
        let lines = hit["lines"].as_array().expect("lines array");
        assert_eq!(lines.len(), 2, "lines must be [start, end]: {hit}");
        assert!(lines[0].is_i64() && lines[1].is_i64(), "line nums: {hit}");
        assert!(hit["score"].is_number(), "score: {hit}");
        assert!(hit["source"].is_string(), "source: {hit}");
        let parts = &hit["score_parts"];
        for key in [
            "relevance",
            "rank",
            "activation",
            "affinity",
            "feedback",
            "final_score",
        ] {
            assert!(parts[key].is_number(), "score_parts.{key}: {hit}");
        }
    }

    // query_id shape + a retrieval_log row tagged source='search-code'
    // with NULL repo/kind columns (no filters were passed).
    let qid = v
        .get("query_id")
        .and_then(Value::as_str)
        .expect("query_id in envelope")
        .to_string();
    assert!(
        comemory::stats::feedback::is_valid_query_id(&qid),
        "query_id shape, got: {qid}"
    );
    let db = open_db(&home);
    let (source, repo_col, kind_col, returned): (String, Option<String>, Option<String>, String) =
        db.query_row(
            "SELECT source, repo, kind, returned_ids FROM retrieval_log WHERE query_id = ?1",
            [&qid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("retrieval_log row for emitted query_id");
    assert_eq!(source, "search-code");
    assert_eq!(repo_col, None, "no --repo filter → NULL repo column");
    assert_eq!(kind_col, None, "no --lang filter → NULL kind column");

    // returned_ids is a JSON array of symbol-id strings matching the hits.
    let logged_ids: Vec<String> = serde_json::from_str(&returned).expect("returned_ids json");
    let hit_ids: Vec<String> = hits
        .iter()
        .map(|h| h["symbol_id"].as_i64().expect("symbol_id i64").to_string())
        .collect();
    assert_eq!(logged_ids, hit_ids, "logged ids must match emitted hits");

    // access_count bumped exactly once on every returned symbol.
    for id in &hit_ids {
        let count: i64 = db
            .query_row(
                "SELECT access_count FROM code_symbols WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .expect("access_count for returned id");
        assert_eq!(count, 1, "returned symbol {id} must be bumped to 1");
    }
}

#[test]
fn repo_and_lang_filters_are_logged_to_retrieval_log() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = build_code_repo(workspace.path());
    index_repo(&home, &repo);

    let v = search_code_json(&home, &["alpha_router", "--repo", "r", "--lang", "rust"]);
    let qid = v
        .get("query_id")
        .and_then(Value::as_str)
        .expect("query_id")
        .to_string();
    let db = open_db(&home);
    let (repo_col, kind_col): (Option<String>, Option<String>) = db
        .query_row(
            "SELECT repo, kind FROM retrieval_log WHERE query_id = ?1",
            [&qid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("retrieval_log row");
    assert_eq!(repo_col.as_deref(), Some("r"), "--repo filter logged");
    assert_eq!(kind_col.as_deref(), Some("rust"), "--lang filter logged");
}

#[test]
fn lang_filter_narrows_hits_to_one_language() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = build_code_repo(workspace.path());
    index_repo(&home, &repo);

    // Unfiltered, the subtoken query reaches both languages.
    let v = search_code_json(&home, &["alpha router"]);
    let langs: Vec<&str> = v["hits"]
        .as_array()
        .expect("hits")
        .iter()
        .map(|h| h["lang"].as_str().expect("lang"))
        .collect();
    assert!(langs.contains(&"rust"), "rust hit expected: {langs:?}");
    assert!(langs.contains(&"python"), "python hit expected: {langs:?}");

    let v = search_code_json(&home, &["alpha router", "--lang", "rust"]);
    let hits = v["hits"].as_array().expect("hits");
    assert!(!hits.is_empty(), "rust-filtered hits expected");
    assert!(
        hits.iter().all(|h| h["lang"].as_str() == Some("rust")),
        "--lang rust must drop non-rust hits: {v}"
    );

    let v = search_code_json(&home, &["alpha router", "--lang", "python"]);
    let hits = v["hits"].as_array().expect("hits");
    assert!(!hits.is_empty(), "python-filtered hits expected");
    assert!(
        hits.iter().all(|h| h["lang"].as_str() == Some("python")),
        "--lang python must drop non-python hits: {v}"
    );
}

#[test]
fn empty_index_exits_zero_and_hints_index_code() {
    // Fresh data dir, nothing indexed: exit 0 with a TTY hint pointing at
    // `comemory index-code` instead of a silent empty result.
    let home = tempdir().expect("tempdir");
    let assert = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["search-code", "anything at all"])
        .assert()
        .success();
    let out = assert.get_output();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("comemory index-code"),
        "empty-index hint must mention comemory index-code: {combined}"
    );
}

#[test]
fn tty_mode_shows_score_path_lines_symbol_kind_and_footer() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = build_code_repo(workspace.path());
    index_repo(&home, &repo);

    let assert = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["search-code", "alpha_router", "--lang", "rust"])
        .assert()
        .success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    // `score path:start-end symbol (kind)` — alpha_router sits on line 1.
    let hit_line = out
        .lines()
        .find(|l| l.contains("alpha.rs:1-1"))
        .unwrap_or_else(|| panic!("expected a path:start-end hit line, got: {out}"));
    assert!(hit_line.contains("alpha_router"), "symbol: {hit_line}");
    assert!(hit_line.contains("(function)"), "kind: {hit_line}");
    // The line carries a 3-decimal blended score (priors can push the
    // product above 1.0, so only the `d.ddd` shape is pinned). Scanned
    // byte-wise to stay robust to ANSI color escapes around the token.
    let b = hit_line.as_bytes();
    let has_score = (0..b.len().saturating_sub(4)).any(|i| {
        b[i].is_ascii_digit() && b[i + 1] == b'.' && b[i + 2..i + 5].iter().all(u8::is_ascii_digit)
    });
    assert!(has_score, "3-decimal score prefix: {hit_line}");
    // The query footer carries the qid and the code-flavored feedback hint.
    assert!(out.contains("query: q-"), "query footer: {out}");
    assert!(
        out.contains("--used-code"),
        "code feedback hint must reference --used-code: {out}"
    );
}
