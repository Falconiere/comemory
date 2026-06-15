//! TestBackend render tests for the preview-pane widget.

use comemory::retrieval::rerank::{Reranked, ScoreParts};
use comemory::retrieval::router::Source;
use comemory::tui::app::App;
use comemory::tui::view::preview;
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
fn preview_pane_shows_selected_body() {
    let mut app = App::new(None, None, 12);
    app.set_memory_hits(vec![mem_hit("aaaa0001", "uniquebodytoken")], false);

    let mut term = Terminal::new(TestBackend::new(60, 16)).expect("terminal");
    term.draw(|f| {
        let area = f.area();
        preview::render(f, &app, area);
    })
    .expect("draw");

    let s = buf_string(&term);
    assert!(s.contains("preview"), "preview title missing");
    assert!(s.contains("uniquebodytoken"), "body text missing: {s}");
}
