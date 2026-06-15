//! The results-list widget: the active tab's hits, selection highlighted.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use crate::retrieval::code_rerank::CodeReranked;
use crate::retrieval::rerank::Reranked;
use crate::tui::app::{App, Tab};

/// Render the active tab's hits into `area` as a selectable list.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = match app.tab {
        Tab::Memory => app
            .memory_hits
            .iter()
            .map(|h| ListItem::new(memory_row(h)))
            .collect(),
        Tab::Code => app
            .code_hits
            .iter()
            .map(|h| ListItem::new(code_row(h)))
            .collect(),
    };
    let widget = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("results"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    let mut state = ListState::default();
    if app.active_len() > 0 {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(widget, area, &mut state);
}

/// One memory row: id + the body's first line.
fn memory_row(h: &Reranked) -> String {
    let first = h.body.lines().next().unwrap_or("");
    format!("{}  {}", h.memory_id, first)
}

/// One code row: symbol + `path:line`.
fn code_row(h: &CodeReranked) -> String {
    format!("{}  {}:{}", h.symbol, h.path, h.line_start)
}
