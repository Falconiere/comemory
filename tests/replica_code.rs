#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Code generations over the real surface: the real CLI binary, real spawned
//! `comemory serve` engines, real HTTP, and real git checkouts built from
//! pinned snapshots of this repository's own Rust sources.
//!
//! This half covers issue 252's AC-1 (one generation per indexed revision,
//! whichever surface indexed it), AC-2 (a repository-sized generation crosses
//! in bounded parts and becomes visible whole), AC-3 (two engines planning
//! from one parent), AC-4 (an import never touches a local row) and AC-5
//! (upload selection offers only what this machine built).
//!
//! The reader half — a checkout-less machine, a tracked deletion and an
//! unmounted checkout — is `replica_code_2.rs`.

#[path = "common/replica_support.rs"]
mod replica_support;

use replica_support::{
    CODE_REPO, Engine, active_generation, cli_raw, code_envelope, git, pinned_repo,
    planned_generation, publish_locally, shared_paths,
};

/// Over `MAX_BATCH_FILES`, so the manifest crosses the bound a push cuts on.
const WIDE: usize = 537;

/// A small pinned tree, for the cases that are not about size.
const SMALL: usize = 12;

/// Index `root` under [`CODE_REPO`] through the real CLI. `index-code`
/// prints nothing on success, so the exit status is the result.
fn index_cli(engine: &Engine, root: &std::path::Path) {
    let (code, _out, err) = cli_raw(
        &engine.data_dir(),
        &[
            "index-code",
            "--repo",
            CODE_REPO,
            "--path",
            root.to_str().expect("utf8 path"),
        ],
    );
    assert_eq!(code, 0, "index-code failed: {err}");
}

/// Wait for a queued job to finish, and assert it succeeded.
///
/// `POST /api/v1/code/index` answers `202` with a job id: the walk runs off
/// the request path, so a case that reads what it indexed has to wait for it.
fn await_job(engine: &Engine, job_id: &str) {
    for _ in 0..200 {
        let (status, body) = engine.get(&format!("/api/v1/jobs/{job_id}"));
        assert_eq!(status, 200, "job status: {body}");
        match body["data"]["status"].as_str() {
            Some("done") => return,
            Some("error" | "cancelled") => panic!("index job did not succeed: {body}"),
            _ => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
    panic!("index job {job_id} never finished");
}

#[test]
fn indexing_through_the_cli_and_through_http_produce_the_same_generation() {
    let workspace = tempfile::tempdir().expect("workspace");
    let root = pinned_repo(workspace.path(), SMALL);

    let cli_engine = Engine::spawn(&[]);
    index_cli(&cli_engine, &root);
    let from_cli = planned_generation(&cli_engine.data_dir(), CODE_REPO).expect("cli generation");

    // The HTTP index route contains every path it is given, so the fixture
    // tree has to be an allowed root for this session.
    let http_engine = Engine::spawn(&[
        "--allow-path",
        workspace.path().to_str().expect("utf8 path"),
    ]);
    let (status, body) = http_engine.post(
        "/api/v1/code/index",
        &serde_json::json!({
            "repo": CODE_REPO,
            "path": root.to_str().expect("utf8 path"),
            "mode": "incremental",
        }),
    );
    assert_eq!(status, 202, "index over HTTP: {body}");
    await_job(
        &http_engine,
        body["data"]["job_id"].as_str().expect("job_id"),
    );
    let from_http =
        planned_generation(&http_engine.data_dir(), CODE_REPO).expect("http generation");

    assert_eq!(
        from_cli["generation_id"], from_http["generation_id"],
        "one revision, one identity — the surface that indexed it is not part of it"
    );
    assert_eq!(from_cli["head"], from_http["head"]);
    assert_eq!(
        from_cli["files"].as_array().expect("files").len(),
        SMALL,
        "every pinned file is in the manifest"
    );
}

#[test]
fn no_generation_carries_source_text() {
    let workspace = tempfile::tempdir().expect("workspace");
    let root = pinned_repo(workspace.path(), SMALL);
    let engine = Engine::spawn(&[]);
    index_cli(&engine, &root);

    let payload = planned_generation(&engine.data_dir(), CODE_REPO).expect("generation");

    let wire = serde_json::to_string(&payload).expect("serialize");
    assert!(!wire.contains("snippet"), "no snippet field");
    // A line every one of this repository's sources begins with. If any body
    // text leaked into the manifest, this is what it would look like.
    assert!(
        !wire.contains("#![allow("),
        "no source text of any kind is on the wire"
    );
    let symbols = payload["symbols"].as_array().expect("symbols");
    assert!(!symbols.is_empty(), "but the symbols themselves are there");
    for symbol in symbols {
        assert!(symbol.get("snippet").is_none(), "{symbol}");
    }
}

#[test]
fn a_repository_sized_generation_arrives_whole_over_http() {
    let workspace = tempfile::tempdir().expect("workspace");
    let root = pinned_repo(workspace.path(), WIDE);
    let author = Engine::spawn(&[]);
    index_cli(&author, &root);
    let payload = planned_generation(&author.data_dir(), CODE_REPO).expect("generation");
    assert_eq!(
        payload["files"].as_array().expect("files").len(),
        WIDE,
        "the manifest is over the batch bound"
    );

    let peer = Engine::spawn(&[]);
    let (status, body) = peer.post(
        "/api/v1/sync/replica/import",
        &code_envelope("op-20260922-wide0001", CODE_REPO, &payload),
    );

    assert_eq!(status, 200, "import: {body}");
    assert_eq!(body["data"]["results"][0]["disposition"], "accepted");
    assert_eq!(
        shared_paths(&peer.data_dir(), CODE_REPO).len(),
        WIDE,
        "every file of the generation became visible at once"
    );
    assert_eq!(
        active_generation(&peer.data_dir(), CODE_REPO).as_deref(),
        payload["generation_id"].as_str(),
    );
}

#[test]
fn a_severed_upload_leaves_the_peer_at_the_generation_it_had() {
    let workspace = tempfile::tempdir().expect("workspace");
    let root = pinned_repo(workspace.path(), SMALL);
    let author = Engine::spawn(&[]);
    index_cli(&author, &root);
    let payload = planned_generation(&author.data_dir(), CODE_REPO).expect("generation");
    let peer = Engine::spawn(&[]);
    let (status, _) = peer.post(
        "/api/v1/sync/replica/import",
        &code_envelope("op-20260922-sever001", CODE_REPO, &payload),
    );
    assert_eq!(status, 200);
    let settled = active_generation(&peer.data_dir(), CODE_REPO).expect("first generation");

    // A real second generation: the author's tree moves, and the upload of
    // what that produced is cut off after its first part.
    std::fs::write(root.join("file_9999.rs"), "pub fn added_later() {}\n").expect("write");
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "add a file"]);
    index_cli(&author, &root);
    let next = planned_generation(&author.data_dir(), CODE_REPO).expect("second generation");
    assert_ne!(
        next["generation_id"], payload["generation_id"],
        "the tree moved, so this is a different generation"
    );
    let bytes = serde_json::to_string(&next).expect("bytes");
    let mut half = bytes.len() / 2;
    while !bytes.is_char_boundary(half) {
        half += 1;
    }
    let (status, body) = peer.post(
        "/api/v1/sync/replica/stage",
        &serde_json::json!({
            "protocol": "replica-v1",
            "staging_id": "upload-severed",
            "part_index": 0,
            "part_count": 2,
            "bytes": &bytes[..half],
        }),
    );
    assert_eq!(status, 200, "stage: {body}");
    assert_eq!(body["data"]["complete"], false);

    assert_eq!(
        active_generation(&peer.data_dir(), CODE_REPO),
        Some(settled),
        "the peer is still at the generation that completed"
    );
    assert!(
        !shared_paths(&peer.data_dir(), CODE_REPO).contains(&"file_9999.rs".to_string()),
        "and holds nothing the severed upload was carrying"
    );
    let (status, changes) = peer.get("/api/v1/sync/replica/changes?since=0&limit=100");
    assert_eq!(status, 200, "{changes}");
    let entries = changes["data"]["entries"].as_array().expect("entries");
    assert_eq!(
        entries.len(),
        1,
        "the half-uploaded generation is not history: {changes}"
    );
}

#[test]
fn two_engines_planning_from_one_parent_do_not_union_two_heads() {
    let workspace = tempfile::tempdir().expect("workspace");
    let root = pinned_repo(workspace.path(), SMALL);
    let author = Engine::spawn(&[]);
    index_cli(&author, &root);
    let first = planned_generation(&author.data_dir(), CODE_REPO).expect("first");
    publish_locally(&author.data_dir(), CODE_REPO);

    let peer = Engine::spawn(&[]);
    let (status, _) = peer.post(
        "/api/v1/sync/replica/import",
        &code_envelope("op-20260922-race0001", CODE_REPO, &first),
    );
    assert_eq!(status, 200);

    // Two machines each extend the SAME parent with a different head. The
    // first accepted wins; the second must be refused rather than merged.
    let rival = |head: &str, oid: &str| {
        let mut payload = first.clone();
        payload["parent_id"] = first["generation_id"].clone();
        payload["head"] = serde_json::json!(head);
        payload["files"][0]["blob_oid"] = serde_json::json!(oid);
        let text = serde_json::to_string(&payload).expect("bytes");
        let mut decoded = comemory::domains::code::replica_payload::CodeGenerationV1::decode(&text)
            .expect("decode");
        decoded.generation_id = String::new();
        let minted = decoded.mint_id().expect("mint");
        let (bytes, _) = decoded.with_id(&minted).canonical().expect("canonical");
        serde_json::from_str::<serde_json::Value>(&bytes).expect("json")
    };
    let (status, body) = peer.post(
        "/api/v1/sync/replica/import",
        &code_envelope("op-20260922-race0002", CODE_REPO, &rival("head-a", "a1")),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"]["results"][0]["disposition"], "accepted");
    let winner = active_generation(&peer.data_dir(), CODE_REPO).expect("winner");

    let (status, body) = peer.post(
        "/api/v1/sync/replica/import",
        &code_envelope("op-20260922-race0003", CODE_REPO, &rival("head-b", "b2")),
    );

    assert_eq!(status, 200, "the envelope itself is fine: {body}");
    assert_eq!(
        body["data"]["results"][0]["disposition"], "rejected_stale",
        "the loser is answered, not merged and not thrown: {body}"
    );
    assert_eq!(
        active_generation(&peer.data_dir(), CODE_REPO),
        Some(winner),
        "the repo is at exactly one of the two heads"
    );
}
