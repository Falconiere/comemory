# api/sync/

**What belongs here:** the shared middle of the cloud-sync routes — the
memory three, `GET /sync/changes`, `GET /sync/manifest` and
`POST /sync/import`, and the code-index two, `GET /sync/code/manifest` and
`POST /sync/code/import` — moved out of `serve::routes` so the HTTP surface
and the CLI `comemory sync push/pull` path call one implementation (Binding
Rule 1).

**What does NOT belong here:** rate limits, device-key auth, GitHub App
allowlist enforcement (Worker rule 0), or cursor bookkeeping in
`sync_state` — those live on the platform side or in the CLI sync driver.

## Contents

| File | Primary item | Purpose |
| --- | --- | --- |
| `types.rs` | `SyncEntry` | Wire types (`SyncEntry`, `ImportRequest`, result statuses) |
| `changes.rs` | `run` | `GET /sync/changes` — enrich log rows above a cursor |
| `manifest.rs` | `run` | `GET /sync/manifest` — 256-bucket content-hash digests |
| `import.rs` | `run` | `POST /sync/import` batch orchestrator (≤500 entries) |
| `import_rules.rs` | `apply_entry` | Per-entry import rules 1–10 |
| `import_state.rs` | `validate_record` | Validation + live/trash/cursor probes |
| `import_write.rs` | `write_new_memory` | Markdown + SQLite mirror + sync log writes |
| `code_types.rs` | `CodeImportRequest` | Wire types of the code-index half: per-file `{path, blob_oid, symbols, imports}`, the co-change set, manifest and import envelopes — snippet-free by construction |
| `code_manifest.rs` | `run` | `GET /sync/code/manifest?repo=` — the workspace's `(path, blob_oid)` list plus `repo_marker` head and mining cursor; an unknown label answers empty |
| `code_import.rs` | `run` | `POST /sync/code/import` — one transaction per batch: replace files, drop removed ones, replace the co-change set, stamp the marker, `recompute_rank`. Any rejection refuses the whole batch |
| `code_import_rules.rs` | `validate` | The store-free rules a batch must pass first: relative paths, sane line ranges, the 500-file cap, the 200 000-symbol quota |
| `code_import_write.rs` | `write_file` | Per-file replace (symbols with empty snippets, outgoing `imports` edges, the `indexed_files` cursor; a matching blob is a no-op) and `remove_file` |

The module root is `src/api/sync.rs` (declares the submodules above).

The code half never touches a memory row or `sync_log`: the two logs stay
disjoint, and a code import creates no `code_fts` row — the projection has no
source text to index, so code search stays where the source is.

Local writes append to `sync_log` at the four API callers (`save`, `delete`,
`restore`, `update`) with `origin = local`; import appends with
`origin = sync`. Never inside `memory_row::insert`.
