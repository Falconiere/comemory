# comemory documentation

`comemory` fuses engram-style developer memory, semantic code search, and
ast-grep AST patterns into one local SQLite-backed CLI. These docs are
organized by what you're trying to do.

## Start here

- **[Getting started](getting-started.md)** — install, save your first memory,
  search, and index code in a few minutes. Read this first.

## How-to guides

Task-oriented recipes for a specific job:

- **[Bring your own vectors](guides/byo-vectors.md)** — embed memories and code
  with your own model via `--vector` / `--vector-stdin` (dims 1024 / 768).
- **[Keep the code index fresh](guides/auto-reindex.md)** — `lazy` (default),
  `hook`, and `off` auto-reindex modes and how the wired lazy trigger works.
- **[Measure and tune ranking](guides/ranking-and-eval.md)** — the
  `eval → mine → tune` learning loop and the ranking knobs.
- **[Serve the web viewer](guides/serve-web.md)** — `comemory serve` and the
  `/api/graph` endpoint.
- **[Prune, rebuild, and gc](guides/prune-and-gc.md)** — maintenance: trim
  low-value memories, rebuild the DB from markdown, garbage-collect logs.

## Reference

Look-it-up material:

- **[CLI reference](cli-reference.md)** — every subcommand and flag, with the
  `--json` pagination envelope (generated from `--help`).
- **[Configuration](configuration.md)** — every environment variable, the
  config-file-only knobs, and the pagination envelope shape.
- **[Release process](release.md)** — how releases are cut and published.
- **[Build performance](build-perf.md)** — build-time notes.

## Explanation

Understanding-oriented background:

- **[Architecture](architecture.md)** — the design: storage layout, the
  retrieval pipeline, the edge graph, auto-reinforcement, and pagination.
