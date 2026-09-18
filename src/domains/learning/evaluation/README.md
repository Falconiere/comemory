# `src/domains/learning/evaluation/`

**What belongs here:** the evaluation and ranking-search algorithms of the
learning capability — golden sets (hand-written + feedback-harvested),
recall@k/MRR metrics, the eval runner that drives the real pipeline,
reformulation mining into `query_expansions`, and the deterministic, sampled
and bandit searches over the ranking blend knobs.

**What does NOT belong here:** the retrieval pipeline itself, and the command
cores. `runner` calls `retrieval::pipeline` and `retrieval::code_search`
exactly as a CLI caller would; it never reimplements ranking. The
`comemory eval` / `tune` / `mine` / `bandit` middles live one level up as
`learning::{eval, tune, mine, bandit}` — this folder is named `evaluation`
because `eval` is already taken by the core it serves.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `bandit.rs` | `Arm` | Eval-gated Thompson bandit over the `[tune]` discrete grid; the arm-state CRUD (`seed`/`load`/`record_outcome`) delegates to `store::bandit_arms` |
| `bandit_rng.rs` | `SplitMix64` | Deterministic SplitMix64 + Beta/Gamma sampling for the bandit (no `rand`) |
| `golden.rs` | `GoldenPair` | Golden-set model: hand-written YAML pairs merged with feedback-harvested pairs; the harvest join delegates to `store::feedback::used_events_for_golden`, pinned to `manual` provenance |
| `metrics.rs` | `recall_at_k` | Pure retrieval-quality metrics: recall@k, MRR, and percentile bootstrap CI |
| `mine.rs` | `MinedMapping` | Reformulation mining: distill failed→fixed query pairs into expansions; the `retrieval_log`/`feedback_events` scans delegate to `store::retrieval_log`/`store::feedback`, and `apply`'s replace-all delegates to `store::query_expansions` |
| `runner.rs` | `QueryResult` | Drive the real retrieval pipeline over a golden set and score it |
| `tune.rs` | `MIN_GOLDEN_PAIRS` | Deterministic/sampled search over blend weights, scored by eval MRR |
| `tune_sample.rs` | `pool_sizes` | Seeded uniform sampling over the `[tune]` grid pools |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from
`src/domains/learning/evaluation.rs` (`pub mod <name>;`) and callers import
concrete paths.
