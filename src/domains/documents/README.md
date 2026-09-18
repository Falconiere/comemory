# domains/documents/

**What belongs here:** the document capability end to end — registering an
external root, deciding which of its files are in scope, turning those files
into searchable rows, and removing them again. `source/` owns *which* files
exist and are eligible; `document/` owns *what is in* them; `index`, `sources`
and `unindex` are the command cores both `cli::` and `serve::routes::` call, so
neither surface duplicates the logic.

**What does NOT belong here:** SQL. Every `sources` / `source_files` /
`documents` / `document_chunks` / `document_fts` statement lives in
`store::{sources,documents,document_fts}`, and the schema declarations and
migration history stay in `store/` and the crate-root `migrations/`. Argument
parsing, TTY/`--json` rendering and HTTP concerns stay in `cli::` and `serve::`.
Document *retrieval* is `retrieval::doc_route`'s job: this capability writes the
rows it ranks. The shared exclusive-flock guard is `utilities::file_lock`, not
a file here.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `document.rs` | `ExtractedDocument` | Format-neutral extraction models (`DocumentFormat`, `Chunk`, `Block`) and the `document/` declarations |
| `index.rs` | `run` | `comemory index`: register one or more roots, then walk, extract and write every candidate |
| `source.rs` | `SourceId` | Source identity (`SourceKind`, `SourceEntry`) and the `source/` declarations |
| `sources.rs` | `run` | `comemory sources`: list registered sources, with a skippable reconcile side effect |
| `unindex.rs` | `run` | `comemory unindex`: unregister one source and drop its derived rows |

Sub-folders: `document/` (extraction, chunking, fingerprinting, the index
writer) and `source/` (registry, classification, discovery, SQLite mirror), each
with its own README. When you add a file here, add its row above so the index
stays current. No `mod.rs` barrel — submodules are declared from
`src/domains/documents.rs` (`pub mod <name>;`) and callers import concrete paths.
