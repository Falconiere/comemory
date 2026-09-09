# api/rebuild/

**What belongs here:** the internals of `api::rebuild` that do not fit
alongside its entry point — today just the live-table allowlist pair
(`COPIED_TABLES`/`RECONSTRUCTABLE_TABLES`) the coverage test reads, plus the
thin delegate that runs the allowlist sanity check before handing off to
`crate::store::rebuild_copy`, which owns the actual `ATTACH`-based
preservation copy. Split out because the donor CLI file already sat near the
300-line ceiling.

**What does NOT belong here:** the command's entry point, `Request` /
`Response` shape, and the swap itself (including the pre-swap snapshot),
which stay in `src/api/rebuild.rs`. The per-table `ATTACH` copy SQL itself
lives in `src/store/rebuild_copy*.rs` (store-chokepoint move) — see
`src/store/README.md`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `copy.rs` | `COPIED_TABLES` | Declares `COPIED_TABLES`/`RECONSTRUCTABLE_TABLES`, the live-table allowlist pair the coverage test reads, and `copy_preserved_tables_from_old`, a thin delegate that runs the allowlist debug-assert then calls `crate::store::rebuild_copy::copy_preserved_tables_from_old` |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/api/rebuild.rs`
(`pub mod <name>;`) and callers import concrete paths.
