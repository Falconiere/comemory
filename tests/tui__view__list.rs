//! TestBackend render tests for the results-list widget.

use comemory::retrieval::rerank::{Reranked, ScoreParts};
use comemory::retrieval::router::Source;
use comemory::tui::app::App;
use comemory::tui::view::list;
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
fn list_renders_hit_rows_with_selection_marker() {
    let mut app = App::new(None, None, 12);
    app.set_memory_hits(
        vec![
            mem_hit("aaaa0001", "first body"),
            mem_hit("bbbb0002", "second body"),
        ],
        false,
    );

    let mut term = Terminal::new(TestBackend::new(80, 12)).expect("terminal");
    term.draw(|f| {
        let area = f.area();
        list::render(f, &app, area);
    })
    .expect("draw");

    let s = buf_string(&term);
    assert!(s.contains("aaaa0001"), "first hit id missing");
    assert!(s.contains("bbbb0002"), "second hit id missing");
    // The selection (row 0) is marked with the highlight symbol.
    assert!(s.contains("> aaaa0001"), "selection marker missing: {s}");
}
