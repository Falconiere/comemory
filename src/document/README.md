# document/

**What belongs here:** pure, in-process document extraction (TXT, Markdown,
HTML/XHTML, CSV/TSV) and size-bounded chunking. Every extractor turns
already-read bytes into an `ExtractedDocument` of `Chunk`s; no schema or store
dependency lives below `writer.rs`.

**What does NOT belong here:** the SQLite row storage of extracted documents
and chunks — `documents`/`document_chunks`/`document_fts` row CRUD lives in
`store::documents` and `store::document_fts`. `writer.rs` is the one file here
that crosses into orchestration: it owns the fingerprint skip-check and the
one-transaction row replacement, calling into `store::` to do it.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `chunk.rs` | `split` | Split a document's `Block`s into size-bounded `Chunk`s at paragraph boundaries |
| `delimited.rs` | `extract` | CSV/TSV extraction via the `csv` crate, one row per line |
| `extract.rs` | `extract` | Format dispatch plus the TXT/Markdown extractors |
| `fingerprint.rs` | `FileStat` | Per-candidate fingerprint bundle, size+mtime skip check, SHA-256 identity hashing |
| `html.rs` | `extract` | HTML/XHTML extraction via the `tl` crate (recursive DOM walk) |
| `writer.rs` | `UpdateOutcome` | Per-file index writer: fingerprint skip, extraction, one-transaction row replacement |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/document.rs` (`pub mod
<name>;`) and callers import concrete paths.
