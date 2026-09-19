# `utilities/`

**What belongs here:** transport-neutral shared primitives — anything a
capability, `cli`, and `serve` may all need, with exactly one implementation
each. One concern per file, named after it, so a caller reaches the concern
directly instead of through a technical-layer barrel. Created by
[#166](https://github.com/Falconiere/comemory/issues/166) as part of the
[domain-first migration](../../docs/designs/2026-09-17-domain-first-migration.md).

**What does NOT belong here:** anything owned by one capability, and anything
that belongs to a delivery adapter. Nothing in this folder may import `cli`,
`serve`, or `output`: clap argument structs, stdin/stdout handling, JSON/TTY
emission, and HTTP status/envelope mapping all stay in delivery, and a
command's result model stays with the capability that produces it.

Four helpers still name a capability-owned *type* whose owner is deliberately
not moving here — `id_list` delegates memory-id policy to
`memory::id::is_valid_memory_id`, `when` returns `retrieval::scope::TimeScope`
and formats bounds through `store::memory_row`, and `ref_args` builds
`memory::References` from `git_utils` lookups. Those owners move under
`domains::` in #167/#169/#171; the direction (shared → capability) stays legal
for a shared utility under the final tree.

Two files here reach into a capability and are declared for it in
`scripts/architecture-policy.json`'s `shared_domain_dependencies` (#178):
`id_list` validates a memory id through `domains::memories::id`, and
`ref_args` resolves blob OIDs through `domains::code::git_utils` and produces
`domains::memories::{Ref, References}` — the parsed value *is* a memories
model. Anything not on that list is a gate failure (`shared layer
dependency`), so a new one cannot appear unnoticed.

## Contents

One line per file, named after its primary item:

| File | Primary item | Owns |
| --- | --- | --- |
| `context.rs` | `Ctx` | The execution context every command core runs against: `Paths` + `Config` plus a borrowed or lazily opened connection, so conn-free commands never touch the database and a job worker gets exactly one connection |
| `dated_id.rs` | `dated_id` | The shared `<prefix>-<yyyymmdd>-<8hex>` id shape, minted and validated in one place, so the `q-` retrieval query id and the `o-` observation id keep one form |
| `digest.rs` | `sha256_hex` | SHA-256 hex digests and `is_lower_hex`, the shape check the memory-id and query-id contracts share |
| `embed.rs` | `embed_query` | The `COMEMORY_EMBED_CMD` shell-out, bounded by `EMBED_TIMEOUT`, parsing the child's JSON payload through `embedding_input` |
| `embedding_input.rs` | `parse_payload` | Pure decoding of a `--vector` CSV list and a `{"embedding":[..]}` JSON payload — no process I/O |
| `fetch.rs` | `exchange` | `curl` (falling back to `wget`) HTTP: `exchange`, `download`, `final_url`; no in-process TLS stack |
| `file_lock.rs` | `FileLock` | Exclusive advisory lock over a sibling lock file, held by the source registry's read-modify-write cycle and by `store::migrate`'s preflight snapshot |
| `http_error.rs` | `map_reqwest` | Outbound `reqwest` transport errors mapped into `crate::Error` with the source chain preserved |
| `id_list.rs` | `csv_unique` | Comma-separated flag values split, trimmed, de-duplicated in first-mention order, plus the memory-id and symbol-id list parsers built on it |
| `pagination.rs` | `Page` | The requested `PageWindow`, the generic `{items, limit, offset, total, has_more}` envelope, the retrieval `PageMeta` cursor, and the `page_window` / `page_meta` builders — the single home of the `limit == 0` means "all" rule |
| `path_containment.rs` | `resolve_within` | Canonicalize-and-contain checks: `resolve_within` for a repo-relative `file:<repo>:<path>` id and `contain_abs` for a caller-supplied absolute path; both reject `..`, absolute, NUL, and symlink escapes |
| `progress.rs` | `ProgressSink` | The progress / cancellation contract a long-running walk reports through and a job worker consumes |
| `query_id.rs` | `generate_query_id` | The `q-<yyyymmdd>-<8hex>` retrieval-log id: mint and validate, kept out of the learning capability so retrieval does not depend on it |
| `repo_root.rs` | `resolve_root` | Resolve a `file:<repo>:<path>` node id to an absolute file on disk, and the `RootOverrides` map a caller may layer over the stored `repo_marker` roots — the one repository resolver `cli`, `serve`, `retrieval::code_ref_fetch` and `domains::memories::refresh_refs` share |
| `ref_args.rs` | `collect` | The `--ref-file` / `--ref-symbol` values qualified, rewritten repo-root-relative, and anchored to the HEAD blob into a `References` block |
| `simhash.rs` | `simhash64` | 64-bit SimHash, Hamming distance, and the `NEAR_DUP_HAMMING` near-duplicate radius |
| `telemetry.rs` | `PROV_MANUAL` | The persisted vocabularies: `retrieval_log.source`, `feedback_events.target_kind`, the four `provenance` values, and the two auto-reinforcement sentinel query ids |
| `vector_stdin.rs` | `read_optional` | Acquiring a caller-supplied vector from the flag pair and process stdin, under the 8 MiB payload cap — the only file here that reads stdin |
| `when.rs` | `parse_when` | `--since` / `--until` / `--as-of` value parsing, including the bare-date day-edge expansion. Building the window from three parsed instants is `domains::retrieval::scope::scope_from_flags` — `TimeScope` is the retrieval capability's, and a shared module must not reach into one (#178) |

`src/lib.rs` re-exports `embed`, `fetch`, `http_error` and `simhash` at the
crate root so `comemory::<name>` keeps resolving for external consumers; in-crate
code names `crate::utilities::<name>` directly.

Colocated unit tests live in `tests/` beside this file and are reached through
each module's `#[path]` bridge.
