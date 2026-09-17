# `domains/memories/`

**What belongs here:** the memory lifecycle end to end — the markdown record
(`Frontmatter`, slug, content-derived id, versioned references), the atomic
markdown store over `memories/{id}-{slug}.md`, and the command cores both
delivery adapters call to save, read, patch, soft-delete, restore and re-pin a
memory.

**What does NOT belong here:** SQL, and delivery. Every `rusqlite` import and
SQL string stays in `store` (`store::memory_row`, `store::memory_purge`,
`store::memory_meta`, `store::memory_list`, `store::trash_list`, …); this folder
calls those. clap flag declarations, process I/O and the inline cloud push after
a write stay in `cli`; HTTP status and envelope mapping stays in `serve`;
rendering stays in `output`. Ranking a memory hit is retrieval's job.

## Contents

One line per file, named after its primary item:

| File | Primary item | Owns |
| --- | --- | --- |
| `delete.rs` | `Response` | Shared middle of `comemory delete` / `DELETE /api/v1/memories/{id}`, plus `soft_delete` (markdown into `memories/.trash/`, then the mirror) and `mirror_soft_delete` (the `deleted_at` stamp, FTS/vector drop and edge purge in one transaction) — the two helpers `comemory prune`'s apply and heal paths and the sync import reuse so no deletion surface can drift |
| `frontmatter.rs` | `Kind` | YAML frontmatter struct plus markdown split/render helpers |
| `id.rs` | `memory_id` | Deterministic 8-hex memory id derived from the body content hash, and the shape check every id-taking surface validates against |
| `list.rs` | `Request` | Shared middle of `comemory list` / `GET /api/v1/memories` — paging live memories out of the SQLite mirror with optional `repo`/`kind` filters |
| `nav.rs` | `title_of` | The two derived fields every memory listing reports: a memory's title (the first non-empty line of its body — the rule `save`'s title folding compares against) and the absolute path of its markdown file |
| `prior.rs` | `Prior` | `MemoryStore::prior` — the frontmatter facts (`created`, `content_hash`, `trashed`) the live file or its `.trash/` copy already holds for an id, read before a save so `save` can refuse a same-id different-body collision, carry `created` across a replay, and report `created: bool`; also backs the sync import's collision rule |
| `references.rs` | `Ref` | Versioned code reference (file/symbol pointer + captured anchor), string-or-struct serde |
| `refresh_refs.rs` | `Response` | Console-only: `POST /api/v1/memories/{id}/references/refresh` — re-pin every anchored reference to the current HEAD through `utilities::repo_root::resolve_root` (explicit `--root` override before the stored `repo_marker.root_path`), writing through `update::mirror_record` rather than a second write path |
| `restore.rs` | `Response` | Console-only: `POST /api/v1/memories/{id}/restore` and `POST /api/v1/trash/{id}/restore` — the exact reverse of `soft_delete`, re-deriving the incoming relation edges the restored markdown cannot regenerate |
| `save.rs` | `Request` | Shared middle of `comemory save` / `POST /api/v1/memories` — the content-addressed replay contract, `supersedes` and `ref_*` validation ahead of every effect, the near-duplicate advisory, and the atomic markdown write plus SQLite mirror |
| `show.rs` | `Request` | Shared middle of `comemory show` / `GET /api/v1/memories/{id}` — body, frontmatter, activation and code-reference freshness in one round trip |
| `slug.rs` | `slug_from_body` | Filesystem-safe slug derivation for memory filenames |
| `store.rs` | `SaveParams` | Markdown-backed memory store: atomic save (purges a same-id `.trash/` copy — a re-saved body is live again) / rewrite-in-place / load / list / soft-delete (stamps the trashed file's mtime as the deletion instant, the clock gc reads) / restore-from-trash (checks the live tree FIRST so a stale trash copy is never renamed over a live re-save) |
| `trash.rs` | `Request` | Console-only: `GET /api/v1/trash` — soft-deleted memories with their days until gc, counted off the trashed file's mtime and never creating the database |
| `update.rs` | `Request` | Console-only: `PATCH /api/v1/memories/{id}` — a frontmatter-only patch in place, or a body patch as a superseding re-save through `save::run_with`; `mirror_record` is the one re-mirror path `refresh_refs` shares |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/domains/memories.rs`
(`pub mod <name>;`) and callers import concrete paths.

`src/lib.rs` re-exports this module as `memory` at the crate root so
`comemory::memory::…` keeps resolving for external consumers; in-crate code
names `crate::domains::memories::<name>` directly.

Colocated unit tests live in `tests/` beside their module and are reached through
each module's `#[path]` bridge.
