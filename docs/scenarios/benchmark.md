# `comemory benchmark`

Score a reviewed, versioned benchmark set over the real retrieval legs —
memory, code, documents and mixed — and emit a replayable artifact of every
candidate observation. Candidate-pool recall is reported separately from
recall@k, MRR and nDCG@k, so a result retrieval never produced is
distinguishable from one it ranked below the cut.

The set pins its own retrieval configuration (`ranking:`) and its own budgets,
before anything is scored. `ranking.decay: 0.0` freezes ACT-R activation, which
disabling access tracking alone does not do. The run writes no `retrieval_log`
row and bumps no access counter.

Distinct from [`eval`](eval.md), which scores memory-only lexical retrieval
against a memory-id golden file. That command, its format and its history are
unchanged.

**Runnable tests:** `tests/cli__benchmark.rs`

**HTTP:** none — CLI only. A run writes its artifact to an operator-named
filesystem path, which a server must never do on request.

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--set` | required | Path to the reviewed benchmark set YAML (version 1) |
| `--report` | unset | Write the full artifact JSON — every candidate observation included — to this path. Created or truncated |
| `--scores` | unset, repeatable | Add one arm from a scorer's JSON scores file (`{arm, scorer_version?, scores: {task: {candidate_ref: score}}}`) |
| `--k` | the set's `defaults.k` | Override the recall@k / nDCG@k cut. Must be at least 1 |

`--json` prints the summary: the artifact with every candidate list emptied, so
it stays pipeable. `--report` writes the same shape with the candidates intact.

## Scenarios

### benchmark-01 Full observation artifact over every domain

- **Flags:** `--set` `--report` `--json`
- **Setup:** three memories saved through the binary, a real git repo indexed
  with `index-code --repo demo`, a markdown tree indexed with `index`
- **Command:** `comemory --json benchmark --set benchmark-mixed-v1.yaml --report artifact.json`
- **Expect:** `artifact_version`, `observation_version`, the corpus snapshot
  with each repo's `last_head`, and — in the written artifact — candidates from
  all three corpora, each with a parseable `candidate_ref` and a text digest.
  The `--json` summary carries every field but no candidates.
- **Covered by:** `tests/cli__benchmark.rs::benchmark_emits_a_full_observation_artifact_over_every_domain`

### benchmark-02 Pool recall apart from the page, and judgment coverage

- **Flags:** `--set` `--report`
- **Setup:** the same mixed corpus
- **Command:** `comemory --json benchmark --set benchmark-mixed-v1.yaml --report artifact.json`
- **Expect:** `pool_recall >= recall_at_k`; a per-domain block for each corpus;
  `unjudged_in_page > 0` and `judged_page_fraction < 1.0` stated outright; each
  task's `filters` naming only the legs it runs.
- **Covered by:** `tests/cli__benchmark.rs::benchmark_measures_the_pool_separately_from_the_page_and_reports_coverage`

### benchmark-03 Two arms over one candidate snapshot

- **Flags:** `--set` `--scores` `--report`
- **Setup:** run once with `--report`, then build a scores file from that
  artifact that inverts every pool position
- **Command:** `comemory --json benchmark --set benchmark-mixed-v1.yaml --scores inverted.json --report compared.json`
- **Expect:** two arms; the scored arm carries its `scorer_version`,
  `scored_fraction` and a `paired_vs_baseline` nDCG@k delta with a confidence
  interval; an inverted ranking never improves; `pool_recall` is identical
  across arms, since an arm reorders the pool and cannot change what is in it.
- **Covered by:** `tests/cli__benchmark.rs::benchmark_compares_two_arms_over_one_candidate_snapshot`

### benchmark-04 No telemetry, and a repeatable score

- **Flags:** `--set`
- **Setup:** the mixed corpus; run the benchmark twice
- **Command:** `comemory --json benchmark --set benchmark-mixed-v1.yaml`
- **Expect:** `retrieval_log` stays empty, `memories.access_count` and
  `code_symbols.access_count` stay at zero, `comemory eval --history` is
  unchanged, and a decay-frozen run reproduces its metrics exactly.
- **Covered by:** `tests/cli__benchmark.rs::benchmark_writes_no_telemetry_and_leaves_eval_untouched`,
  `tests/cli__benchmark.rs::a_repeated_run_over_an_unchanged_corpus_scores_identically`

### benchmark-05 BYO-vector scenario

- **Flags:** `--set`
- **Setup:** a memory saved with `--vector`, and a set declaring
  `vectors: {model, dim: 1024}` with a 1024-value vector on its task
- **Command:** `comemory --json benchmark --set vector-set.yaml`
- **Expect:** the task's `filters.vector` is `supplied` with the model, the
  dimension and a digest; a lexical set over the same corpus records
  `{"kind": "lexical"}`. A declared model with a task missing its vector is a
  load error naming the task, exit `78`.
- **Covered by:** `tests/cli__benchmark.rs::a_supplied_vector_scenario_records_its_model_identity_and_digest`,
  `tests/cli__benchmark.rs::a_vector_set_missing_a_task_vector_is_refused_naming_the_task`

### benchmark-07 Cut override

- **Flags:** `--set` `--k`
- **Setup:** the mixed corpus
- **Command:** `comemory --json benchmark --set benchmark-mixed-v1.yaml --k 1`
- **Expect:** the report's `k` and every observation's `page_limit` follow the
  override rather than the set's `defaults.k`; `pool_recall` is unchanged,
  because the cut moves the page and never the pool; a narrower cut cannot
  raise `recall_at_k`. `--k 0` is a clap usage error, exit `2`.
- **Covered by:** `tests/cli__benchmark.rs::a_k_override_replaces_the_set_default_cut`

### benchmark-06 Malformed set, and the TTY summary

- **Flags:** `--set`
- **Setup:** a set file with an unsupported `version`, and a missing path
- **Command:** `comemory benchmark --set bad.yaml`
- **Expect:** exit `78` (`EX_CONFIG`) naming the file. A valid run's TTY view
  states the set name and version, whether decay is frozen, each arm's verdict,
  its pool recall and whether it is within the latency budget.
- **Covered by:** `tests/cli__benchmark.rs::a_malformed_set_exits_with_the_config_code_and_names_the_file`,
  `tests/cli__benchmark.rs::the_tty_summary_states_the_verdict_the_budget_and_the_decay_pin`
