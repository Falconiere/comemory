# `comemory repos`

Indexed code repositories and index freshness (`fresh` \| `stale` \|
`unknown` \| `shared`). Git failure degrades to `unknown` and never errors.

A repo a peer replicated is listed too. Its row carries `shared_head` (the
head that peer indexed) and `shared_files` (the size of the manifest it sent),
alongside `last_head` and the counters for what THIS machine indexed — one row
per repository, whichever side it is known from. A repo known only from a peer
has no working tree to probe, so it reports `status: "shared"` with no
`root_path`, no `last_head` and zero counters; `search-code` returns nothing
for it, because a replicated generation carries no source text.

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

### repos-04 A repo a peer shared

- **Flags:** `--json`
- **Setup:** a data dir with no checkout of the repo, holding an active
  replicated generation for it
- **Command:** `comemory repos --json`
- **Expect:** one row with `status=shared`, `shared_head` set and
  `last_head` absent; after indexing a matching checkout, still ONE row,
  now carrying both revisions.
- **Covered by:**
  `src/domains/code/tests/remote_view.rs::a_machine_with_no_checkout_answers_repos_and_the_graph_but_not_search`,
  `src/domains/code/tests/remote_view.rs::connecting_a_checkout_shows_one_repo_with_both_revisions`
