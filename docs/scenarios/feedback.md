# `comemory feedback`

Record used vs irrelevant verdicts against a `query_id` from
`search` / `search-code` / `find` / `context`. Writes `feedback_events`
and updates Beta counters.

**Runnable tests:** `tests/cli__feedback.rs`, `tests/cli__feedback_2.rs`,
`tests/cli_scenario_code.rs`, `tests/cli_scenario_learning.rs`

**HTTP:** `POST /api/v1/feedback` — covered by `tests/serve_scenario_code.rs`, `tests/serve_scenario_learning.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`<QUERY_ID>` — `q-<yyyymmdd>-<8hex>` from a prior search envelope.

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--used` | empty | Comma-separated memory ids that helped |
| `--irrelevant` | empty | Comma-separated memory ids that did not |
| `--used-code` | empty | Comma-separated code-symbol ids (positive integers) that helped |
| `--irrelevant-code` | empty | Comma-separated code-symbol ids that did not |

## Scenarios

### feedback-01 Used memory

- **Flags:** `--used`
- **Setup:** `search --json` on a saved memory, take `query_id` and hit id
- **Command:** `comemory feedback <query_id> --used <memory_id> --json`
- **Expect:** exit 0; a later search reorders toward the used id.
- **Covered by:** `tests/cli__feedback.rs`, `tests/cli_rank_smoke.rs::irrelevant_feedback_reorders_results`

### feedback-02 Irrelevant memory

- **Flags:** `--irrelevant`
- **Setup:** as above
- **Command:** `comemory feedback <query_id> --irrelevant <memory_id>`
- **Expect:** that memory drops in subsequent ranking.
- **Covered by:** `tests/cli_rank_smoke.rs::irrelevant_feedback_reorders_results`

### feedback-03 Used code

- **Flags:** `--used-code`
- **Setup:** `search-code --json`, take `query_id` and `symbol_id`
- **Command:** `comemory feedback <query_id> --used-code <symbol_id> --json`
- **Expect:** exit 0; access counters on the symbol bump.
- **Covered by:** `tests/cli_scenario_code.rs`, `scripts/e2e.sh`

### feedback-04 Mixed memory and code

- **Flags:** `--used` `--used-code`
- **Command:** `comemory feedback <qid> --used a1b2c3d4 --used-code 12`
- **Expect:** both verdicts recorded in one call.
- **Covered by:** `tests/cli__feedback.rs`

### feedback-05 Irrelevant code

- **Flags:** `--irrelevant-code`
- **Command:** `comemory feedback <qid> --irrelevant-code <symbol_id>`
- **Expect:** exit 0; code-feedback counters update.
- **Covered by:** `tests/cli__feedback_2.rs`
