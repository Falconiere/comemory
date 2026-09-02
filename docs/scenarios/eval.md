# `comemory eval`

Score retrieval against a golden set (YAML file and/or harvested
feedback). Prints recall@k, MRR, optional CIs. `--history` lists past
eval/tune/bandit runs instead of scoring.

**Runnable tests:** `tests/cli__eval.rs`, `tests/cli__eval_history.rs`,
`tests/cli_scenario_learning.rs`

**HTTP:** `POST /api/v1/eval`, `POST /api/v1/learning/evals` — covered by `tests/serve_scenario_learning.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--golden` | unset | Path to a YAML golden file (`- query: … / relevant: [ids]`) |
| `--golden-only` | off | Score only the file; ignore harvested feedback. Requires `--golden` |
| `--k` | `3` | Recall@k depth |
| `--history` | off | List past runs (conflicts with golden-set flags) |
| `--limit` | `20` | Max history rows, newest-first. Requires `--history` |

## Scenarios

### eval-01 Harvested feedback

- **Flags:** _(defaults)_
- **Setup:** save + search + `feedback --used`
- **Command:** `comemory eval --json`
- **Expect:** `queries ≥ 1`, `recall_at_k` / `mrr` present. Empty data dir
  exits 69 (`unavailable`) with "no golden pairs".
- **Covered by:** `tests/cli__eval.rs::eval_scores_harvested_feedback_through_real_binary`

### eval-02 Golden file only

- **Flags:** `--golden` `--golden-only` `--k`
- **Setup:** 10 saved topics + a YAML pairing each query to its id
- **Command:** `comemory eval --golden golden.yaml --golden-only --k 3 --json`
- **Expect:** `queries == 10`; CIs present without changing point estimates.
  `--golden-only` without `--golden` is clap usage and must not create the db.
- **Covered by:** `tests/cli__eval.rs`, `tests/cli_scenario_learning.rs`

### eval-03 History

- **Flags:** `--history` `--limit`
- **Setup:** at least one prior `eval` run
- **Command:** `comemory eval --history --json`
- **Expect:** a JSON array, newest first. Fresh data dir → `[]`.
- **Covered by:** `tests/cli__eval_history.rs`, `tests/cli_scenario_learning.rs`
