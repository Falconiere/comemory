//! TestBackend render tests for the top-level frame layout.

use comemory::retrieval::rerank::{Reranked, ScoreParts};
use comemory::retrieval::router::Source;
use comemory::tui::app::App;
use comemory::tui::view::layout;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

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

fn buf_string(term: &Terminal<TestBackend>) -> String {
    term.backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect()
}

#[test]
fn renders_query_hits_and_chrome() {
    let mut app = App::new(None, Some("pool".to_string()), 12);
    app.set_memory_hits(vec![mem_hit("aaaa0001", "sqlite pool fix")], false);

    let mut term = Terminal::new(TestBackend::new(90, 20)).expect("terminal");
    term.draw(|f| layout::render(f, &app)).expect("draw");

    let s = buf_string(&term);
    assert!(s.contains("pool"), "search query missing");
    assert!(s.contains("aaaa0001"), "hit id missing");
    assert!(s.contains("results"), "results title missing");
    assert!(s.contains("preview"), "preview title missing");
}

#[test]
fn renders_empty_state_without_panic() {
    let app = App::new(None, None, 12);
    let mut term = Terminal::new(TestBackend::new(90, 20)).expect("terminal");
    term.draw(|f| layout::render(f, &app)).expect("draw");
    assert!(buf_string(&term).contains("comemory tui"), "header missing");
}
