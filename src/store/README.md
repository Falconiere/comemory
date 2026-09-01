# store/

**What belongs here:** the central SQLite layer backing `comemory.db` —
connection setup (PRAGMAs, migrations, `sqlite-vec` auto-extension), the
versioned migration runner, FTS5 helpers for both the memory and code legs,
`vec0` insert/KNN with dim guards, the `edges` FTS triplet index, batched
per-memory metadata, the identifier tokenizer, and row CRUD for memories,
code symbols, code references, documents, and the source-registry mirror.

**What does NOT belong here:** ranking or business logic. `store/` only reads
and writes rows; `retrieval/` decides what to query and how to combine
results, and `graph/` decides what edges mean.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `code_ref.rs` | `CodeRefRow` | `code_ref` side table: version-anchor store for explicit code references |
| `code_row.rs` | `CodeSymbolRow` | `code_symbols` row upserts (insert, refresh, delete-by-file) |
| `connection.rs` | `open` | Connection open: PRAGMAs, migrations, `sqlite-vec` auto-extension registration |
| `document_fts.rs` | `DocumentFtsHit` | `document_fts` insert/delete helpers + the BM25 MATCH query leg |
| `documents.rs` | `DocumentUpsert` | `documents` + `document_chunks` row CRUD |
| `edge_fts.rs` | `EdgeFtsHit` | FTS5 triplet index over `edges`: rendering, refresh, and the `comemory edges` lexical ladder |
| `embed.rs` | `to_vec_blob` | f32 ↔ `vec0` BLOB encoding plus the per-table dim guards |
| `fts.rs` | `CodeFtsHit` | FTS5 insert/search helpers for the code leg |
| `fts_memory.rs` | `MemoryFtsHit` | Memory-leg FTS5 ladder (strict → relaxed → subtoken → expanded) behind `run_memory_match` |
| `memory_list.rs` | `ListRow` | Paginated listing of live memories |
| `memory_meta.rs` | `MemoryMeta` | Batched per-memory metadata: path, repo, kind, tags, references |
| `memory_row.rs` | `insert` | `memories` row upserts and their edge materialization |
| `migrate.rs` | `CURRENT_VERSION` | Versioned, idempotent schema migrations plus `schema_meta`; loops over the `MIGRATIONS` slice declared in `migrate/list.rs` |
| `schema.rs` | — | Module-doc placeholder for the v0.2 schema; DDL text lives in `sql/` |
| `simhash_scan.rs` | `SimhashRow` | Bulk `(id, simhash)` scan over live memories, shared by save + consolidate |
| `eval_runs.rs` | `insert` | `eval_runs` writer + newest-first reader — one row per eval/tune/bandit RUN, never per scored candidate |
| `gc_runs.rs` | `insert` | `gc_runs` writer — one row per completed `comemory gc` sweep, with bytes freed |
| `random_id.rs` | `random_hex` | Shared random-hex id helper, moved out of `serve::security` so non-HTTP callers can use it |
| `sources.rs` | `SourceRootUpsert` | `source_roots` row CRUD — the SQLite mirror of `sources.toml` |
| `tokenizer.rs` | — | Parent declaration for the `tokenizer/` folder (see `store/tokenizer/README.md`) |
| `vector.rs` | `MemoryHit` | `vec0` insert and KNN against `memory_vec` / `code_vec` |

`sql/` (migration DDL), `tokenizer/` (FTS5 tokenizer FFI), and `migrate/`
(the `MIGRATIONS` slice plus the migration preflight/snapshot safety net) are
documented in their own `README.md` per the guardrails nested-folder rule.

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/store.rs` (`pub mod
<name>;`) and callers import concrete paths.
