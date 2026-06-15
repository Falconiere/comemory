//! Top-level frame layout: search bar, results/preview split, status line.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::tui::app::App;
use crate::tui::view::{list, preview};

/// Render the full explorer frame from `app`.
pub fn render(frame: &mut Frame, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(frame.area());
    render_search(frame, app, rows[0]);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(rows[1]);
    list::render(frame, app, cols[0]);
    preview::render(frame, app, cols[1]);
    render_status(frame, app, rows[2]);
}

/// The search box: a bordered prompt showing the live query and active tab.
fn render_search(frame: &mut Frame, app: &App, area: Rect) {
    let title = format!("comemory tui — {} (Tab to switch)", app.tab.label());
    let para = Paragraph::new(format!("> {}", app.query))
        .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(para, area);
}

/// The one-line status/help bar: counts, flags, hint, and key legend.
fn render_status(frame: &mut Frame, app: &App, area: Rect) {
    let more = if app.has_more { " +more" } else { "" };
    let sem = if app.enriched { " semantic" } else { "" };
    let hint = if app.status.is_empty() {
        "Ctrl-S enrich · Ctrl-Y id · Enter pick · Esc quit".to_string()
    } else {
        app.status.clone()
    };
    let line = format!("{} hit(s){}{} · {}", app.active_len(), more, sem, hint);
    frame.render_widget(Paragraph::new(line), area);
}
