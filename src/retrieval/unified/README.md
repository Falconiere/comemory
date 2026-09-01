# `src/retrieval/unified/`

Unified retrieval across the three domains — the `comemory find` core. The
parent `src/retrieval/unified.rs` owns the entry point and the pagination
rule; this folder owns everything about combining the legs.

`search` answers the memory domain and `search-code` the code domain, each
with its own hit shape. Neither can produce a single ranked list mixing the
two, which is what the console's Search screen renders.

| File | Responsibility |
| --- | --- |
| `fuse_domains.rs` | `UnifiedHit` (the domain-tagged hit), `HitParts` (an untagged enum carrying each domain's own score breakdown verbatim), and `fuse` — weighted N-ary RRF over the three legs' already-reranked orders, via the crate's existing `fuse::rrf_multi_weighted`. Ids are namespaced `<domain>:<id>` before fusion so a `code_symbols` rowid cannot collide with a document ordinal. |

Two invariants worth keeping:

- **Each leg fuses in its own reranked order.** RRF ranks by position, so
  preserving a leg's internal order is what makes a single-domain `find`
  order-identical to that domain's dedicated command.
- **One `pool_size` for every leg.** RRF is prefix-stable, so growing all
  legs by the same rule appends tail candidates without reordering the head.
  Divergent pools would let a deeper page reorder a shallower one.

`cfg.retrieval.document_leg_weight` is consumed here. It has been declared
and validated since the document domain landed and was read by nothing until
this module existed.
