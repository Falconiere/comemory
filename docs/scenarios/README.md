# CLI scenario catalog

This directory is the **human-readable test plan** for the `comemory` CLI
and its `/api/v1` HTTP twin.
`docs/cli-reference.md` is generated from `--help` and lists flags. These
files say, for every command, **which combinations of flags we actually
run**, what setup they need, and which integration test covers them.

Every file is named after the clap subcommand (`save.md`, `search-code.md`,
…). `tests/cli_scenario_catalog.rs` walks the built `Cli::command()` and
fails if a subcommand has no file here, if a file here names no subcommand,
if a long flag / visible alias / positional is missing from its file, if a
flag has no scenario section that both names it and cites a test, if a
backticked `tests/…rs::fn` citation does not resolve, if a command's
`**HTTP:**` line disagrees with the live `GET /api/v1/commands`, or if a
journey file is missing from the tables below.

## How to read a scenario

Each command file has:

1. **What it does** — one paragraph.
2. **Positionals / flags** — every clap argument, including aliases
   (`--limit` = `--k` where that alias exists).
3. **Scenarios** — numbered `\<cmd\>-NN` cases. Each one is a real
   invocation against a throwaway `COMEMORY_DATA_DIR` (or `--data-dir`),
   never the developer's `~/.comemory`. A scenario names:
   - the flags it exercises
   - the setup (save / index-code / fixture repo)
   - the exact command
   - the expected TTY or `--json` outcome
   - the Rust test that drives it (`tests/cli__*.rs` or
     `tests/cli_scenario_*.rs`)
4. **HTTP** — the `/api/v1` route(s) the command maps to, straight from
   `GET /api/v1/commands`, and the `tests/serve_scenario_*.rs` journey that
   drives them. Server-wide contracts (token, envelope, read-only, confirm,
   jobs) are in [globals.md](globals.md#http-twins).

Global flags `--json` and `--data-dir` are documented once in
[globals.md](globals.md) and apply to every command.

## Cross-command journeys

Per-command files do not replace the multi-command journeys:

| Journey | File | Commands chained |
| --- | --- | --- |
| Getting started | `tests/cli_scenario_getting_started.rs` | doctor, save, search, index-code, search-code, context, edges, stats, repos, show |
| Memory lifecycle | `tests/cli_scenario_memory_lifecycle.rs` | save, list, show, search, delete, doctor |
| Code index | `tests/cli_scenario_code.rs` | index-code, search-code, feedback, graph, ast |
| Documents | `tests/cli_scenario_documents.rs` | index, sources, find, unindex |
| Learning loop | `tests/cli_scenario_learning.rs` | save, search, feedback, mine, eval, tune, bandit |
| Maintenance | `tests/cli_scenario_maintenance.rs` | save, consolidate, prune, rebuild, gc |
| Hooks | `tests/cli_scenario_hooks.rs` | install-hooks, hooks, index-code, search-code |
| BYO-vector | `tests/cli_scenario_vectors.rs` | index-code --extract, ingest-code, search-code --vector-stdin, save --vector-stdin |
| Globals | `tests/cli_scenario_globals.rs` | `--data-dir` vs env, `--json` placement, usage exits |

Each journey has an HTTP twin over a real `comemory serve`, sharing
`tests/common/serve_bin.rs` (spawn, token, envelope unwrap, job polling):

| Journey | HTTP twin | Routes chained |
| --- | --- | --- |
| Getting started | `tests/serve_scenario_getting_started.rs` | doctor, memories, memories/search, code/index (job), code/search, context, edges, stats, repos, memories/{id} |
| Memory lifecycle | `tests/serve_scenario_memory_lifecycle.rs` | memories (POST/GET/DELETE + confirm gate), memories/search, trash, doctor |
| Code index | `tests/serve_scenario_code.rs` | code/index (job), code/search, feedback, graph, repos, code/ast |
| Documents | `tests/serve_scenario_documents.rs` | sources (job/GET/DELETE), find |
| Learning loop | `tests/serve_scenario_learning.rs` | memories, memories/search, feedback, mine, eval (job), tune (job), bandit (job) |
| Maintenance | `tests/serve_scenario_maintenance.rs` | consolidate, prune (GET/POST), memories, rebuild (job), gc |
| Hooks | `tests/serve_scenario_hooks.rs` | hooks/install, hooks, hooks/{name}, code/index (job), code/search |
| BYO-vector | `tests/serve_scenario_vectors.rs` | code/ingest (job), code/search (vector), memories (vector), memories/search (vector) |

## Command index

| Command | Scenario file |
| --- | --- |
| *(globals)* | [globals.md](globals.md) |
| `save` | [save.md](save.md) |
| `search` | [search.md](search.md) |
| `search-code` | [search-code.md](search-code.md) |
| `list` | [list.md](list.md) |
| `delete` | [delete.md](delete.md) |
| `show` | [show.md](show.md) |
| `feedback` | [feedback.md](feedback.md) |
| `eval` | [eval.md](eval.md) |
| `mine` | [mine.md](mine.md) |
| `tune` | [tune.md](tune.md) |
| `bandit` | [bandit.md](bandit.md) |
| `doctor` | [doctor.md](doctor.md) |
| `stats` | [stats.md](stats.md) |
| `repos` | [repos.md](repos.md) |
| `index-code` | [index-code.md](index-code.md) |
| `ingest-code` | [ingest-code.md](ingest-code.md) |
| `index` | [index.md](index.md) |
| `sources` | [sources.md](sources.md) |
| `unindex` | [unindex.md](unindex.md) |
| `find` | [find.md](find.md) |
| `context` | [context.md](context.md) |
| `edges` | [edges.md](edges.md) |
| `graph` | [graph.md](graph.md) |
| `ast` | [ast.md](ast.md) |
| `hooks` | [hooks.md](hooks.md) |
| `install-hooks` | [install-hooks.md](install-hooks.md) |
| `prune` | [prune.md](prune.md) |
| `consolidate` | [consolidate.md](consolidate.md) |
| `rebuild` | [rebuild.md](rebuild.md) |
| `gc` | [gc.md](gc.md) |
| `serve` | [serve.md](serve.md) |
| `completions` | [completions.md](completions.md) |

## Running the catalog

```bash
# inventory: every clap command has a scenario file, every flag is named
cargo nextest run --all-features -E 'binary(cli_scenario_catalog)'

# journeys, CLI and HTTP
cargo nextest run --all-features -E 'binary(/cli_scenario/) | binary(/serve_scenario/)'

# per-command contracts
cargo nextest run --all-features -E 'binary(/cli__/)'
```
