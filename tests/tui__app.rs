//! Unit tests for the explorer's pure UI state transitions ([`App::apply`]).
//!
//! No terminal, no store — exercises query edits, tab switching, paging, the
//! generation counter, and selection clamping directly on the state machine.

use comemory::retrieval::rerank::{Reranked, ScoreParts};
use comemory::retrieval::router::Source;
use comemory::tui::app::{Action, App, Effect, Tab};

/// Mint a memory hit with a given id for selection/clamp tests.
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
        body: format!("body {id}"),
        simhash: 0,
    }
}

#[test]
fn insert_char_rewinds_window_and_dispatches_search() {
    let mut app = App::new(None, None, 12);
    app.window.offset = 24;
    app.selected = 3;
    app.enriched = true;
    let before = app.seq;

    let eff = app.apply(Action::InsertChar('a'));

    assert_eq!(eff, Effect::Search);
    assert_eq!(app.query, "a");
    assert_eq!(app.window.offset, 0);
    assert_eq!(app.selected, 0);
    assert!(!app.enriched);
    assert_eq!(app.seq, before + 1);
}

#[test]
fn backspace_pops_last_char() {
    let mut app = App::new(None, Some("ab".to_string()), 12);
    let eff = app.apply(Action::Backspace);
    assert_eq!(eff, Effect::Search);
    assert_eq!(app.query, "a");
}

#[test]
fn clear_query_empties_and_searches() {
    let mut app = App::new(None, Some("abc".to_string()), 12);
    app.apply(Action::ClearQuery);
    assert_eq!(app.query, "");
}

#[test]
fn switch_tab_toggles_and_dispatches_search() {
    let mut app = App::new(None, None, 12);
    assert_eq!(app.tab, Tab::Memory);
    app.enriched = true;

    let eff = app.apply(Action::SwitchTab);

    assert_eq!(eff, Effect::Search);
    assert_eq!(app.tab, Tab::Code);
    assert_eq!(app.window.offset, 0);
    assert!(!app.enriched);
}

#[test]
fn semantic_is_memory_only() {
    let mut app = App::new(None, Some("q".to_string()), 12);
    app.has_embedder = true;
    assert_eq!(app.apply(Action::Semantic), Effect::Semantic);

    app.tab = Tab::Code;
    let eff = app.apply(Action::Semantic);
    assert_eq!(eff, Effect::Redraw);
    assert!(
        app.status.contains("Memory-tab only"),
        "status: {}",
        app.status
    );
}

#[test]
fn semantic_without_embedder_does_not_bump_seq() {
    // Regression: pressing Ctrl-S with no embedder configured must not advance
    // the generation counter — otherwise an in-flight lexical response (tagged
    // with the prior seq) is silently discarded.
    let mut app = App::new(None, Some("q".to_string()), 12);
    assert!(!app.has_embedder);
    let before = app.seq;

    let eff = app.apply(Action::Semantic);

    assert_eq!(eff, Effect::Redraw, "no-embedder Ctrl-S must not dispatch");
    assert_eq!(app.seq, before, "no-embedder Ctrl-S must not bump seq");
    assert!(
        app.status.contains("no embed command"),
        "status: {}",
        app.status
    );
}

#[test]
fn page_next_requires_has_more() {
    let mut app = App::new(None, None, 12);
    assert_eq!(app.apply(Action::PageNext), Effect::Redraw);
    assert_eq!(app.window.offset, 0);

    app.has_more = true;
    let eff = app.apply(Action::PageNext);
    assert_eq!(eff, Effect::Search);
    assert_eq!(app.window.offset, 12);
}

#[test]
fn page_prev_is_noop_on_first_page() {
    let mut app = App::new(None, None, 12);
    assert_eq!(app.apply(Action::PagePrev), Effect::Redraw);

    app.window.offset = 24;
    let eff = app.apply(Action::PagePrev);
    assert_eq!(eff, Effect::Search);
    assert_eq!(app.window.offset, 12);
}

#[test]
fn selection_clamps_to_active_hits() {
    let mut app = App::new(None, None, 12);
    app.set_memory_hits(vec![mem_hit("a1b2c3d4"), mem_hit("b2c3d4e5")], false);

    app.apply(Action::SelectDown);
    assert_eq!(app.selected, 1);
    app.apply(Action::SelectDown); // already at the last row
    assert_eq!(app.selected, 1);
    app.apply(Action::SelectUp);
    assert_eq!(app.selected, 0);
    assert_eq!(app.selected_id().as_deref(), Some("a1b2c3d4"));
}

#[test]
fn set_hits_clamps_stale_selection() {
    let mut app = App::new(None, None, 12);
    app.selected = 5;
    app.set_memory_hits(vec![mem_hit("only0001")], false);
    assert_eq!(app.selected, 0);
}

#[test]
fn copy_id_without_selection_reports_empty() {
    let mut app = App::new(None, None, 12);
    app.apply(Action::CopyId);
    assert!(
        app.status.contains("no selection"),
        "status: {}",
        app.status
    );
}

#[test]
fn accept_and_quit_map_to_their_effects() {
    assert_eq!(
        App::new(None, None, 12).apply(Action::Accept),
        Effect::Accept
    );
    assert_eq!(App::new(None, None, 12).apply(Action::Quit), Effect::Quit);
}
