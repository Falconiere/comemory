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
| `journal.rs` | `record_revision`, `record_tombstone` | What a document mutation owes the `replica-v1` feed, written inside the caller's transaction — at index time rather than push time, because once a tombstone has run nothing is left to describe what was removed |
| `replica_payload.rs` | `DocumentRevisionV1` | The wire revision and the id it must own: `shared_id` is the digest of the repository and the document's repository-relative path, so a payload altered in flight no longer owns its key |
| `share.rs` | `name_for`, `shared_id`, `blocked_reason` | The portable name a document is shared under, and the secret scan that refuses a revision whole — title, passages, headings and link targets alike |
| `unindex.rs` | `run` | `comemory unindex`: unregister one source and drop its derived rows |

Sub-folders: `document/` (extraction, chunking, fingerprinting, the index
writer) and `source/` (registry, classification, discovery, SQLite mirror), each
with its own README. When you add a file here, add its row above so the index
stays current. No `mod.rs` barrel — submodules are declared from
`src/domains/documents.rs` (`pub mod <name>;`) and callers import concrete paths.
