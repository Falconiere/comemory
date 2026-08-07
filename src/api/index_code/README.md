# api/index_code/

**What belongs here:** the walk-path internals of `api::index_code` — the
per-file steps the `comemory index-code` run repeats for every candidate
source file. Split out because the donor CLI file already sat near the
300-line ceiling.

**What does NOT belong here:** the command's entry point and `Request` /
`Response` shape, which stay in `src/api/index_code.rs`, and symbol
extraction itself, which is `ast::extract`'s job.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `walk.rs` | `index_file` | Per-file symbol extraction, blob-OID skip check, and import collection inside the caller's transaction |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/api/index_code.rs`
(`pub mod <name>;`) and callers import concrete paths.
