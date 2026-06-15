//! The preview-pane widget: the selected row's detail, wrapped in a border.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::tui::app::App;
use crate::tui::preview;

/// Render the preview pane for the selected row into `area`.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let text = preview::preview_text(app);
    let para = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("preview"))
        .wrap(Wrap { trim: false });
    frame.render_widget(para, area);
}
