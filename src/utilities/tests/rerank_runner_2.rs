#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Protocol conformance of the shipped Python reference backend (#212).
//!
//! Every test here starts a real `python3` child running the real
//! `integrations/reranker/comemory_rerank.py`, feeds it the real versioned
//! JSON on real pipes, and reads the answer back through the real
//! [`RerankRunner`]. Nothing is mocked and nothing is stubbed.
//!
//! The backend runs in its deterministic `lexical-overlap` mode, whose scoring
//! function is pinned in the design document so every expected order below is
//! computed by hand rather than recorded from a run. That mode measures **no
//! relevance quality** — it exists so this suite can prove the protocol
//! without a model download. What the model can prove is the separate, opt-in
//! `bash integrations/reranker/run-model-tests.sh` suite, which refuses to run
//! at all when the pinned weights are absent.
//!
//! `python3` is a prerequisite of this suite. It is not skipped when the
//! interpreter is missing: a skipped protocol test that reads as a pass is the
//! failure mode the whole design is arranged to avoid.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use tempfile::TempDir;

use comemory::utilities::process_runner::ProcessRunner;
use comemory::utilities::rerank_outcome::{RerankFailure, RerankOutcome};
use comemory::utilities::rerank_protocol::{RerankCandidate, RerankRequest};
use comemory::utilities::rerank_runner::RerankRunner;

/// The identity a `lexical-overlap` process answers to. Deliberately not a
/// model name, so this mode cannot answer a request expecting a real model.
const LEXICAL_MODEL: &str = "lexical-overlap@1";

/// The label of the pinned cross-encoder, used to prove the identity gate.
const PINNED_MODEL: &str =
    "cross-encoder/ms-marco-MiniLM-L6-v2@233902d25c440f23af6f7d6e94d2946bac0bee0a";

const MEMORY: &str = "memory:aaaaaaaa";
const CODE: &str = "code:comemory:src/utilities/process_runner.rs:run";
const DOCUMENT: &str = "document:notes.md#3";

/// The query whose hand-computed Jaccard scores the happy path asserts.
const QUERY: &str = "bounded subprocess deadline";

/// `sysexits.h` `EX_DATAERR`: the request was refused.
const EX_DATAERR: i32 = 65;
/// `sysexits.h` `EX_UNAVAILABLE`: the warm server could not be reached.
const EX_UNAVAILABLE: i32 = 69;
/// `sysexits.h` `EX_USAGE`: the command line itself was wrong.
const EX_USAGE: i32 = 64;
/// `sysexits.h` `EX_CANTCREAT`: the warm socket could not be taken over.
const EX_CANTCREAT: i32 = 73;

/// The shipped backend, addressed from the manifest so the path is stable
/// wherever this file is compiled from.
fn backend() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/integrations/reranker/comemory_rerank.py"
    ))
}

/// Fail loudly, naming the prerequisite, rather than skipping.
fn require_python() {
    let found = Command::new("python3")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match found {
        Ok(status) if status.success() => {}
        _ => panic!(
            "python3 is required by the reranker conformance suite and was not runnable. \
             It drives the shipped backend at {}. This test refuses to skip, because a \
             skipped protocol test that reads as a pass is worse than no protocol test.",
            backend().display()
        ),
    }
}

/// A runner pointed at the backend in its deterministic mode.
fn lexical_runner() -> RerankRunner {
    require_python();
    RerankRunner::new(
        "python3",
        backend_args(&["score", "--scoring", "lexical-overlap"]),
    )
    .with_timeout(Duration::from_secs(30))
}

/// The backend script followed by `rest`, as one argument vector. The program
/// and its arguments are always passed separately, so no payload can reach a
/// shell.
fn backend_args(rest: &[&str]) -> Vec<OsString> {
    let mut args = vec![backend().into_os_string()];
    args.extend(rest.iter().map(OsString::from));
    args
}

/// The same, with a filesystem path as the final argument — a socket, say,
/// which is not representable as `&str` on every platform.
fn backend_args_at(rest: &[&str], path: &Path) -> Vec<OsString> {
    let mut args = backend_args(rest);
    args.push(path.to_path_buf().into_os_string());
    args
}

/// A bare process runner over the backend, for the cases that need a request
/// body `RerankRequest` cannot express.
fn backend_process(rest: &[&str]) -> ProcessRunner {
    require_python();
    ProcessRunner::new("python3", backend_args(rest)).with_timeout(Duration::from_mins(1))
}

/// The three candidates whose scores the design document computes by hand:
/// `memory` shares no token with the query (`0.0`), `code` shares all three of
/// four (`0.75`), `document` shares one of three (`1/3`).
fn hand_computed_candidates() -> Vec<RerankCandidate> {
    vec![
        RerankCandidate::new(MEMORY, 0, "markdown is the source of truth"),
        RerankCandidate::new(CODE, 1, "the bounded subprocess deadline"),
        RerankCandidate::new(DOCUMENT, 2, "deadline"),
    ]
}

fn request(
    model: &str,
    adapter: Option<String>,
    candidates: Vec<RerankCandidate>,
) -> RerankRequest {
    RerankRequest::with_request_id("rr-20260918-1a2b3c4d", model, adapter, QUERY, candidates)
}

/// The exit code a declined outcome reports, or a panic naming what it was.
fn declined_code(outcome: &RerankOutcome) -> i32 {
    match outcome.failure() {
        Some(RerankFailure::NonZeroExit { code: Some(code) }) => *code,
        other => panic!("expected a non-zero exit, found {other:?}"),
    }
}

#[test]
fn reorders_by_the_backends_own_scores() {
    let outcome =
        lexical_runner().rerank(&request(LEXICAL_MODEL, None, hand_computed_candidates()));
    assert!(
        outcome.is_applied(),
        "expected the shipped backend to be applied, got {outcome:?}"
    );
    assert_eq!(outcome.order_ids(), vec![CODE, DOCUMENT, MEMORY]);
    let RerankOutcome::Applied(applied) = &outcome else {
        unreachable!("checked above")
    };
    assert_eq!(applied.model, LEXICAL_MODEL);
    assert_eq!(applied.adapter, None);
    assert_eq!(applied.request_id, "rr-20260918-1a2b3c4d");
    // The pinned Jaccard definition, recomputed here rather than recorded.
    assert_eq!(applied.order[0].score, 3.0 / 4.0);
    assert_eq!(applied.order[1].score, 1.0 / 3.0);
    assert_eq!(applied.order[2].score, 0.0);
    // A successful run says nothing on stderr, so a caller logging the excerpt
    // logs nothing on the happy path.
    assert!(
        applied.stderr_excerpt.is_empty(),
        "{:?}",
        applied.stderr_excerpt
    );
}

#[test]
fn equal_scores_keep_the_submitted_order() {
    let candidates = vec![
        RerankCandidate::new("code:first", 0, "bounded subprocess deadline"),
        RerankCandidate::new("code:second", 1, "bounded subprocess deadline"),
        RerankCandidate::new("code:third", 2, "unrelated prose"),
    ];
    let outcome = lexical_runner().rerank(&request(LEXICAL_MODEL, None, candidates));
    assert!(outcome.is_applied(), "{outcome:?}");
    assert_eq!(
        outcome.order_ids(),
        vec!["code:first", "code:second", "code:third"],
        "a tie must fall back to the submitted rank, ascending"
    );
}

#[test]
fn refuses_a_model_label_it_does_not_answer_to() {
    let request = request(PINNED_MODEL, None, hand_computed_candidates());
    let outcome = lexical_runner().rerank(&request);
    assert_eq!(declined_code(&outcome), EX_DATAERR);
    assert_eq!(outcome.order_ids(), vec![MEMORY, CODE, DOCUMENT]);
    let RerankOutcome::Declined(declined) = &outcome else {
        unreachable!("checked above")
    };
    assert!(
        declined.stderr_excerpt.contains(LEXICAL_MODEL)
            && declined.stderr_excerpt.contains("ms-marco-MiniLM-L6-v2"),
        "the diagnostic must name both identities: {:?}",
        declined.stderr_excerpt
    );
}

#[test]
fn refuses_an_adapter_a_base_only_process_cannot_honour() {
    let request = request(
        LEXICAL_MODEL,
        Some("lora-v1".to_string()),
        hand_computed_candidates(),
    );
    let outcome = lexical_runner().rerank(&request);
    assert_eq!(declined_code(&outcome), EX_DATAERR);
    assert_eq!(outcome.order_ids(), vec![MEMORY, CODE, DOCUMENT]);
}

#[test]
fn refuses_every_malformed_request_body() {
    let runner = backend_process(&["score", "--scoring", "lexical-overlap"]);

    let cases: [(&str, &str); 8] = [
        ("", "empty body"),
        ("not json at all", "unparsable body"),
        (
            r#"{"protocol_version":2,"request_id":"rr-20260918-1a2b3c4d","model":"lexical-overlap@1","adapter":null,"query":"q","candidates":[{"id":"a","rank":0,"text":"t"}]}"#,
            "future protocol version",
        ),
        (
            r#"{"protocol_version":1,"request_id":"not-an-id","model":"lexical-overlap@1","adapter":null,"query":"q","candidates":[{"id":"a","rank":0,"text":"t"}]}"#,
            "malformed request id",
        ),
        (
            r#"{"protocol_version":1,"request_id":"rr-20260918-1a2b3c4d","model":"lexical-overlap@1","adapter":null,"query":"q","surprise":1,"candidates":[{"id":"a","rank":0,"text":"t"}]}"#,
            "unknown request key",
        ),
        (
            r#"{"protocol_version":1,"request_id":"rr-20260918-1a2b3c4d","model":"lexical-overlap@1","adapter":null,"query":"q","candidates":[]}"#,
            "empty candidate list",
        ),
        (
            r#"{"protocol_version":1,"request_id":"rr-20260918-1a2b3c4d","model":"lexical-overlap@1","adapter":null,"query":"q","candidates":[{"id":"a","rank":0,"text":"t"},{"id":"a","rank":1,"text":"u"}]}"#,
            "duplicate candidate id",
        ),
        (
            r#"{"protocol_version":1,"request_id":"rr-20260918-1a2b3c4d","model":"lexical-overlap@1","adapter":null,"query":"q","candidates":[{"id":"a","rank":0,"text":"t","weight":1}]}"#,
            "unknown candidate key",
        ),
    ];

    for (body, what) in cases {
        let output = runner
            .run(body.as_bytes())
            .unwrap_or_else(|e| panic!("{what}: the child did not complete: {e:?}"));
        assert_eq!(
            output.status.code(),
            Some(EX_DATAERR),
            "{what} must be refused with EX_DATAERR"
        );
        assert!(output.stdout.is_empty(), "{what} must write no response");
        assert!(
            !output.stderr.is_empty(),
            "{what} must leave a diagnostic on stderr"
        );
    }
}

#[test]
fn fingerprint_marks_the_deterministic_mode_as_not_a_model() {
    let runner = backend_process(&["fingerprint", "--scoring", "lexical-overlap"]);
    let output = runner.run(b"").expect("fingerprint must complete");
    assert_eq!(output.status.code(), Some(0));
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("the fingerprint must be JSON");
    assert_eq!(parsed["protocol_version"], 1);
    assert_eq!(parsed["scoring"], "lexical-overlap");
    assert_eq!(parsed["model_label"], LEXICAL_MODEL);
    assert_eq!(parsed["scoring_is_neural"], false);
    assert_eq!(parsed["score_direction"], "higher_is_better");
}

#[test]
fn benchmark_reports_load_cost_latency_and_memory() {
    let runner = backend_process(&[
        "benchmark",
        "--scoring",
        "lexical-overlap",
        "--candidates",
        "32",
        "--repeat",
        "5",
    ]);
    let output = runner.run(b"").expect("benchmark must complete");
    assert_eq!(output.status.code(), Some(0));
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("the report must be JSON");
    for key in ["load_ms", "p50_ms", "p95_ms", "peak_rss_bytes"] {
        assert!(
            parsed[key].is_number(),
            "the report must carry {key}: {parsed}"
        );
    }
    assert!(parsed["peak_rss_bytes"].as_u64().unwrap_or(0) > 0);
}

#[test]
fn shell_metacharacters_are_scored_as_literal_text() {
    let dir = TempDir::new().expect("temp dir");
    let sentinel = dir.path().join("pwned");
    let injected = format!(
        "$(touch {}) bounded subprocess deadline",
        sentinel.display()
    );
    let candidates = vec![
        RerankCandidate::new("code:injected", 0, injected.as_str()),
        RerankCandidate::new("memory:disjoint", 1, "entirely unrelated prose"),
    ];
    let query = format!(
        "\"; touch {}; # bounded subprocess deadline",
        sentinel.display()
    );
    let request = RerankRequest::with_request_id(
        "rr-20260918-1a2b3c4d",
        LEXICAL_MODEL,
        None,
        query,
        candidates,
    );
    let outcome = lexical_runner().rerank(&request);
    assert!(outcome.is_applied(), "{outcome:?}");
    assert!(
        !sentinel.exists(),
        "candidate text reached a shell: {} exists",
        sentinel.display()
    );
    assert_eq!(
        outcome.order_ids(),
        vec!["code:injected", "memory:disjoint"]
    );
    let RerankOutcome::Applied(applied) = &outcome else {
        unreachable!("checked above")
    };
    assert!(
        applied.order[0].score > applied.order[1].score,
        "the metacharacter candidate shares the query's tokens and must outrank a disjoint one"
    );
}

#[test]
fn a_warm_server_answers_two_requests_without_reloading() {
    require_python();
    let dir = TempDir::new().expect("temp dir");
    let socket = dir.path().join("rerank.sock");
    let ready = dir.path().join("ready");
    let log = dir.path().join("server.log");

    let mut server = spawn_server(&socket, &ready, &log);
    let (first, second) = wait_then_drive(&ready, &socket, &mut server);

    // Both requests go through the thin client, which is a real child process
    // speaking the real protocol on real pipes — exactly what #213 would run.
    assert!(first.is_applied(), "first request: {first:?}");
    assert!(second.is_applied(), "second request: {second:?}");
    assert_eq!(first.order_ids(), vec![CODE, DOCUMENT, MEMORY]);
    assert_eq!(second.order_ids(), vec![CODE, DOCUMENT, MEMORY]);

    // Stop the server before reading its log, so every line it will ever write
    // is already there. The guard would reap it anyway, including on a panic
    // above; this is about the log being complete, not about the process.
    drop(server);

    // The observation that makes "warm" a fact rather than a claim: the server
    // process announced readiness once and served twice, so the model — here a
    // deterministic scorer, but on the same code path — was prepared once.
    let recorded = fs::read_to_string(&log).expect("the server must have written its log");
    assert_eq!(
        recorded.matches(" ready ").count(),
        1,
        "expected exactly one readiness line: {recorded}"
    );
    assert_eq!(
        recorded.matches(" served ").count(),
        2,
        "expected exactly one served line per request: {recorded}"
    );
}

#[test]
fn a_client_with_no_server_declines_and_keeps_the_original_order() {
    require_python();
    let dir = TempDir::new().expect("temp dir");
    let absent = dir.path().join("absent.sock");
    let runner = RerankRunner::new("python3", backend_args_at(&["client", "--socket"], &absent))
        .with_timeout(Duration::from_secs(30));
    let outcome = runner.rerank(&request(LEXICAL_MODEL, None, hand_computed_candidates()));
    assert_eq!(declined_code(&outcome), EX_UNAVAILABLE);
    assert_eq!(outcome.order_ids(), vec![MEMORY, CODE, DOCUMENT]);
}

#[test]
fn verbose_puts_the_fingerprint_on_stderr_without_touching_the_response() {
    require_python();
    let runner = RerankRunner::new(
        "python3",
        backend_args(&["score", "--scoring", "lexical-overlap", "--verbose"]),
    )
    .with_timeout(Duration::from_secs(30));
    let outcome = runner.rerank(&request(LEXICAL_MODEL, None, hand_computed_candidates()));
    assert!(outcome.is_applied(), "{outcome:?}");
    assert_eq!(outcome.order_ids(), vec![CODE, DOCUMENT, MEMORY]);
    let RerankOutcome::Applied(applied) = &outcome else {
        unreachable!("checked above")
    };
    // The fingerprint travels on stderr, never in the response: the response
    // object denies unknown fields, so an extra key there would be a protocol
    // version bump rather than a diagnostic.
    assert!(
        applied
            .stderr_excerpt
            .contains("\"scoring\":\"lexical-overlap\"")
            && applied
                .stderr_excerpt
                .contains("\"scoring_is_neural\":false"),
        "expected the fingerprint on stderr, found {:?}",
        applied.stderr_excerpt
    );
}

#[test]
fn an_adapter_is_a_usage_error_in_the_deterministic_mode() {
    let runner = backend_process(&[
        "score",
        "--scoring",
        "lexical-overlap",
        "--adapter",
        "/nonexistent/lora-v1",
    ]);
    let output = runner.run(b"").expect("the child must complete");
    assert_eq!(
        output.status.code(),
        Some(EX_USAGE),
        "a mode with no model to adapt must refuse --adapter as a usage error"
    );
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no model to adapt"),
        "{:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_second_server_never_displaces_the_one_already_listening() {
    require_python();
    let dir = TempDir::new().expect("temp dir");
    let socket = dir.path().join("rerank.sock");
    let ready = dir.path().join("ready");
    let log = dir.path().join("server.log");

    let mut server = spawn_server(&socket, &ready, &log);
    await_ready(&ready, &mut server);

    let intruder = ProcessRunner::new(
        "python3",
        backend_args_at(
            &["serve", "--scoring", "lexical-overlap", "--socket"],
            &socket,
        ),
    )
    .with_timeout(Duration::from_secs(30));
    let output = intruder
        .run(b"")
        .expect("the second server must exit, not hang");
    assert_eq!(
        output.status.code(),
        Some(EX_CANTCREAT),
        "a live socket must never be reclaimed from under its owner"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("already listening"),
        "{:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The incumbent is untouched and still answers.
    let client = RerankRunner::new("python3", backend_args_at(&["client", "--socket"], &socket))
        .with_timeout(Duration::from_secs(30));
    let outcome = client.rerank(&request(LEXICAL_MODEL, None, hand_computed_candidates()));
    assert!(
        outcome.is_applied(),
        "the incumbent must still serve: {outcome:?}"
    );
}

#[test]
fn the_opt_in_model_suite_never_reports_success_without_its_prerequisites() {
    require_python();
    let preflight = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/integrations/reranker/comemory_rerank_preflight.py"
    ));
    let runner = ProcessRunner::new("python3", vec![preflight.into_os_string()])
        .with_timeout(Duration::from_mins(1));
    let output = runner.run(b"").expect("the preflight must complete");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    match output.status.code() {
        // Every pinned library and the pinned snapshot are present on this
        // machine, so the opt-in suite would genuinely run.
        Some(0) => assert!(
            stderr.is_empty(),
            "a satisfied preflight says nothing: {stderr:?}"
        ),
        // Something is missing. The contract is that it never exits 0 and that
        // it names the command that fixes each gap, so a skipped model test
        // cannot be mistaken for a passing one.
        Some(EX_UNAVAILABLE) => assert!(
            stderr.contains("pip install -r") && stderr.contains("huggingface-cli download"),
            "an unsatisfied preflight must name both fixes: {stderr:?}"
        ),
        other => panic!("unexpected preflight exit {other:?}: {stderr}"),
    }
}

/// Owns a warm server for the length of a test and always reaps it.
///
/// `Child`'s own `Drop` neither kills nor waits, so an assertion that panics
/// between the spawn and an explicit `kill` would leave the server running for
/// the rest of the suite. A guard makes the cleanup unconditional, which is
/// what "always reaped" has to mean if it is to survive a failing test.
struct ServerGuard(Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Start a real warm server with its stderr captured to `log`.
fn spawn_server(socket: &Path, ready: &Path, log: &Path) -> ServerGuard {
    let sink = fs::File::create(log).expect("server log");
    let child = Command::new("python3")
        .arg(backend())
        .arg("serve")
        .arg("--scoring")
        .arg("lexical-overlap")
        .arg("--socket")
        .arg(socket)
        .arg("--ready-file")
        .arg(ready)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(sink))
        .spawn()
        .expect("the warm server must start");
    ServerGuard(child)
}

/// Block until the server publishes its ready file, or fail the test.
///
/// A server that died during start-up is reported as the exit status it died
/// with, immediately. Waiting out the full deadline first would cost thirty
/// seconds and then report a timeout, which says nothing about why.
fn await_ready(ready: &Path, server: &mut ServerGuard) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready.exists() {
        if let Some(status) = server.0.try_wait().expect("the server must be pollable") {
            panic!("the warm server exited with {status} before becoming ready");
        }
        assert!(
            Instant::now() <= deadline,
            "the warm server never became ready within 30s"
        );
        sleep(Duration::from_millis(20));
    }
}

/// Wait for readiness, then drive two requests through the thin client.
fn wait_then_drive(
    ready: &Path,
    socket: &Path,
    server: &mut ServerGuard,
) -> (RerankOutcome, RerankOutcome) {
    await_ready(ready, server);
    let runner = RerankRunner::new("python3", backend_args_at(&["client", "--socket"], socket))
        .with_timeout(Duration::from_secs(30));
    let first = runner.rerank(&request(LEXICAL_MODEL, None, hand_computed_candidates()));
    let second = runner.rerank(&request(LEXICAL_MODEL, None, hand_computed_candidates()));
    (first, second)
}
