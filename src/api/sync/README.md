# api/sync/

**What belongs here:** the shared middle of the three cloud-sync routes —
`GET /sync/changes`, `GET /sync/manifest`, and `POST /sync/import` — moved
out of `serve::routes` so the HTTP surface and the CLI `comemory sync
push/pull` path call one implementation (Binding Rule 1).

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

The module root is `src/api/sync.rs` (declares the submodules above).

Local writes append to `sync_log` at the four API callers (`save`, `delete`,
`restore`, `update`) with `origin = local`; import appends with
`origin = sync`. Never inside `memory_row::insert`.
