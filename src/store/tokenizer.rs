//! Identifier-aware FTS5 tokenizer: pure splitting logic plus the FFI
//! registration that exposes it to SQLite as `tokenize = 'identifier'`.

pub mod ffi;
pub mod split;
