# `comemory bandit`

Thompson-sample one arm of the `[tune]` grid, confirm with offline eval.
`--apply` writes `config.toml` only when the sample beats baseline.
Ignores `tune.samples` — arms are the full cartesian product unless the
file grid is shrunk.

**Runnable tests:** `tests/cli__bandit.rs`, `tests/cli_scenario_learning.rs`

**HTTP:** `POST /api/v1/bandit` — covered by `tests/serve_scenario_learning.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--golden` | unset | YAML golden file |
| `--golden-only` | off | Ignore harvested feedback. Requires `--golden` |
| `--k` | `3` | Recall@k used while scoring |
| `--apply` | off | Write knobs when the sampled arm beats baseline |

## Scenarios

### bandit-01 Thin golden set

- **Flags:** `--golden` `--golden-only`
- **Setup:** 3 golden pairs (below the floor of 10)
- **Command:** `comemory bandit --golden thin.yaml --golden-only`
- **Expect:** exit 69; stderr mentions golden pairs.
- **Covered by:** `tests/cli__bandit.rs::bandit_thin_golden_set_exits_unavailable`

### bandit-02 Happy path on a one-arm grid

- **Flags:** `--golden` `--golden-only` `--k`
- **Setup:** 10 golden pairs; `config.toml` with a single-value `[tune]` grid
  so the cartesian product is 1 (the default 729-arm grid is too slow here)
- **Command:** `comemory bandit --golden golden.yaml --golden-only --json`
- **Expect:** `report.golden_pairs == 10`; `report.ranked` non-empty;
  `report.proposed` present. `--apply` is optional and only writes on a
  strict beat.
- **Covered by:** `tests/cli_scenario_learning.rs`

### bandit-03 Apply

- **Flags:** `--apply`
- **Setup:** golden set at the floor; `[bandit] enabled` toggled in `config.toml`
- **Command:** `comemory bandit --golden golden.yaml --golden-only --apply`
- **Expect:** without `--apply` nothing is written to `config.toml` (arms are
  still upserted); with `--apply` the write is refused while
  `[bandit] enabled = false`. A winning apply depends on the sampled arm
  beating baseline, so only the two deterministic contracts are pinned.
- **Covered by:** `src/api/tests/bandit.rs::run_without_apply_reports_and_never_writes_config`,
  `src/api/tests/bandit.rs::run_apply_refused_when_bandit_disabled_in_config`
