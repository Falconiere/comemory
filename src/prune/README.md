# prune/

**What belongs here:** read-only, side-effect-free candidate *detection* —
each submodule exposes a `detect` function returning the ids or paths eligible
for removal (orphaned trash entries, low-value memories, ghost code
references).

**What does NOT belong here:** the actual mutation. Turning a candidate list
into a soft-delete or filesystem purge is the CLI surface's job
(`cli::prune`, `cli::gc`, calling `memory::MemoryStore::delete`); nothing in
`prune/` writes to the store.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `low_value.rs` | `detect` | Low-value memory detection: activation, Beta feedback, quality, graph degree. The two SQL scans (`signal_rule`, `superseded_rule`) delegate to `store::prune_signals`; the ACT-R/Beta scoring and the grace-day cutoff computation stay here |
| `orphans.rs` | `detect` | Orphan detection: trash entries whose live counterpart is gone |
| `stale_code.rs` | `detect` | Ghost code-reference detection: pinned `references_symbol` edges gone stale. The `code_ref` scan and the `code_symbols` resolve check delegate to `store::code_ref::for_rel_live` / `store::code_row::symbol_row_exists`; the `RefStatusCache` classification stays here |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/prune.rs` (`pub mod
<name>;`) and callers import concrete paths.
