# `comemory tune`

Grid-search blend knobs against a golden set. Report-only unless `--apply`
and the winner strictly beats baseline. File-only `[tune]` grids in
`config.toml` define the search space.

**Runnable tests:** `tests/cli__tune.rs`, `tests/cli_scenario_learning.rs`

**HTTP:** `POST /api/v1/tune` — covered by `tests/serve_scenario_learning.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--golden` | unset | YAML golden file |
| `--golden-only` | off | Ignore harvested feedback. Requires `--golden` |
| `--k` | `3` | Recall@k used while scoring |
| `--apply` | off | Write the winner into `config.toml` only if it beats baseline |
| `--seed` | derived | Pins the sampled candidate draw |

## Scenarios

### tune-01 Sampled grid is deterministic

- **Flags:** `--golden` `--golden-only`
- **Setup:** 10-topic corpus + golden YAML
- **Command:** `comemory tune --golden golden.yaml --golden-only --json`
- **Expect:** `report.ranked` length follows `tune.samples` (64 at defaults).
  Two runs without `--seed` are byte-identical. `--seed` pins the draw.
- **Covered by:** `tests/cli__tune.rs::tune_json_ranks_the_sampled_candidates_and_is_deterministic`

### tune-02 Apply

- **Flags:** `--apply`
- **Command:** `comemory tune --golden golden.yaml --golden-only --apply --json`
- **Expect:** `applied=true` only when the winner beats baseline; then
  `config.toml` contains `[retrieval]` / `[rank]` knobs. Tie → no write.
- **Covered by:** `tests/cli__tune.rs::tune_apply_writes_config_only_when_winner_beats_baseline`

### tune-03 One-arm grid (fast)

- **Flags:** `--golden` `--golden-only`
- **Setup:** `config.toml` with single-value `[tune]` grids
- **Command:** `comemory tune --golden golden.yaml --golden-only --json`
- **Expect:** `report.ranked` has length 1.
- **Covered by:** `tests/cli_scenario_learning.rs`

### tune-04 Thin golden set

- **Flags:** `--golden` `--golden-only`
- **Setup:** fewer than `tune.min_golden` pairs (default 10)
- **Command:** `comemory tune --golden thin.yaml --golden-only`
- **Expect:** exit 69; stderr mentions golden pairs.
- **Covered by:** `tests/cli__tune.rs`

### tune-05 Page size and seed

- **Flags:** `--k` `--seed`
- **Setup:** golden set at the floor
- **Command:** `comemory tune --golden golden.yaml --golden-only --k 1 --seed 42 --json`
- **Expect:** `--seed` pins the sampled candidate draw (two runs agree);
  `--k` is the recall@k cutoff the report scores at.
- **Covered by:** `tests/cli__tune.rs::tune_seed_flag_pins_the_candidate_draw`,
  `tests/cli__tune.rs::tune_apply_writes_the_graph_knobs_and_keeps_unrelated_keys`
