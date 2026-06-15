//! Unit tests for the pure key-to-[`Action`] mapping ([`map_key`]).

use comemory::tui::app::Action;
use comemory::tui::event::map_key;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A plain key press (no modifiers).
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// A Ctrl-chord key press.
fn ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

#[test]
fn printable_chars_insert() {
    assert_eq!(map_key(key(KeyCode::Char('a'))), Action::InsertChar('a'));
    assert_eq!(map_key(key(KeyCode::Char(' '))), Action::InsertChar(' '));
    // 'q' is a literal query character, NOT a quit key.
    assert_eq!(map_key(key(KeyCode::Char('q'))), Action::InsertChar('q'));
}

#[test]
fn ctrl_chords_bind_actions() {
    assert_eq!(map_key(ctrl(KeyCode::Char('c'))), Action::Quit);
    assert_eq!(map_key(ctrl(KeyCode::Char('s'))), Action::Semantic);
    assert_eq!(map_key(ctrl(KeyCode::Char('y'))), Action::CopyId);
    assert_eq!(map_key(ctrl(KeyCode::Char('u'))), Action::ClearQuery);
}

#[test]
fn navigation_keys_map() {
    assert_eq!(map_key(key(KeyCode::Backspace)), Action::Backspace);
    assert_eq!(map_key(key(KeyCode::Up)), Action::SelectUp);
    assert_eq!(map_key(key(KeyCode::Down)), Action::SelectDown);
    assert_eq!(map_key(key(KeyCode::PageDown)), Action::PageNext);
    assert_eq!(map_key(key(KeyCode::PageUp)), Action::PagePrev);
    assert_eq!(map_key(key(KeyCode::Tab)), Action::SwitchTab);
    assert_eq!(map_key(key(KeyCode::Enter)), Action::Accept);
    assert_eq!(map_key(key(KeyCode::Esc)), Action::Quit);
}

#[test]
fn unbound_keys_are_noop() {
    assert_eq!(map_key(ctrl(KeyCode::Char('z'))), Action::Noop);
    assert_eq!(map_key(key(KeyCode::F(5))), Action::Noop);
}
