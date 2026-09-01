# api/rebuild/

**What belongs here:** the internals of `api::rebuild` that do not fit
alongside its entry point — today the `ATTACH`-based preservation copy that
carries the code index, learning state, and document-domain tables across
the atomic database swap. Split out because the donor CLI file already sat
near the 300-line ceiling.

**What does NOT belong here:** the command's entry point, `Request` /
`Response` shape, and the swap itself (including the pre-swap snapshot),
which stay in `src/api/rebuild.rs`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `copy.rs` | `copy_preserved_tables_from_old` | `ATTACH`-copy the code-index and learning-state tables into the freshly rebuilt database before the swap; declares `COPIED_TABLES`/`RECONSTRUCTABLE_TABLES`, the live-table allowlist pair the coverage test reads |
| `history.rs` | `copy_history_tables` | the run-history half of the preservation copy — `eval_runs` (+ v15's `discarded`), `gc_runs`, `index_runs` — called from `copy.rs`'s learning-state pass |
| `documents.rs` | `copy_document_tables_inner` | the document-domain half of the preservation copy — `source_files`, `documents`, `document_chunks`, `document_fts` — called from `copy.rs`; `source_roots`, the fifth v13 table, is deliberately reconciled from `sources.toml` instead, not copied here |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/api/rebuild.rs`
(`pub mod <name>;`) and callers import concrete paths.
