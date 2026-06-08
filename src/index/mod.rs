//! Vector + lexical indexing now lives under [`crate::store`]
//! (`store::vector`, `store::fts`). The v0.1 LanceDB / fastembed /
//! tantivy submodules were removed in v0.2; this module is intentionally
//! empty so the `comemory::index` path stays available for any future
//! re-introduction without forcing a re-export break elsewhere.
