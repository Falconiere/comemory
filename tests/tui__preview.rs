//! Tests for the pure preview-text formatter ([`preview_text`]).

use comemory::retrieval::code_rerank::{CodeReranked, CodeScoreParts};
use comemory::retrieval::rerank::{Reranked, ScoreParts};
use comemory::retrieval::router::Source;
use comemory::tui::app::{App, Tab};
use comemory::tui::preview::preview_text;

fn mem_hit(id: &str, body: &str) -> Reranked {
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
            final_score: 0.5,
        },
        superseded_by: None,
        body: body.to_string(),
        simhash: 0,
    }
}

fn code_hit() -> CodeReranked {
    CodeReranked {
        symbol_id: 1,
        repo: "r".to_string(),
        path: "src/a.rs".to_string(),
        symbol: "alpha_fn".to_string(),
        kind: "function".to_string(),
        lang: "rust".to_string(),
        line_start: 5,
        line_end: 9,
        source: Source::Lexical,
        parts: CodeScoreParts {
            relevance: 1.0,
            rank: 1.0,
            activation: 1.0,
            affinity: 1.0,
            feedback: 1.0,
            final_score: 0.42,
        },
    }
}

#[test]
fn memory_preview_has_id_and_body() {
    let mut app = App::new(None, None, 12);
    app.set_memory_hits(
        vec![mem_hit("aaaa0001", "the full memory body here")],
        false,
    );
    let text = preview_text(&app);
    assert!(text.contains("aaaa0001"), "id missing: {text}");
    assert!(
        text.contains("the full memory body here"),
        "body missing: {text}"
    );
}

#[test]
fn empty_preview_when_no_selection() {
    let app = App::new(None, None, 12);
    assert_eq!(preview_text(&app), "");
}

#[test]
fn code_preview_has_symbol_and_locator() {
    let mut app = App::new(None, None, 12);
    app.tab = Tab::Code;
    app.set_code_hits(vec![code_hit()], false);
    let text = preview_text(&app);
    assert!(text.contains("alpha_fn"), "symbol missing: {text}");
    assert!(text.contains("src/a.rs:5-9"), "locator missing: {text}");
}
