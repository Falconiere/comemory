# `comemory repos`

Indexed code repositories and index freshness (`fresh` \| `stale` \|
`unknown`). Git failure degrades to `unknown` and never errors.

**Runnable tests:** `tests/cli__repos.rs`, `tests/cli_scenario_getting_started.rs`

**HTTP:** `GET /api/v1/repos` — covered by `tests/serve_scenario_getting_started.rs`, `tests/serve_scenario_code.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--repo` | unset | Restrict the listing to one label |

## Scenarios

### repos-01 Two indexed repos

- **Flags:** `--json`
- **Setup:** two real git repos, both `index-code`'d
- **Command:** `comemory repos --json`
- **Expect:** two rows; `files` / `symbols` > 0; `status=fresh`.
- **Covered by:** `tests/cli__repos.rs::ac3_two_indexed_repos_are_both_listed_fresh_with_nonzero_counts`

### repos-02 Stale after a new commit

- **Flags:** `--json`
- **Setup:** commit a new file in one repo without reindexing
- **Command:** `comemory repos --json`
- **Expect:** that row flips to `stale`; the other stays `fresh`.
- **Covered by:** `tests/cli__repos.rs::ac4_a_new_commit_without_reindexing_flips_only_that_repo_to_stale`

### repos-03 Filter

- **Flags:** `--repo`
- **Command:** `comemory repos --repo demo --json`
- **Expect:** only the `demo` row.
- **Covered by:** `tests/cli__repos.rs` (row lookup), `tests/cli_scenario_getting_started.rs`
