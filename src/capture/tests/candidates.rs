#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Candidate batch build + POST client against a loopback stub.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;

use comemory::capture::candidates::{batch_from_extracted, post_candidates};
use comemory::capture::claude_code::{bash_commands_from_jsonl, read_transcript_file};
use comemory::capture::explicit_save::extract_explicit_saves;

fn fixture_extracted() -> Vec<comemory::capture::ExtractedCandidate> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/claude-code-session-saves.jsonl");
    let text = read_transcript_file(&path).unwrap();
    extract_explicit_saves(&bash_commands_from_jsonl(&text))
}

#[test]
fn batch_from_real_fixture_has_six_claims_and_attestation() {
    let batch = batch_from_extracted(&fixture_extracted()).unwrap();
    assert_eq!(batch.extractor, "claude-code-explicit-save");
    assert_eq!(batch.extractor_version, 1);
    assert_eq!(batch.redaction.version, 1);
    assert_eq!(batch.candidates.len(), 6);
    assert_eq!(
        batch.candidates[0].title,
        "comemory.io feature gap map (2026-09-10)"
    );
}

#[test]
fn batch_redacts_planted_aws_key_in_a_body() {
    let mut extracted = fixture_extracted();
    let body = extracted[0].body.as_mut().unwrap();
    // Bare access-key shape (no `key=` prefix) so the format-anchored rule wins
    // over the client-only high-entropy heuristic.
    body.push_str("\nAKIAIOSFODNN7EXAMPLE\n");
    let batch = batch_from_extracted(&extracted).unwrap();
    let posted = batch.candidates[0].body.as_ref().unwrap();
    assert!(!posted.contains("AKIAIOSFODNN7EXAMPLE"));
    assert!(posted.contains("[REDACTED:aws-access-key-id]"));
    assert!(
        batch
            .redaction
            .findings
            .iter()
            .any(|f| f.rule == "aws-access-key-id" && f.count >= 1)
    );
}

#[test]
fn post_candidates_sends_authorization_only_and_returns_ordered_results() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            let n = stream.read(&mut tmp).unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                // Enough of the headers (and likely the body) to inspect auth.
                let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                let headers = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
                if headers.contains("content-length:") {
                    // Best-effort: stop once we have headers; body size is small.
                    break;
                }
            }
            if buf.len() > 1 << 20 {
                break;
            }
        }
        let req = String::from_utf8_lossy(&buf);
        let lower = req.to_ascii_lowercase();
        assert!(
            lower.contains("post /v1/sessions/sess-1/candidates"),
            "request={req}"
        );
        assert!(
            lower.contains("authorization: bearer cmk_test"),
            "request={req}"
        );
        assert!(!lower.contains("x-comemory-workspace"));
        let json = r#"{"ok":true,"data":{"results":[{"status":"stored","candidateId":"c1","contentDigest":"d1","state":"pending"},{"status":"duplicate","candidateId":"c0","contentDigest":"d0","state":"rejected"}],"stored":1,"duplicates":1},"meta":{}}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            json.len(),
            json
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let batch = batch_from_extracted(&fixture_extracted()[..2]).unwrap();
    let resp = post_candidates(&format!("http://{addr}"), "cmk_test", "sess-1", &batch).unwrap();
    assert_eq!(resp.stored, 1);
    assert_eq!(resp.duplicates, 1);
    assert_eq!(resp.results.len(), 2);
    assert_eq!(resp.results[0].status, "stored");
    assert_eq!(resp.results[1].status, "duplicate");
    assert_eq!(resp.results[1].state.as_deref(), Some("rejected"));
    handle.join().unwrap();
}
