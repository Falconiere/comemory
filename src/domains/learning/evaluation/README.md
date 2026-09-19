# `src/domains/learning/evaluation/`

**What belongs here:** the evaluation and ranking-search algorithms of the
learning capability — golden sets (hand-written + feedback-harvested),
recall@k/MRR metrics, the eval runner that drives the real pipeline,
reformulation mining into `query_expansions`, the deterministic, sampled
and bandit searches over the ranking blend knobs, and — since #208 — the
domain-aware offline benchmark: the `benchmark_*` and `candidate_*` files plus
`judgment` and `run_environment`.

The candidate observation contract those files publish is the one #209
persists, #210 exports, #211 tokenizes and #214 trains on. It is specified in
`docs/designs/2026-09-18-domain-aware-retrieval-benchmark.md`, § Candidate
observation contract; a change to any field's presence or meaning is a change
to `candidate_observation::OBSERVATION_VERSION`.

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
| `benchmark_arm.rs` | `ArmScores` | Benchmark arms: the reserved deterministic baseline plus the scored arms a scorer supplies as a JSON file, and the total ordering rule that lets a partially scored arm degrade toward the baseline |
| `benchmark_arm_report.rs` | `ArmReport` | One arm's aggregate and per-corpus metrics, its latency distribution, its paired nDCG@k delta, and the verdict that delta earns against the set's budgets — read off the interval, never the point estimate |
| `benchmark_metrics.rs` | `TaskMetrics` | Candidate-pool recall measured apart from recall@k, MRR and nDCG@k, over judgments rather than candidates, plus the paired bootstrap |
| `benchmark_observe.rs` | `observe` | Turn a fused candidate pool plus its facts into the published `CandidateObservation` list and the complete `EffectiveFilters` record |
| `benchmark_report.rs` | `BenchmarkReport` | The replayable artifact and its assembly; `summary()` is the same shape with candidates dropped, which `--json` prints |
| `benchmark_runner.rs` | `TaskCapture` | Capture one task's pool through `retrieval::unified::run_legs`, match its judgments, and record whether the pool prefix equals the production page |
| `benchmark_set.rs` | `BenchmarkSet` | The versioned reviewed set: tasks, per-domain filters, the pinned `ranking` knobs, the vector scenario and the budgets, with every silently-inert shape refused at load |
| `candidate_facts.rs` | `CandidateFacts` | Identity, content version and bounded text read off the leg rows before fusion discards them |
| `candidate_identity.rs` | `CandidateIdentity` | Domain-qualified stable identity and the reference-string codec — the opaque token the reranker process protocol passes around |
| `dataset_build.rs` | `assemble` | Bucket split rows by `(split, label class)` FIRST, then cap reviewed negatives inside one bucket — the ordering that makes "splits before negative mining" mechanical |
| `dataset_dedup.rs` | `collapse_duplicates` | Duplicate observations, verdicts naming a stale version or nothing the pool returned, and contradictory verdicts, each resolved deterministically and counted |
| `dataset_files.rs` | `write_buckets` | The closed owned-name set, the sweep that clears it before writing, the JSONL serialization and the manifest write |
| `dataset_manifest.rs` | `DatasetManifest` | What was exported and what was refused, plus `snapshot_digest` and `dataset_id` — the two digests that make a repeated export provable |
| `dataset_record.rs` | `DatasetRecord` | The exported JSONL line and `RECORD_VERSION`; no locator field, and no label invented for a candidate nobody judged |
| `dataset_rows.rs` | `build` | The provenance filter, the contract-version refusal, the per-observation context and the fixed order the pipeline runs in |
| `dataset_select.rs` | `resolve` | Which candidates and verdicts survive — every drop has a name and a counter — and the rows they become |
| `dataset_split.rs` | `plan` | Grouped, seeded split assignment: union-find over the query/content graph, the reserved-repository and reserved-time-slice overrides, then a stable hash |
| `candidate_observation.rs` | `CandidateObservation` | The candidate observation contract: one candidate, its bounded text, and the per-query envelope carrying the filters, the retrieval/corpus version and the reference time |
| `golden.rs` | `GoldenPair` | Golden-set model: hand-written YAML pairs merged with feedback-harvested pairs; the harvest join delegates to `store::feedback::used_events_for_golden`, pinned to `manual` provenance |
| `judgment.rs` | `Judgment` | A reviewed relevance judgment, its validated target key, and the match/stale/no outcome against an observed identity |
| `metrics.rs` | `recall_at_k` | Pure retrieval-quality metrics: recall@k, MRR, and percentile bootstrap CI |
| `mine.rs` | `MinedMapping` | Reformulation mining: distill failed→fixed query pairs into expansions; the `retrieval_log`/`feedback_events` scans delegate to `store::retrieval_log`/`store::feedback`, and `apply`'s replace-all delegates to `store::query_expansions` |
| `run_environment.rs` | `RunEnvironment` | Hardware, memory use, the run-scoped working set, and the pinned corpus/index snapshot behind `RetrievalVersion` |
| `runner.rs` | `QueryResult` | Drive the real retrieval pipeline over a golden set and score it |
| `task_filters.rs` | `TaskFilters` | The per-domain filters one benchmark task applies, and the rule that refuses one whose only leg the task does not run |
| `tune.rs` | `MIN_GOLDEN_PAIRS` | Deterministic/sampled search over blend weights, scored by eval MRR |
| `tune_sample.rs` | `pool_sizes` | Seeded uniform sampling over the `[tune]` grid pools |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from
`src/domains/learning/evaluation.rs` (`pub mod <name>;`) and callers import
concrete paths.
