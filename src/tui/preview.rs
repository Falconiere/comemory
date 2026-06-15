//! Preview text for the selected row (pure formatting).
//!
//! Memory previews show the full body, which `Reranked` already carries
//! in-hand — no extra disk read. Code previews show the symbol's identity and
//! score (the source snippet is not carried by `CodeReranked`, so v1 shows the
//! locator rather than re-reading the file).

use crate::tui::app::{App, Tab};

/// Build the preview pane's text for the currently selected row. Empty when
/// the active tab has no selection.
pub fn preview_text(app: &App) -> String {
    match app.tab {
        Tab::Memory => memory_preview(app),
        Tab::Code => code_preview(app),
    }
}

/// Memory preview: id + score/tier header, then the full body.
fn memory_preview(app: &App) -> String {
    match app.memory_hits.get(app.selected) {
        Some(h) => format!(
            "{}  (score {:.3}, tier {})\n\n{}",
            h.memory_id, h.parts.final_score, h.tier, h.body
        ),
        None => String::new(),
    }
}

/// Code preview: symbol, locator (`repo:path:lines`), kind/lang, and score.
fn code_preview(app: &App) -> String {
    match app.code_hits.get(app.selected) {
        Some(h) => format!(
            "{}\n{}:{}:{}-{}\n{} · {}  (score {:.3})",
            h.symbol, h.repo, h.path, h.line_start, h.line_end, h.kind, h.lang, h.parts.final_score
        ),
        None => String::new(),
    }
}
