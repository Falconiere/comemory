#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Real-subprocess tests for the composed reranker runner.
//!
//! Every scorer here is a real `/bin/sh` child over real pipes. It drains the
//! whole request from stdin, writes it to a side file so the test can assert
//! what actually crossed the wire, extracts the `request_id` out of the JSON,
//! and answers with a response built from that id. Nothing is mocked.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use comemory::utilities::rerank_outcome::{RerankFailure, RerankOutcome};
use comemory::utilities::rerank_protocol::{RerankCandidate, RerankRequest};
use comemory::utilities::rerank_runner::{RerankLimits, RerankRunner};

/// A predicate naming the failure a case must produce.
type Expect = fn(&RerankFailure) -> bool;

const MODEL: &str = "bge-reranker-base";
const MEMORY: &str = "memory:aaaaaaaa";
const CODE: &str = "code:comemory:src/lib.rs:main";
const DOCUMENT: &str = "document:notes.md#3";

/// The default scored order: code first, document second, memory last.
const SCORES: &str = concat!(
    r#"{"id":"memory:aaaaaaaa","score":0.10},"#,
    r#"{"id":"code:comemory:src/lib.rs:main","score":0.90},"#,
    r#"{"id":"document:notes.md#3","score":0.50}"#
);

/// Drain the request, record it for the test, and pull out its `request_id`.
const PREAMBLE: &str = concat!(
    "req=$(cat)\n",
    "printf '%s' \"$req\" > \"$1\"\n",
    "id=$(printf '%s' \"$req\" | sed -n 's/.*\"request_id\":\"\\([^\"]*\\)\".*/\\1/p')\n"
);

/// A response for the child to print, with `%s` standing in for the echoed id.
struct Reply {
    version: &'static str,
    request_id: &'static str,
    model: &'static str,
    adapter: &'static str,
    direction: &'static str,
    scores: &'static str,
}

impl Default for Reply {
    fn default() -> Self {
        Self {
            version: "1",
            request_id: "%s",
            model: MODEL,
            adapter: "null",
            direction: "higher_is_better",
            scores: SCORES,
        }
    }
}

impl Reply {
    /// The `printf` format string this reply prints as.
    fn format(&self) -> String {
        let mut out = String::from(r#"{"protocol_version":"#);
        out.push_str(self.version);
        out.push_str(r#","request_id":""#);
        out.push_str(self.request_id);
        out.push_str(r#"","model":""#);
        out.push_str(self.model);
        out.push_str(r#"","adapter":"#);
        out.push_str(self.adapter);
        out.push_str(r#","score_direction":""#);
        out.push_str(self.direction);
        out.push_str(r#"","scores":["#);
        out.push_str(self.scores);
        out.push_str("]}");
        out
    }
}

fn request() -> RerankRequest {
    RerankRequest::new(
        MODEL,
        None,
        "bounded subprocess",
        vec![
            RerankCandidate::new(MEMORY, 0, "first candidate text"),
            RerankCandidate::new(CODE, 1, "second candidate text"),
            RerankCandidate::new(DOCUMENT, 2, "third candidate text"),
        ],
        time::OffsetDateTime::now_utc(),
    )
}

/// A fresh directory nobody else in this test binary is using.
fn workdir(tag: &str) -> PathBuf {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "comemory-rerank-{}-{tag}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("workdir");
    dir
}

/// Write `body` as a shell script and return its path.
fn script(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("scorer.sh");
    std::fs::write(&path, body).expect("script");
    path
}

/// A runner over `/bin/sh <script> <seen-file>`.
///
/// `sh` is the program and the script is an argument, so the script needs no
/// execute bit and nothing is ever assembled into a command line.
fn runner(dir: &Path, body: &str) -> RerankRunner {
    let path = script(dir, body);
    RerankRunner::new(
        "/bin/sh",
        vec![OsString::from(path), OsString::from(dir.join("seen.json"))],
    )
    .with_timeout(Duration::from_secs(20))
}

/// A responder that prints `reply` after recording the request.
fn responder(dir: &Path, reply: &Reply) -> RerankRunner {
    let body = format!("{PREAMBLE}printf '{}' \"$id\"\n", reply.format());
    runner(dir, &body)
}

/// Run one request against a responder built from `reply`, and return both the
/// outcome and the exact bytes the child read from stdin.
fn run(tag: &str, reply: &Reply) -> (RerankOutcome, String) {
    let dir = workdir(tag);
    let outcome = responder(&dir, reply).rerank(&request());
    let seen = std::fs::read_to_string(dir.join("seen.json")).unwrap_or_default();
    (outcome, seen)
}

/// Assert an outcome was declined, and hand back its failure.
fn declined(outcome: &RerankOutcome) -> &RerankFailure {
    assert!(!outcome.is_applied(), "expected a refusal");
    assert_eq!(
        outcome.order_ids(),
        vec![MEMORY, CODE, DOCUMENT],
        "a refusal must restore the complete submitted order"
    );
    outcome
        .failure()
        .expect("a declined outcome carries a failure")
}

#[test]
fn a_real_scorer_reorders_the_candidates() {
    let (outcome, seen) = run("valid", &Reply::default());
    let RerankOutcome::Applied(applied) = &outcome else {
        panic!("expected Applied, got {outcome:?}");
    };
    assert_eq!(outcome.order_ids(), vec![CODE, DOCUMENT, MEMORY]);
    assert_eq!(applied.model, MODEL);
    assert_eq!(applied.adapter, None);
    assert_eq!(applied.order[0].score, 0.90);
    assert_eq!(
        applied.order[0].rank, 1,
        "the submitted rank travels with it"
    );

    // The child really received the whole versioned request on stdin.
    assert!(seen.contains(r#""protocol_version":1"#), "{seen}");
    assert!(seen.contains(&applied.request_id), "{seen}");
    assert!(seen.contains(r#""query":"bounded subprocess""#), "{seen}");
    assert!(seen.contains(r#""rank":2"#), "{seen}");
}

#[test]
fn every_identity_mismatch_is_refused() {
    let cases: [(&str, Reply, Expect); 4] = [
        (
            "version",
            Reply {
                version: "2",
                ..Reply::default()
            },
            |f| matches!(f, RerankFailure::VersionMismatch { .. }),
        ),
        (
            "request-id",
            Reply {
                request_id: "rr-20260918-deadbeef",
                ..Reply::default()
            },
            |f| matches!(f, RerankFailure::RequestIdMismatch { .. }),
        ),
        (
            "model",
            Reply {
                model: "some-other-model",
                ..Reply::default()
            },
            |f| matches!(f, RerankFailure::ModelMismatch { .. }),
        ),
        (
            "adapter",
            Reply {
                adapter: r#""lora-v1""#,
                ..Reply::default()
            },
            |f| matches!(f, RerankFailure::AdapterMismatch { .. }),
        ),
    ];
    for (tag, reply, expected) in cases {
        let (outcome, _) = run(tag, &reply);
        let failure = declined(&outcome);
        assert!(expected(failure), "{tag}: unexpected {failure:?}");
    }
}

#[test]
fn every_score_set_defect_is_refused() {
    let cases: [(&str, &'static str, Expect); 3] = [
        ("missing", r#"{"id":"memory:aaaaaaaa","score":0.1}"#, |f| {
            matches!(f, RerankFailure::MissingScores { .. })
        }),
        (
            "duplicate",
            concat!(
                r#"{"id":"memory:aaaaaaaa","score":0.1},"#,
                r#"{"id":"code:comemory:src/lib.rs:main","score":0.9},"#,
                r#"{"id":"document:notes.md#3","score":0.5},"#,
                r#"{"id":"memory:aaaaaaaa","score":0.2}"#
            ),
            |f| matches!(f, RerankFailure::DuplicateScore { .. }),
        ),
        (
            "unknown",
            concat!(
                r#"{"id":"memory:aaaaaaaa","score":0.1},"#,
                r#"{"id":"code:comemory:src/lib.rs:main","score":0.9},"#,
                r#"{"id":"document:notes.md#3","score":0.5},"#,
                r#"{"id":"memory:ffffffff","score":0.2}"#
            ),
            |f| matches!(f, RerankFailure::UnknownScore { .. }),
        ),
    ];
    for (tag, scores, expected) in cases {
        let (outcome, _) = run(
            tag,
            &Reply {
                scores,
                ..Reply::default()
            },
        );
        let failure = declined(&outcome);
        assert!(expected(failure), "{tag}: unexpected {failure:?}");
    }
}

#[test]
fn malformed_and_empty_stdout_are_refused() {
    for (tag, body) in [
        ("garbage", "cat > /dev/null; printf 'not json'"),
        ("empty", "cat > /dev/null"),
        (
            "truncated",
            r#"cat > /dev/null; printf '{"protocol_version":1'"#,
        ),
    ] {
        let dir = workdir(tag);
        let outcome = runner(&dir, body).rerank(&request());
        let failure = declined(&outcome);
        assert!(
            matches!(failure, RerankFailure::Malformed { .. }),
            "{tag}: unexpected {failure:?}"
        );
    }
}

#[test]
fn a_valid_response_with_a_nonzero_exit_is_not_applied() {
    // Full validation AND a zero exit: a scorer that answers correctly and
    // then reports failure has still reported failure.
    let dir = workdir("exit3");
    let body = format!(
        "{PREAMBLE}printf '{}' \"$id\"\nexit 3\n",
        Reply::default().format()
    );
    let outcome = runner(&dir, &body).rerank(&request());
    let failure = declined(&outcome);
    assert!(
        matches!(failure, RerankFailure::NonZeroExit { code: Some(3) }),
        "{failure:?}"
    );
}

#[test]
fn a_hanging_scorer_times_out_within_its_budget() {
    let dir = workdir("hang");
    let started = Instant::now();
    let outcome = runner(&dir, "sleep 30")
        .with_timeout(Duration::from_millis(300))
        .rerank(&request());
    let failure = declined(&outcome);
    assert!(
        matches!(failure, RerankFailure::TimedOut { .. }),
        "{failure:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_missing_scorer_is_refused_without_a_panic() {
    let outcome = RerankRunner::new("/nonexistent/reranker", vec![]).rerank(&request());
    let failure = declined(&outcome);
    assert!(
        matches!(failure, RerankFailure::Spawn { .. }),
        "{failure:?}"
    );
}

#[test]
fn an_oversized_response_is_refused() {
    let dir = workdir("huge");
    let outcome = runner(&dir, "cat > /dev/null; yes x | head -c 200000")
        .with_limits(RerankLimits {
            max_stdout_bytes: 1024,
            ..RerankLimits::default()
        })
        .rerank(&request());
    let failure = declined(&outcome);
    assert!(
        matches!(failure, RerankFailure::OutputTooLarge { max: 1024 }),
        "{failure:?}"
    );
}

#[test]
fn a_chatty_scorer_still_applies_with_a_capped_excerpt() {
    let dir = workdir("chatty");
    let body = format!(
        "{PREAMBLE}yes e | head -c 200000 >&2\nprintf '{}' \"$id\"\n",
        Reply::default().format()
    );
    let outcome = runner(&dir, &body)
        .with_limits(RerankLimits {
            max_stderr_bytes: 64,
            ..RerankLimits::default()
        })
        .rerank(&request());
    let RerankOutcome::Applied(applied) = &outcome else {
        panic!("a chatty scorer is not a failing scorer: {outcome:?}");
    };
    assert_eq!(applied.stderr_excerpt.len(), 64);
    assert_eq!(outcome.order_ids(), vec![CODE, DOCUMENT, MEMORY]);
}

#[test]
fn bounds_are_enforced_before_anything_is_spawned() {
    let dir = workdir("no-spawn");
    let sentinel = dir.join("spawned");
    let spawn_detector = RerankRunner::new(
        "/bin/sh",
        vec![
            OsString::from(script(&dir, r#"touch "$1""#)),
            OsString::from(&sentinel),
        ],
    );

    let too_many = spawn_detector
        .clone()
        .with_limits(RerankLimits {
            max_candidates: 2,
            ..RerankLimits::default()
        })
        .rerank(&request());
    assert!(matches!(
        declined(&too_many),
        RerankFailure::TooManyCandidates { count: 3, max: 2 }
    ));

    let too_big = spawn_detector
        .clone()
        .with_limits(RerankLimits {
            max_request_bytes: 32,
            ..RerankLimits::default()
        })
        .rerank(&request());
    assert!(matches!(
        declined(&too_big),
        RerankFailure::RequestTooLarge { max: 32, .. }
    ));

    let long_text = spawn_detector
        .clone()
        .with_limits(RerankLimits {
            max_candidate_text_bytes: 4,
            ..RerankLimits::default()
        })
        .rerank(&request());
    assert!(matches!(
        declined(&long_text),
        RerankFailure::CandidateTextTooLarge { .. }
    ));

    assert!(!sentinel.exists(), "no bound check may spawn the scorer");
}

#[test]
fn an_empty_or_ambiguous_candidate_list_is_refused() {
    let dir = workdir("bad-request");
    let scorer = responder(&dir, &Reply::default());

    let empty = RerankRequest::with_request_id("rr-20260918-1a2b3c4d", MODEL, None, "q", vec![]);
    assert!(matches!(
        scorer.rerank(&empty).failure(),
        Some(RerankFailure::EmptyCandidates)
    ));

    let duplicated = RerankRequest::with_request_id(
        "rr-20260918-1a2b3c4d",
        MODEL,
        None,
        "q",
        vec![
            RerankCandidate::new(MEMORY, 0, "one"),
            RerankCandidate::new(MEMORY, 1, "two"),
        ],
    );
    assert!(matches!(
        scorer.rerank(&duplicated).failure(),
        Some(RerankFailure::DuplicateCandidateId { .. })
    ));
}

#[test]
fn shell_metacharacters_travel_as_data_and_never_execute() {
    let dir = workdir("injection");
    let pwned = dir.join("pwned");
    let injection = format!(r#""; touch {}; #"#, pwned.display());
    let substitution = format!("$(touch {})", pwned.display());
    let scorer = responder(&dir, &Reply::default());

    let request = RerankRequest::new(
        MODEL,
        None,
        injection.clone(),
        vec![
            RerankCandidate::new(MEMORY, 0, substitution.clone()),
            RerankCandidate::new(CODE, 1, "plain"),
            RerankCandidate::new(DOCUMENT, 2, "plain"),
        ],
        time::OffsetDateTime::now_utc(),
    );
    let outcome = scorer.rerank(&request);
    assert!(outcome.is_applied(), "{outcome:?}");
    assert!(!pwned.exists(), "payload text must never reach a shell");

    // ...and it did arrive at the child, verbatim, on stdin.
    let seen = std::fs::read_to_string(dir.join("seen.json")).expect("seen");
    let decoded: serde_json::Value = serde_json::from_str(&seen).expect("valid JSON on stdin");
    assert_eq!(decoded["query"], serde_json::json!(injection));
    assert_eq!(
        decoded["candidates"][0]["text"],
        serde_json::json!(substitution)
    );
}

#[test]
fn one_runner_serves_concurrent_requests() {
    let dir_a = workdir("concurrent-a");
    let dir_b = workdir("concurrent-b");
    let body = format!("{PREAMBLE}printf '{}' \"$id\"\n", Reply::default().format());
    let path = script(&dir_a, &body);
    let path_b = script(&dir_b, &body);
    let runner_a = RerankRunner::new(
        "/bin/sh",
        vec![
            OsString::from(&path),
            OsString::from(dir_a.join("seen.json")),
        ],
    );
    let runner_b = RerankRunner::new(
        "/bin/sh",
        vec![
            OsString::from(&path_b),
            OsString::from(dir_b.join("seen.json")),
        ],
    );

    let first = request();
    let second = request();
    assert_ne!(first.request_id, second.request_id, "distinct ids");

    std::thread::scope(|scope| {
        let a = scope.spawn(|| runner_a.rerank(&first));
        let b = scope.spawn(|| runner_b.rerank(&second));
        for (outcome, expected) in [(a.join().unwrap(), &first), (b.join().unwrap(), &second)] {
            let RerankOutcome::Applied(applied) = &outcome else {
                panic!("expected Applied, got {outcome:?}");
            };
            assert_eq!(applied.request_id, expected.request_id);
            assert_eq!(outcome.order_ids(), vec![CODE, DOCUMENT, MEMORY]);
        }
    });
}
