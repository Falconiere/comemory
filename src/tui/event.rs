//! Pure key-to-[`Action`] mapping for the explorer.
//!
//! Kept free of IO and state so it is exhaustively unit-testable: feed a
//! [`KeyEvent`], assert the [`Action`]. The event loop owns reading keys and
//! applying the resulting action to [`crate::tui::app::App`].
//!
//! Quit is `Esc` or `Ctrl-C` only — a bare `q` is a literal query character in
//! the live search box, so it cannot also mean "quit".

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::app::Action;

/// Decode a key press into a UI [`Action`]. Unbound keys map to
/// [`Action::Noop`]. Ctrl-chords take precedence over literal insertion.
pub fn map_key(key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match (key.code, ctrl) {
        (KeyCode::Char('c'), true) => Action::Quit,
        (KeyCode::Char('s'), true) => Action::Semantic,
        (KeyCode::Char('y'), true) => Action::CopyId,
        (KeyCode::Char('u'), true) => Action::ClearQuery,
        (KeyCode::Char(c), false) => Action::InsertChar(c),
        (KeyCode::Backspace, _) => Action::Backspace,
        (KeyCode::Up, _) => Action::SelectUp,
        (KeyCode::Down, _) => Action::SelectDown,
        (KeyCode::PageDown, _) => Action::PageNext,
        (KeyCode::PageUp, _) => Action::PagePrev,
        (KeyCode::Tab, _) => Action::SwitchTab,
        (KeyCode::Enter, _) => Action::Accept,
        (KeyCode::Esc, _) => Action::Quit,
        _ => Action::Noop,
    }
}
