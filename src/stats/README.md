# stats/

**What belongs here:** usage/feedback tables inside `comemory.db` — per-memory
feedback counters, per-symbol code feedback counters, the raw retrieval log,
and the single home of the `retrieval_log.source` / `feedback_events`
vocabularies so writers and readers cannot drift on literal strings.

**What does NOT belong here:** interpreting those counters into a ranking
prior. `stats/` only stores and returns raw counters; `retrieval::score` and
`retrieval::code_prior` turn them into bounded multipliers.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `code_feedback.rs` | `record_code_with_provenance` | Per-symbol code feedback counters: `used` and `irrelevant` |
| `feedback.rs` | `PROV_AUTO_COACTIVATION` | Per-memory feedback counters: `used` and `irrelevant`, plus provenance constants |
| `sqlite.rs` | `StatsDb` | SQLite-backed stats store, opened via the shared connection helper |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/stats.rs` (`pub mod
<name>;`) and callers import concrete paths.
