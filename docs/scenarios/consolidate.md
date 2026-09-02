# `comemory consolidate`

Advisory near-duplicate cluster report over live memories (SimHash +
union-find). Read-only — the merge stays a human `save --supersedes`.

**Runnable tests:** `tests/cli__consolidate.rs`, `tests/cli_scenario_maintenance.rs`

**HTTP:** `GET /api/v1/consolidate` — covered by `tests/serve_scenario_maintenance.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--radius` | config `rank.near_dup_hamming` (8) | Hamming radius. 0..=64; 65 is usage |
| `--repo` | unset | Restrict the scan to one repo |
| `--all` | off | Include clusters already resolved by live supersede edges |
| `--k` / `--limit` | config `retrieval.top_k` | Page size over clusters |
| `--offset` | `0` | Skip this many clusters |

## Scenarios

### consolidate-01 Near-dup cluster

- **Flags:** `--json`
- **Setup:** two bodies with Hamming distance 5 (within radius 8)
- **Command:** `comemory consolidate --json`
- **Expect:** `clusters.total ≥ 1`; keeper first (`hamming_to_keeper=0`).
  Two consecutive runs are byte-identical; no writes.
- **Covered by:** `tests/cli__consolidate.rs`, `tests/cli_scenario_maintenance.rs`

### consolidate-02 Radius zero

- **Flags:** `--radius`
- **Command:** `comemory consolidate --radius 0 --json`
- **Expect:** one-word-apart bodies do not cluster. `--radius 65` is usage.
- **Covered by:** `tests/cli__consolidate.rs::radius_zero_clusters_only_identical_bodies`

### consolidate-03 Repo and all

- **Flags:** `--repo` `--all`
- **Command:** `comemory consolidate --repo demo --all --json`
- **Expect:** only that repo; resolved supersede clusters are included when
  `--all` is set.
- **Covered by:** `tests/cli__consolidate.rs`

### consolidate-04 Pagination

- **Flags:** `--k` `--limit` `--offset`
- **Command:** `comemory consolidate --limit 1 --offset 0 --json`
- **Expect:** `clusters` is a `Page`.
- **Covered by:** `tests/cli__consolidate.rs`
