#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! MCP stdio journey (AC-1 … AC-5) — the real `comemory mcp` binary spawned
//! as a child process and driven by the real rmcp client over its stdio.
//! Nothing is mocked: the memories are written by the `save` tool, the
//! recalls come back from the real ranker, and the provenance assertions read
//! the `feedback_events` rows that actually landed.
//!
//! One test function per acceptance criterion, named `mcp_0N_*` so the
//! scenario catalog can cite them.

use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;
use comemory::mcp::catalog::TOOLS;
use comemory::store::connection;
use comemory::store::feedback::events_for_query;
use mcp_bin::McpHome;
use serde_json::{Value, json};

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;
#[path = "common/git_worktree.rs"]
mod git_worktree;
#[path = "common/mcp_bin.rs"]
mod mcp_bin;

const REPO: &str = "demo";
const POSTGRES: &str = "Use Postgres for analytics, not the primary store\n";
const NEXTEST: &str = "Prefer nextest over cargo test for the suite\n";

/// A long-lived agent must observe a rebuilt database on its next tool call.
#[tokio::test]
async fn mcp_07_rebuild_between_calls_keeps_agents_on_the_live_store() {
    let cwd = tempfile::tempdir().unwrap();
    let home = McpHome::spawn(cwd.path(), &["--repo", REPO]).await;
    save(&home, POSTGRES, REPO).await;
    Command::new(assert_cmd::cargo::cargo_bin("comemory"))
        .arg("rebuild")
        .env("COMEMORY_DATA_DIR", home.data_dir())
        .assert()
        .success();
    let fresh = home.attach(cwd.path(), &["--repo", REPO]).await;
    let id = save(
        &fresh,
        "Freshstore discovery after database replacement",
        REPO,
    )
    .await;
    let seen = home
        .data("find", json!({"query": "Freshstore", "k": 3}))
        .await;
    assert!(
        hit_ids(&seen).contains(&id),
        "old session must read the live database"
    );
    let later = save(&home, "Oldsession writes after database replacement", REPO).await;
    let seen = fresh
        .data("find", json!({"query": "Oldsession", "k": 3}))
        .await;
    assert!(
        hit_ids(&seen).contains(&later),
        "old session must write the live database"
    );
    fresh.cancel().await;
    home.cancel().await;
}

/// Separate agent processes contend for SQLite's writer, not a shared Rust mutex.
#[tokio::test]
async fn mcp_08_concurrent_agents_save_and_find_each_others_memories() {
    let cwd = tempfile::tempdir().unwrap();
    let home = McpHome::spawn(cwd.path(), &["--repo", REPO]).await;
    let (a, b, c, d) = tokio::join!(
        home.attach(cwd.path(), &["--repo", REPO]),
        home.attach(cwd.path(), &["--repo", REPO]),
        home.attach(cwd.path(), &["--repo", REPO]),
        home.attach(cwd.path(), &["--repo", REPO]),
    );
    let (a_id, b_id, c_id, d_id) = tokio::join!(
        save(
            &a,
            "Parallelwriters alpha saves an independent observation",
            REPO
        ),
        save(
            &b,
            "Parallelwriters beta saves its own verified decision",
            REPO
        ),
        save(
            &c,
            "Parallelwriters gamma records a reproduced correction",
            REPO
        ),
        save(
            &d,
            "Parallelwriters delta preserves a measured discovery",
            REPO
        ),
    );
    let seen = home
        .data("find", json!({"query": "Parallelwriters", "k": 10}))
        .await;
    let ids = hit_ids(&seen);
    for id in [a_id, b_id, c_id, d_id] {
        assert!(
            ids.contains(&id),
            "every successful save must be indexed: {id}"
        );
    }
    // Replays must not race the same markdown staging file, and exactly one
    // writer creates the content-derived id.
    let body = "Shared replay evidence from concurrent agents. ".repeat(5000);
    let (one, two, three, four) = tokio::join!(
        a.data("save", json!({"body": body, "repo": REPO})),
        b.data("save", json!({"body": body, "repo": REPO})),
        c.data("save", json!({"body": body, "repo": REPO})),
        d.data("save", json!({"body": body, "repo": REPO})),
    );
    let replays = [one, two, three, four];
    assert_eq!(replays.iter().filter(|r| r["created"] == true).count(), 1);
    assert!(replays.iter().all(|r| r["id"] == replays[0]["id"]));
    let shown = home.data("show", json!({"id": replays[0]["id"]})).await;
    assert_eq!(shown["body"].as_str().unwrap().trim_end(), body.trim_end());
    for agent in [a, b, c, d] {
        agent.cancel().await;
    }
    home.cancel().await;
}

/// Save one memory through the `save` tool and return its 8-hex id.
async fn save(home: &McpHome, body: &str, repo: &str) -> String {
    let data = home
        .data("save", json!({ "body": body, "repo": repo }))
        .await;
    data["id"]
        .as_str()
        .unwrap_or_else(|| panic!("save returned no id: {data}"))
        .to_string()
}

/// The hit ids of a `find` result, in returned order.
fn hit_ids(data: &Value) -> Vec<String> {
    data["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["id"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// `data.query_id`, which every tracked recall must mint.
fn query_id(data: &Value) -> String {
    data["query_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no query_id in {data}"))
        .to_string()
}

/// The `feedback_events` rows recorded against `query_id`, read straight out
/// of the session's database — the verdict as stored, not as echoed.
fn stored_events(data_dir: &Path, query_id: &str) -> Vec<(String, String, String)> {
    let conn = connection::open(data_dir.join("comemory.db")).expect("open the session database");
    events_for_query(&conn, query_id)
        .expect("read feedback_events")
        .into_iter()
        .map(|r| (r.memory_id, r.verdict, r.provenance))
        .collect()
}

/// AC-1: `initialize` succeeds and `tools/list` is exactly the catalog —
/// every name, every description, an object `inputSchema` on each — and
/// `--read-only` advertises the same eleven.
#[tokio::test]
async fn mcp_01_lists_catalog() {
    let tmp = tempfile::TempDir::new().expect("cwd");
    let home = McpHome::spawn(tmp.path(), &[]).await;

    let tools = home.list_tools().await;
    // rmcp's `ToolRouter::list_all` sorts by name, so the catalog is
    // compared as a sorted set rather than in its declaration order.
    let mut listed: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    listed.sort_unstable();
    let mut expected: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
    expected.sort_unstable();
    assert_eq!(listed, expected, "tools/list must be exactly the catalog");

    for tool in &tools {
        let row = TOOLS
            .iter()
            .find(|t| t.name == tool.name.as_ref())
            .unwrap_or_else(|| panic!("listed tool `{}` is not catalogued", tool.name));
        let description = tool
            .description
            .as_ref()
            .unwrap_or_else(|| panic!("tool `{}` has no description", tool.name));
        assert!(
            !description.is_empty(),
            "tool `{}` describes nothing",
            tool.name
        );
        assert_eq!(
            description.as_ref(),
            row.description,
            "tool `{}`'s advertised description has drifted from its catalog row",
            tool.name
        );
        assert_eq!(
            tool.input_schema.get("type").and_then(Value::as_str),
            Some("object"),
            "tool `{}` must advertise an object inputSchema, got {:?}",
            tool.name,
            tool.input_schema
        );
    }

    let instructions = home.instructions();
    assert!(instructions.contains("find"), "{instructions}");
    assert!(instructions.contains("feedback"), "{instructions}");
    assert!(
        instructions.contains("no default repo"),
        "a session outside any git work tree must say so: {instructions}"
    );

    let read_only = McpHome::spawn(tmp.path(), &["--read-only"]).await;
    let ro_tools = read_only.list_tools().await;
    let mut ro_listed: Vec<&str> = ro_tools.iter().map(|t| t.name.as_ref()).collect();
    ro_listed.sort_unstable();
    assert_eq!(
        ro_listed, expected,
        "--read-only hides nothing: the two writers are refused at call time"
    );
}

/// AC-2: two saved memories, a `find` that ranks the right one first, a
/// `query_id` the `retrieval_log` really holds (proven through the
/// `recall_status` tool, which reads that table), and an empty corpus that
/// still mints one.
#[tokio::test]
async fn mcp_02_find_returns_hits() {
    let empty_cwd = tempfile::TempDir::new().expect("cwd");
    let empty = McpHome::spawn(empty_cwd.path(), &[]).await;
    let cold = empty
        .data("find", json!({"query": "postgres analytics"}))
        .await;
    assert!(
        hit_ids(&cold).is_empty(),
        "an empty corpus returns no hits: {cold}"
    );
    assert!(
        !query_id(&cold).is_empty(),
        "an empty corpus still mints a query_id to judge"
    );
    drop(empty);

    let tmp = tempfile::TempDir::new().expect("cwd");
    let home = McpHome::spawn(tmp.path(), &[]).await;
    let postgres = save(&home, POSTGRES, REPO).await;
    let nextest = save(&home, NEXTEST, REPO).await;
    assert_ne!(postgres, nextest, "two distinct bodies, two distinct ids");

    let found = home
        .data("find", json!({"query": "postgres analytics", "repo": REPO}))
        .await;
    let ids = hit_ids(&found);
    assert_eq!(
        ids.first().map(String::as_str),
        Some(postgres.as_str()),
        "the Postgres memory must rank first for `postgres analytics`: {found}"
    );

    // The `retrieval_log` row is proven by reading it back: `recall_status`
    // lists exactly the tracked queries that still owe a verdict.
    let qid = query_id(&found);
    let status = home
        .data(
            "recall_status",
            json!({"repo": REPO, "since": "2000-01-01"}),
        )
        .await;
    let pending: Vec<&str> = status["pending"]
        .as_array()
        .expect("pending array")
        .iter()
        .filter_map(|p| p["query_id"].as_str())
        .collect();
    assert!(
        pending.contains(&qid.as_str()),
        "the tracked recall {qid} must be in retrieval_log: {status}"
    );

    let ack = home
        .data("feedback", json!({"query_id": qid, "used": [postgres]}))
        .await;
    assert_eq!(ack["used"], json!(1), "feedback ack: {ack}");
    assert_eq!(
        ack["known_query"],
        json!(true),
        "the query id must be the one retrieval_log recorded: {ack}"
    );
}

/// AC-3: a linked worktree files under the MAIN worktree's basename, so a
/// second session started in the main checkout sees the memory. Outside any
/// git work tree, an unscoped `save` is refused and `find` still works.
#[tokio::test]
async fn mcp_03_scope_is_main_worktree() {
    let tmp = tempfile::TempDir::new().expect("workspace");
    let main = tmp.path().join("sample");
    git_repo::init_repo(&main);
    git_commit::commit_files(&main, &[("README.md", "sample\n")], "init");
    let linked = tmp.path().join("sample-feature");
    git_worktree::add_worktree(&main, &linked, "feature");

    // Saved from the LINKED worktree, with no `repo` parameter at all.
    let from_worktree = McpHome::spawn(&linked, &[]).await;
    assert!(
        !from_worktree.instructions().contains("no default repo"),
        "a session inside a work tree has a scope and must not warn"
    );
    let id = from_worktree
        .data("save", json!({ "body": POSTGRES }))
        .await["id"]
        .as_str()
        .expect("save id")
        .to_string();

    // A second server on the same data dir, started in the MAIN worktree.
    let from_main = from_worktree.attach(&main, &[]).await;
    let found = from_main
        .data("find", json!({"query": "postgres analytics"}))
        .await;
    assert!(
        hit_ids(&found).contains(&id),
        "a memory saved from a linked worktree must be visible from the main one: {found}"
    );
    let page = from_main.data("list", json!({})).await;
    assert_eq!(
        page["items"][0]["repo"],
        json!("sample"),
        "the scope key is the MAIN worktree's basename: {page}"
    );

    // Outside any work tree: no scope, so an unscoped write is refused.
    let nowhere_cwd = tempfile::TempDir::new().expect("cwd");
    let nowhere = McpHome::spawn(nowhere_cwd.path(), &[]).await;
    let refusal = nowhere.error("save", json!({ "body": NEXTEST })).await;
    assert_eq!(refusal["code"], json!("repo_required"), "{refusal}");
    assert!(
        refusal["message"]
            .as_str()
            .is_some_and(|m| m.contains("--repo")),
        "the refusal must say how to supply a scope: {refusal}"
    );
    let unscoped = nowhere.data("find", json!({"query": "postgres"})).await;
    assert!(
        !query_id(&unscoped).is_empty(),
        "an unscoped read still runs: {unscoped}"
    );
}

/// AC-4: `--read-only` refuses both writers with a structured `read_only`
/// code, still answers `find`, and writes no `retrieval_log` row — proven
/// with the real `comemory recall-status --json` binary before and after.
#[tokio::test]
async fn mcp_04_read_only() {
    let tmp = tempfile::TempDir::new().expect("cwd");
    let seed = McpHome::spawn(tmp.path(), &[]).await;
    let postgres = save(&seed, POSTGRES, REPO).await;
    let data_dir = seed.data_dir().to_path_buf();
    // A SECOND server, `--read-only`, over the SAME seeded data dir.
    let home = seed.attach(tmp.path(), &["--read-only"]).await;

    for (tool, args) in [
        ("save", json!({"body": NEXTEST, "repo": REPO})),
        (
            "feedback",
            json!({"query_id": "q-20260918-aabbccdd", "used": [postgres.clone()]}),
        ),
    ] {
        let refusal = home.error(tool, args).await;
        assert_eq!(
            refusal["code"],
            json!("read_only"),
            "`{tool}` must be refused with the read_only code word: {refusal}"
        );
        assert!(
            refusal["message"]
                .as_str()
                .is_some_and(|m| m.contains(tool)),
            "the refusal must name the tool: {refusal}"
        );
    }

    // `--read-only` outranks the scope check: this server runs in a plain
    // tempdir (no git, no `--repo`), so an unscoped `save` would otherwise be
    // `repo_required` — read-only wins, as it does for every mutating route.
    let unscoped = home.error("save", json!({"body": NEXTEST})).await;
    assert_eq!(
        unscoped["code"],
        json!("read_only"),
        "read-only must outrank repo_required: {unscoped}"
    );

    // A TRACKED recall on the writable session first, so the counter is
    // proven to move at all — otherwise "unchanged" would be 0 == 0, a
    // comparison that holds however broken the gate is.
    assert_eq!(tracked_queries(&data_dir), 0, "a fresh store logs nothing");
    seed.data("find", json!({"query": "postgres analytics", "repo": REPO}))
        .await;
    let before = tracked_queries(&data_dir);
    assert_eq!(before, 1, "a writable session logs its tracked recall");

    let found = home
        .data("find", json!({"query": "postgres analytics", "repo": REPO}))
        .await;
    assert_eq!(
        hit_ids(&found).first().map(String::as_str),
        Some(postgres.as_str()),
        "a read-only server still recalls: {found}"
    );
    assert_eq!(
        tracked_queries(&data_dir),
        before,
        "a --read-only session writes nothing, telemetry included"
    );
}

/// `comemory recall-status --repo demo --json` `.queries`, read through the
/// real binary — an independent witness to the row count.
fn tracked_queries(data_dir: &Path) -> u64 {
    let out = Command::cargo_bin("comemory")
        .expect("binary")
        .env("COMEMORY_DATA_DIR", data_dir)
        .args([
            "recall-status",
            "--repo",
            REPO,
            "--since",
            "2000-01-01",
            "--json",
        ])
        .output()
        .expect("run recall-status");
    assert!(
        out.status.success(),
        "recall-status failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: Value = serde_json::from_slice(&out.stdout).expect("recall-status --json");
    report["queries"].as_u64().expect("queries count")
}

/// AC-5: an agent's own verdict lands as `implicit`, a user-stated one as
/// `manual`, read back from `feedback_events`. A malformed query id is a
/// tool-level error carrying the code word the core raises.
#[tokio::test]
async fn mcp_05_feedback_provenance() {
    let tmp = tempfile::TempDir::new().expect("cwd");
    let home = McpHome::spawn(tmp.path(), &[]).await;
    let postgres = save(&home, POSTGRES, REPO).await;

    let found = home
        .data("find", json!({"query": "postgres analytics", "repo": REPO}))
        .await;
    let qid = query_id(&found);

    let implicit = home
        .data(
            "feedback",
            json!({"query_id": qid, "used": [postgres.clone()]}),
        )
        .await;
    assert_eq!(
        implicit["provenance"],
        json!("implicit"),
        "an inferred verdict must never enter the golden harvest: {implicit}"
    );
    assert_eq!(
        stored_events(home.data_dir(), &qid),
        vec![(postgres.clone(), "used".to_string(), "implicit".to_string())],
        "the stored row, not the echoed field"
    );

    let manual = home
        .data(
            "feedback",
            json!({
                "query_id": qid,
                "used": [postgres.clone()],
                "confirmed_by_user": true
            }),
        )
        .await;
    assert_eq!(manual["provenance"], json!("manual"), "{manual}");
    let provenances: Vec<String> = stored_events(home.data_dir(), &qid)
        .into_iter()
        .map(|(_, _, p)| p)
        .collect();
    assert_eq!(
        provenances,
        vec!["implicit".to_string(), "manual".to_string()],
        "the two calls land as two rows with distinct provenance"
    );

    // The core refuses a query id that is not `q-<yyyymmdd>-<8hex>` with
    // `Error::Config`, which `mcp::result` maps to the `config` code word.
    let bad = home
        .error("feedback", json!({"query_id": "nope", "used": [postgres]}))
        .await;
    assert_eq!(bad["code"], json!("config"), "{bad}");
    assert!(
        bad["message"]
            .as_str()
            .is_some_and(|m| m.contains("invalid query id")),
        "{bad}"
    );
}

/// Diagnostics never reach stdout: with `RUST_LOG` enabled the binary still
/// emits a byte-exact JSON-RPC stream on stdout and puts every log line on
/// stderr. Driven with a raw pipe rather than the rmcp client so the
/// assertion is on the bytes a host would parse.
#[tokio::test]
async fn mcp_06_diagnostics_stay_off_stdout_under_rust_log() {
    use std::io::Write;
    use std::process::Stdio;

    let home = tempfile::tempdir().expect("tempdir");
    let data_dir = home.path().join(".comemory");
    let mut child = Command::cargo_bin("comemory")
        .expect("binary")
        .arg("mcp")
        .env("COMEMORY_DATA_DIR", &data_dir)
        .env("RUST_LOG", "info")
        .current_dir(home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn comemory mcp");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        let lines = [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion":"2025-06-18","capabilities":{},
                "clientInfo":{"name":"journey","version":"0"}}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        ];
        for line in lines {
            writeln!(stdin, "{line}").expect("write request");
        }
        // Dropping stdin sends EOF, which is the clean-exit path.
    }
    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success(), "exit status: {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).expect("utf-8 stdout");
    let replies: Vec<Value> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str(l).unwrap_or_else(|e| panic!("non-JSON stdout line {l:?}: {e}"))
        })
        .collect();
    assert_eq!(
        replies.len(),
        2,
        "one reply per request, nothing else: {stdout}"
    );
    let names: Vec<&str> = replies[1]["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().expect("name"))
        .collect();
    assert_eq!(
        names.len(),
        TOOLS.len(),
        "every catalog tool listed: {names:?}"
    );
    let stderr = String::from_utf8(out.stderr).expect("utf-8 stderr");
    assert!(
        stderr.contains("INFO") || stderr.contains("info"),
        "RUST_LOG=info must produce at least one log line on stderr; got {stderr:?}"
    );
}
