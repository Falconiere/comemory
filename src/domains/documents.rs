//! The document capability: turning registered external roots into searchable
//! document rows, and back out again.
//!
//! `source` owns *which* files are in scope (the `sources.toml` registry, its
//! exclusive-flock guard, classification, the discovery walk, and the SQLite
//! `source_roots` mirror). `document` owns *what is in* them (format
//! extraction, chunking, fingerprinting, and the per-file index writer).
//! `index`, `sources` and `unindex` are the command cores `cli::` and
//! `serve::routes::` both call, so neither surface duplicates the logic.
//!
//! All SQL stays in `store`; this capability calls it and never writes its own.

/// Pure in-process extraction (TXT/Markdown/HTML/CSV), chunking, and the
/// per-file index writer.
pub mod document;
/// `comemory index`: register document sources and reconcile them.
pub mod index;
/// A document's portable name: the digest of its canonical repository and
/// its normalized repository-relative path.
pub mod share;
/// Durable source registry, classification, discovery, and the SQLite mirror.
pub mod source;
/// `comemory sources`: list registered sources, optionally reconciling first.
pub mod sources;
/// `comemory unindex`: unregister a document source and its derived rows.
pub mod unindex;
