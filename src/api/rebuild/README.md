# api/rebuild/

**What belongs here:** the internals of `api::rebuild` that do not fit
alongside its entry point — today the `ATTACH`-based preservation copy that
carries the code index and learning state across the atomic database swap.
Split out because the donor CLI file already sat near the 300-line ceiling.

**What does NOT belong here:** the command's entry point, `Request` /
`Response` shape, and the swap itself, which stay in `src/api/rebuild.rs`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `copy.rs` | `copy_preserved` | `ATTACH`-copy the code-index and learning-state tables into the freshly rebuilt database before the swap |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/api/rebuild.rs`
(`pub mod <name>;`) and callers import concrete paths.
