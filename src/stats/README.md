# stats/

**What belongs here:** usage/feedback tables inside `comemory.db` — per-memory
feedback counters, per-symbol code feedback counters, the raw retrieval log,
and the single home of the `retrieval_log.source` / `feedback_events`
vocabularies so writers and readers cannot drift on literal strings.

**What does NOT belong here:** interpreting those counters into a ranking
prior. `stats/` only stores and returns raw counters; `retrieval::score` and
`retrieval::code_prior` turn them into bounded multipliers. It also does NOT
own the SQL itself any more — the store-layer chokepoint work moved every
`feedback`/`code_feedback`/`index_failures` query into
`store::{feedback,code_feedback,index_failures}`; this folder keeps the
provenance vocabulary, the chunk-to-parent identity resolution rule, and
every transaction boundary (`StatsDb::conn_mut().transaction()`), and calls
the store helpers from inside it.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `code_feedback.rs` | `record_code_with_provenance` | Per-symbol code feedback counters: `used` and `irrelevant`. Owns the rowid→identity resolution (chunk-to-parent walk) and the transaction; the SQL is `store::code_feedback` |
| `feedback.rs` | `PROV_AUTO_COACTIVATION` | Per-memory feedback counters: `used` and `irrelevant`, plus the provenance vocabulary — `PROV_MANUAL`, `PROV_IMPLICIT`, the two `PROV_AUTO_*` tags, and `Source` (the routes' `explicit\|implicit` request words with `parse` and the one `provenance()` mapping onto the stored value). Owns the query-id contract and the transaction; the SQL is `store::feedback` |
| `sqlite.rs` | `StatsDb` | SQLite-backed stats store, opened via the shared connection helper; its `index_failures` methods delegate to `store::index_failures` |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/stats.rs` (`pub mod
<name>;`) and callers import concrete paths.
