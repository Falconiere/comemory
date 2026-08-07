//! ratatui widgets for the explorer. Every function is a pure render from
//! `&App` into a frame region — no state, no IO — so the layout is snapshot-
//! testable against a `TestBackend`.

/// Top-level frame layout (search bar, list + preview split, status line).
pub mod layout;
/// The results-list widget.
pub mod list;
/// The preview-pane widget.
pub mod preview;
