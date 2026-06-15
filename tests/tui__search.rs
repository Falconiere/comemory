//! Tests for the render-side search bridge (request building + response
//! application + stale-seq discard).

use comemory::retrieval::rerank::{Reranked, ScoreParts};
use comemory::retrieval::router::Source;
use comemory::tui::app::{Action, App};
use comemory::tui::search::{apply_response, build_request, is_current};
use comemory::tui::worker::{Hits, Response};

/// Mint a memory hit with a given id.
fn mem_hit(id: &str) -> Reranked {
    Reranked {
        memory_id: id.to_string(),
        source: Source::Lexical,
        tier: 1,
        parts: ScoreParts {
            rrf: 1.0,
            activation: 1.0,
            feedback: 1.0,
            quality: 1.0,
            supersede: 1.0,
            final_score: 1.0,
        },
        superseded_by: None,
        body: "b".to_string(),
        simhash: 0,
    }
}

/// A Memory response at a given generation.
fn mem_response(seq: u64, semantic: bool, ids: &[&str], has_more: bool) -> Response {
    Response {
        seq,
        semantic,
        result: Ok(Hits::Memory {
            hits: ids.iter().map(|i| mem_hit(i)).collect(),
            has_more,
        }),
    }
}

#[test]
fn lexical_request_carries_no_embed() {
    let app = App::new(Some("repo1".to_string()), Some("q".to_string()), 12);
    let req = build_request(&app, Some("embedder"), false);
    assert_eq!(req.query, "q");
    assert_eq!(req.seq, app.seq);
    assert_eq!(req.repo.as_deref(), Some("repo1"));
    assert!(req.embed.is_none());
    assert!(req.vec.is_none());
}

#[test]
fn semantic_request_carries_embed_cmd() {
    let app = App::new(None, Some("q".to_string()), 12);
    let req = build_request(&app, Some("embedder"), true);
    assert_eq!(req.embed.as_deref(), Some("embedder"));
}

#[test]
fn semantic_without_embedder_has_no_cmd() {
    let app = App::new(None, Some("q".to_string()), 12);
    let req = build_request(&app, None, true);
    assert!(req.embed.is_none());
}

#[test]
fn current_response_applies_hits() {
    let mut app = App::new(None, Some("q".to_string()), 12);
    let seq = app.seq;
    apply_response(&mut app, mem_response(seq, false, &["aaaa0001"], true));
    assert_eq!(app.memory_hits.len(), 1);
    assert!(app.has_more);
    assert!(!app.enriched);
}

#[test]
fn semantic_response_sets_enriched() {
    let mut app = App::new(None, Some("q".to_string()), 12);
    let seq = app.seq;
    apply_response(&mut app, mem_response(seq, true, &["aaaa0001"], false));
    assert!(app.enriched);
}

#[test]
fn stale_response_is_discarded() {
    let mut app = App::new(None, Some("q".to_string()), 12);
    // Advance the generation past the response's seq.
    app.apply(Action::InsertChar('x'));
    assert_eq!(app.seq, 1);

    let stale = mem_response(0, false, &["zzzz0009"], false);
    assert!(!is_current(&app, &stale));
    apply_response(&mut app, stale);
    assert!(app.memory_hits.is_empty(), "stale hits must not be applied");
}

#[test]
fn error_response_sets_status() {
    let mut app = App::new(None, Some("q".to_string()), 12);
    let resp = Response {
        seq: app.seq,
        semantic: true,
        result: Err("embed-cmd timed out".to_string()),
    };
    apply_response(&mut app, resp);
    assert!(app.status.contains("timed out"), "status: {}", app.status);
}
