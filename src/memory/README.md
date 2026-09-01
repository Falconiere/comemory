# memory/

**What belongs here:** the markdown-as-source-of-truth layer — `Frontmatter`
parsing/rendering, slug and id derivation, and the atomic save / load / list /
soft-delete store over `memories/{id}-{slug}.md`.

**What does NOT belong here:** the SQLite mirror of a memory row. `memory/`
only touches the filesystem; `store::memory_row` owns the `memories` table
upsert and edge materialization that runs alongside every save.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `frontmatter.rs` | `Kind` | YAML frontmatter struct plus markdown split/render helpers |
| `id.rs` | `memory_id` | Deterministic 8-hex memory id derived from the body content hash |
| `references.rs` | `Ref` | Versioned code reference (file/symbol pointer + captured anchor), string-or-struct serde |
| `slug.rs` | `slug_from_body` | Filesystem-safe slug derivation for memory filenames |
| `store.rs` | `SaveParams` | Markdown-backed memory store: atomic save (purges a same-id `.trash/` copy — a re-saved body is live again) / rewrite-in-place / load / list / soft-delete (stamps the trashed file's mtime as the deletion instant, the clock gc reads) / restore-from-trash (checks the live tree FIRST so a stale trash copy is never renamed over a live re-save) |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/memory.rs` (`pub mod
<name>;`) and callers import concrete paths.
