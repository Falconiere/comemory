# CLI reference

This page is **generated** by `scripts/regen-cli-docs.sh`. Do not edit by
hand — re-run the script and commit the result. Drift is enforced by
`scripts/cli-docs-check.sh` in the umbrella gate.

## Global options

Every subcommand inherits two global flags:

| Flag | Description |
|---|---|
| `--json` | Emit machine-readable JSON instead of a human TTY view. |
| `--data-dir <DATA_DIR>` | Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable. |

Exit codes follow `sysexits.h`: `0` success, non-zero for usage / I/O /
data errors.

## Top-level help

```
Agentic dev memory + code-aware semantic search

Usage: comemory [OPTIONS] <COMMAND>

Commands:
  save           Save a memory (body via arg, `-`, or stdin)
  search         Search the memory index by natural-language query
  search-code    Search the code index by natural-language or identifier query
  list           List memories with optional repo/kind filters
  delete         Soft-delete a memory by id (moves to `.trash/`)
  feedback       Record per-memory feedback (used vs irrelevant)
  eval           Score retrieval quality against a golden set (recall@k, MRR)
  mine           Mine reformulation pairs from the query log into term-expansion mappings (report only; `--apply` rebuilds `query_expansions`)
  tune           Grid-search blend weights against the golden set (report only; `--apply` writes the winner into config.toml)
  doctor         Report on the data directory and SQLite mirror health
  index-code     Walk a repo, extract symbols, and upsert into the code index
  ingest-code    Read pre-embedded JSONL rows from stdin and ingest them into the code index (`code_symbols` + `code_fts` + `code_vec`)
  ast            Run an ast-grep pattern against a single source file
  graph          Export the file-level code-connection graph (imports + co-change) as JSON, Graphviz DOT, or an interactive HTML page
  serve          Launch the local web viewer + in-browser code editor (loopback HTTP)
  context        Headline lookup: code symbol + memories matching a key
  completions    Emit a shell completion script for `bash`, `zsh`, `fish`, `powershell`, or `elvish`
  prune          Detect (and optionally soft-delete) stale memories
  rebuild        Drop `comemory.db` and repopulate it from the markdown source of truth
  gc             Purge old `memories/.trash/` entries and learning telemetry past retention
  install-hooks  Install git hooks that trigger `comemory index-code` on `post-commit`, `post-merge`, and `post-checkout`
  help           Print this message or the help of the given subcommand(s)

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help
  -V, --version              Print version
```

---

## comemory save

```
Save a memory (body via arg, `-`, or stdin)

Usage: comemory save [OPTIONS] [BODY]

Arguments:
  [BODY]  Memory body. Use `-` (or omit) to read from stdin

Options:
      --json                     Emit machine-readable JSON instead of a human TTY view
      --kind <KIND>              Memory kind: decision|bug|convention|discovery|pattern|note [default: note] [possible values: decision, bug, convention, discovery, pattern, note]
      --data-dir <DATA_DIR>      Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --repo <REPO>              Repo name attached to the memory (free-form string) [default: ""]
      --tags <TAGS>              Comma-separated tag list (e.g. `database,postgres`) [default: ""]
      --author <AUTHOR>          Author identifier. Defaults to empty so callers may omit [default: ""]
      --quality <QUALITY>        Quality rating 1..=5. Defaults to 3 [default: 3]
      --supersedes <SUPERSEDES>  Comma-separated 8-hex memory ids this memory replaces (e.g. `a1b2c3d4,e5f6a7b8`). Recorded in the frontmatter `relations.supersedes` list and materialized as `supersedes` edges, so the older memories are demoted in ranking and annotated `superseded_by` in search results [default: ""]
      --vector <VECTOR>          Caller-supplied dense vector as a comma-separated float list. Length must equal the configured memory vector dim or the save fails with `vector dim mismatch`
      --vector-stdin             Read a JSON `{ "embedding": [..] }` payload from stdin and use it as the dense vector for the saved memory. Mutually exclusive with body being read from stdin (the body must be supplied as a positional arg when `--vector-stdin` is set)
  -h, --help                     Print help

Examples:
  # Save a decision with tags and elevated quality
  comemory save "Use Postgres for analytics" --kind decision --repo myrepo --tags db,postgres --quality 4

  # Pipe a bug report body from another command
  echo "Race in run_migration when run twice in <1s" | comemory save - --kind bug --repo myrepo

  # Save with a caller-supplied embedding (BYO-vector)
  echo '{"embedding":[0.1,0.2,...]}' | comemory save "...body..." --vector-stdin

  # Minimal note (kind defaults to `note`, no repo/tags)
  comemory save "Remember: cargo nextest serializes the embedder group"

  # Replace an outdated memory: a1b2c3d4 is annotated `superseded_by` in
  # search results and demoted in ranking (score_parts.supersede = 0.2)
  comemory save "new convention: pgbouncer in transaction mode" --supersedes a1b2c3d4

  # Near-duplicate detection: if a similar memory exists, a TTY warning is
  # printed to stderr and --json output includes a `duplicate_of` field with
  # the matching memory id. The save always proceeds — use `--supersedes` to
  # mark the relationship if the new memory replaces the old one.
```

---

## comemory search

```
Search the memory index by natural-language query

Usage: comemory search [OPTIONS] <QUERY>

Arguments:
  <QUERY>  Natural-language query string

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --k <K>                Page size — overrides the configured `retrieval.top_k`. `--limit` is an accepted alias. `0` means "all remaining within the `max_page_window`" [aliases: --limit]
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --offset <OFFSET>      Number of leading ranked results to skip (deep paging). Bounded by `retrieval.max_page_window`; once the window ceiling is reached `has_more` is false and deeper results require refining the query [default: 0]
      --repo <REPO>          Optional repo filter forwarded to the vector branch
      --kind <KIND>          Filter results to one memory kind [possible values: decision, bug, convention, discovery, pattern, note]
      --vector <VECTOR>      Caller-supplied dense vector as a comma-separated float list
      --vector-stdin         Read a JSON `{ "embedding": [..] }` payload from stdin and use it as the dense vector for the query
  -h, --help                 Print help

Examples:
  # Natural-language query, top 12 hits (default); weighted BM25 + priors
  comemory search "postgres pool exhausted"

  # Identifier-aware matching — camelCase/snake_case tokens split automatically
  comemory search "VecDimMismatch"

  # JSON output; hits[].score_parts breaks down every ranking factor:
  #   rrf         — pool-normalized relevance in [0,1]
  #   activation  — ACT-R recency boost (post-clamp), neutral = 1.0
  #   feedback    — Beta-smoothed used/irrelevant ratio, neutral = 1.0
  #   quality     — frontmatter quality nudge (1-5 scale), neutral = 1.0
  #   supersede   — 0.2 penalty when superseded by a live memory, else 1.0
  #   final_score — product of all factors (== score at root level)
  # The envelope also carries query_id — the retrieval_log row for this
  # run; pass it to `comemory feedback <query_id> --used <ids>`.
  comemory search "auth race" --json

  # Caller-supplied vector (BYO-vector, CSV form)
  comemory search "advisory lock" --vector 0.1,0.2,0.3,...
```

---

## comemory search-code

```
Search the code index by natural-language or identifier query

Usage: comemory search-code [OPTIONS] <QUERY>

Arguments:
  <QUERY>  Natural-language or identifier query string

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --k <K>                Page size — overrides the configured `retrieval.top_k`. `--limit` is an accepted alias. `0` means "all remaining within the `max_page_window`" [aliases: --limit]
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --offset <OFFSET>      Number of leading ranked results to skip (deep paging). Bounded by `retrieval.max_page_window`; once the window ceiling is reached `has_more` is false and deeper results require refining the query [default: 0]
      --repo <REPO>          Restrict hits to one repo label (as passed to `index-code --repo`)
      --lang <LANG>          Restrict hits to one language: `rust`, `typescript`, `javascript`, `python`, `go` (short aliases like `rs`/`ts`/`py` accepted)
      --vector <VECTOR>      Caller-supplied dense vector as a comma-separated float list
      --vector-stdin         Read a JSON `{ "embedding": [..] }` payload from stdin and use it as the dense vector for the query
  -h, --help                 Print help

Examples:
  # Lexical code search; identifier tokens split automatically
  comemory search-code "parse frontmatter"

  # JSON output; hits[].score_parts breaks down every ranking factor
  # (relevance, rank, activation, affinity, feedback, final_score) and
  # the envelope carries query_id — pass it to
  # `comemory feedback <query_id> --used-code <ids>`.
  comemory search-code "dim guard" --json

  # Scope to one repo and language (aliases like `rs`/`py` accepted)
  comemory search-code "router" --repo myrepo --lang rust

  # Caller-supplied vector (BYO-vector; code vectors are 768-dim)
  comemory search-code "knn" --vector 0.1,0.2,0.3,...

The working-set affinity boost applies only when search-code runs inside
the indexed repo's checkout (the CWD is used to detect dirty/recent files)
AND the repo label used at index time (`index-code --repo`) matches the
--repo flag — or, when --repo is omitted, the checkout directory's
basename.
```

---

## comemory list

```
List memories with optional repo/kind filters

Usage: comemory list [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --repo <REPO>          Filter to memories whose `repo` matches exactly
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --kind <KIND>          Filter by kind (case-insensitive): decision|bug|convention|discovery|pattern|note
      --limit <LIMIT>        Maximum number of results to return. `0` means "all" (no limit) [default: 50]
      --offset <OFFSET>      Number of leading results to skip before the window starts [default: 0]
  -h, --help                 Print help

Examples:
  # All decisions in a single repo
  comemory list --repo myrepo --kind decision

  # Every memory across all repos, JSON
  comemory list --json

  # Filter by kind only
  comemory list --kind bug

  # Second page of 20 memories
  comemory list --limit 20 --offset 20
```

---

## comemory delete

```
Soft-delete a memory by id (moves to `.trash/`)

Usage: comemory delete [OPTIONS] <ID>

Arguments:
  <ID>  8-hex memory id to delete

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  # Soft-delete by id (moves to memories/.trash/)
  comemory delete a1b2c3d4

  # JSON output for scripting
  comemory delete a1b2c3d4 --json
```

---

## comemory feedback

```
Record per-memory feedback (used vs irrelevant)

Usage: comemory feedback [OPTIONS] <QUERY_ID>

Arguments:
  <QUERY_ID>  Id of the originating search query (`q-<yyyymmdd>-<8hex>`, as printed by `comemory search`); recorded for provenance

Options:
      --json
          Emit machine-readable JSON instead of a human TTY view
      --used <USED>
          Comma-separated memory ids that were used [default: ""]
      --data-dir <DATA_DIR>
          Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --irrelevant <IRRELEVANT>
          Comma-separated memory ids that were judged irrelevant [default: ""]
      --used-code <USED_CODE>
          Comma-separated code-symbol ids (positive integers, as printed by `comemory search-code`) that were used [default: ""]
      --irrelevant-code <IRRELEVANT_CODE>
          Comma-separated code-symbol ids that were judged irrelevant [default: ""]
  -h, --help
          Print help

Examples:
  # Mark two hits as useful and one as irrelevant
  comemory feedback q-20260610-a1b2c3d4 --used a1b2c3d4,e5f6a7b8 --irrelevant 00112233

  # Only-used feedback
  comemory feedback q-20260610-b2c3d4e5 --used a1b2c3d4

  # Only-irrelevant feedback
  comemory feedback q-20260610-c3d4e5f6 --irrelevant 00112233

  # Code-symbol feedback (ids printed by comemory search-code)
  comemory feedback q-20260610-d4e5f6a7 --used-code 12 --irrelevant-code 13

  # Memory and code verdicts in one call
  comemory feedback q-20260610-e5f6a7b8 --used a1b2c3d4 --used-code 12
```

---

## comemory eval

```
Score retrieval quality against a golden set (recall@k, MRR)

Usage: comemory eval [OPTIONS]

Options:
      --golden <GOLDEN>      Path to a YAML golden file (`- query: ...` / `  relevant: [..]`)
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --golden-only          Skip the feedback harvest; use only the --golden file
      --k <K>                recall@k cut (defaults to 3) [default: 3]
  -h, --help                 Print help

Examples:
  # Score retrieval against feedback-harvested golden pairs
  comemory eval

  # Merge a hand-written golden file (file wins on duplicate query)
  comemory eval --golden golden.yaml

  # File only, recall@5, JSON report
  comemory eval --golden golden.yaml --golden-only --k 5 --json
```

---

## comemory mine

```
Mine reformulation pairs from the query log into term-expansion mappings (report only; `--apply` rebuilds `query_expansions`)

Usage: comemory mine [OPTIONS]

Options:
      --apply                Rebuild the query_expansions table from the mined mappings (default: report only)
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  # Report mined expansion mappings without changing retrieval
  comemory mine

  # Rebuild the query_expansions table from the current retrieval_log
  comemory mine --apply

  # Machine-readable report
  comemory mine --json
```

---

## comemory tune

```
Grid-search blend weights against the golden set (report only; `--apply` writes the winner into config.toml)

Usage: comemory tune [OPTIONS]

Options:
      --golden <GOLDEN>      Path to a YAML golden file (`- query: ...` / `  relevant: [..]`)
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --golden-only          Skip the feedback harvest; use only the --golden file
      --k <K>                recall@k cut (defaults to 3) [default: 3]
      --apply                Rewrite config.toml with the winning knobs when (and only when) the winner strictly beats the current config. Comments in an existing config.toml are dropped by the rewrite
  -h, --help                 Print help

Examples:
  # Grid-search the configured [tune] grid (81 configs by default)
  # against the merged golden set (report only)
  comemory tune

  # File-only golden set, recall@5, machine-readable report
  # (JSON envelope: {"report": <TuneReport>, "applied": bool})
  comemory tune --golden golden.yaml --golden-only --k 5 --json

  # Write the winning knobs into config.toml (atomic rename; the file
  # is re-rendered from parsed TOML, so comments are dropped)
  comemory tune --golden golden.yaml --apply
```

---

## comemory doctor

```
Report on the data directory and SQLite mirror health

Usage: comemory doctor [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  # Human-readable health report
  comemory doctor

  # JSON for monitoring or CI
  comemory doctor --json
```

---

## comemory index-code

```
Walk a repo, extract symbols, and upsert into the code index

Usage: comemory index-code [OPTIONS] --repo <REPO> --path <PATH>

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --repo <REPO>          Repo label stored alongside each symbol row
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --path <PATH>          Root of the working tree to walk. Must live inside a git repo so blob OIDs are available for the incremental skip path
      --extract              Emit JSONL on stdout instead of inserting rows. Suitable for piping into an external embedder + `comemory ingest-code`
  -h, --help                 Print help

Examples:
  # Index the current working directory with explicit repo label
  comemory index-code --repo myrepo --path .

  # Emit one JSONL row per symbol on stdout (skips DB writes)
  comemory index-code --repo myrepo --path ./src --extract
```

---

## comemory ingest-code

```
Read pre-embedded JSONL rows from stdin and ingest them into the code index (`code_symbols` + `code_fts` + `code_vec`)

Usage: comemory ingest-code [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  # Pipe pre-embedded JSONL from your embedder into the SQLite store
  comemory index-code --repo myrepo --path . --extract \
    | embed-snippets \
    | comemory ingest-code
```

---

## comemory ast

```
Run an ast-grep pattern against a single source file

Usage: comemory ast [OPTIONS] --lang <LANG> --file <FILE> <PATTERN>

Arguments:
  <PATTERN>  ast-grep pattern (`$VAR`, `$$$ARGS`, etc.)

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --lang <LANG>          Language tag: `rs`/`rust`, `ts`/`tsx`/`typescript`, `js`/`jsx`/`javascript`, `py`/`python`, `go`
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --file <FILE>          File to search
      --limit <LIMIT>        Maximum number of results to return. `0` means "all" (no limit) [default: 50]
      --offset <OFFSET>      Number of leading results to skip before the window starts [default: 0]
  -h, --help                 Print help

Examples:
  # Match every fn returning Result<_>
  comemory ast 'fn $NAME($$$ARGS) -> Result<$RET>' --lang rs --file src/db.rs

  # Find tokio::spawn call sites
  comemory ast 'tokio::spawn($$$)' --lang rs --file src/lib.rs --json

  # Hunt for `console.log` left in TypeScript
  comemory ast 'console.log($$$)' --lang ts --file src/index.ts
```

---

## comemory graph

```
Export the file-level code-connection graph (imports + co-change) as JSON, Graphviz DOT, or an interactive HTML page

Usage: comemory graph [OPTIONS]

Options:
      --json
          Emit machine-readable JSON instead of a human TTY view

      --repo <REPO>
          Restrict to one repo label (as passed to `index-code --repo`)

      --data-dir <DATA_DIR>
          Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable
          
          [env: COMEMORY_DATA_DIR=]

      --format <FORMAT>
          Output format

          Possible values:
          - json: Machine-readable `{ nodes, edges }` JSON
          - dot:  Graphviz DOT source (pipe to `dot`)
          - html: Interactive HTML page (sigma.js, loaded from a CDN)
          
          [default: json]

      --rel <REL>
          Which edge relations to include

          Possible values:
          - all:        Both `imports` and `co_changed`
          - imports:    Static import edges only
          - co-changed: Git co-change edges only
          
          [default: all]

      --min-weight <MIN_WEIGHT>
          Drop `co_changed` edges whose accumulated weight is below this floor (does not affect `imports`, which always carry weight 1). Must be >= 1
          
          [default: 1]

      --limit <LIMIT>
          Maximum number of results to return. `0` means "all" (no limit)
          
          [default: 50]

      --offset <OFFSET>
          Number of leading results to skip before the window starts
          
          [default: 0]

  -h, --help
          Print help (see a summary with '-h')

Examples:
  # Whole graph as JSON (every indexed repo)
  comemory graph

  # Interactive viewer for one repo
  comemory graph --repo myrepo --format html > graph.html && open graph.html

  # Graphviz DOT, imports only, piped to an SVG
  comemory graph --repo myrepo --rel imports --format dot | dot -Tsvg > graph.svg

  # Drop weak co-change links (accumulated weight < 3)
  comemory graph --rel co-changed --min-weight 3
```

---

## comemory serve

```
Launch the local web viewer + in-browser code editor (loopback HTTP)

Usage: comemory serve [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --repo <REPO>          Restrict the graph to one repo label (as passed to `index-code --repo`)
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --port <PORT>          Loopback port to bind. `0` (default) selects an ephemeral port whose URL is printed at startup [default: 0]
      --read-only            Disable all writes: `PUT /api/file` returns 405 and the editor's Save action is hidden
      --root <REPO=PATH>     Override a repo's working-tree root as `<repo>=<abs-path>` (repeatable). Required for repos indexed before the v7 schema captured the root
      --open                 Open the printed URL in the default browser after binding. The URL carries the session token and is passed as an argument to the system opener, so it is briefly visible to other local users (e.g. via `/proc/<pid>/cmdline` or `ps`)
  -h, --help                 Print help

Examples:
  # Serve the graph + editor for every indexed repo on an ephemeral port
  comemory serve

  # One repo, fixed port, opened in the browser
  comemory serve --repo myrepo --port 8787 --open

  # Read-only exploration (no writes to disk)
  comemory serve --read-only

  # Supply a repo root for repos indexed before the v7 schema captured it
  comemory serve --root myrepo=/abs/path/to/repo
```

---

## comemory context

```
Headline lookup: code symbol + memories matching a key

Usage: comemory context [OPTIONS] <QUERY>

Arguments:
  <QUERY>  Free-form query — symbol name, file path fragment, or phrase

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --k <K>                Page size for the bundle's memory list — overrides the configured `retrieval.top_k`. `--limit` is an accepted alias. `0` means "all remaining within the `max_page_window`" [aliases: --limit]
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --offset <OFFSET>      Number of leading ranked memories to skip (deep paging of the bundle's memory list). Bounded by `retrieval.max_page_window`. Per- memory code refs are not paginated — each surfaced memory keeps its full ref set [default: 0]
      --repo <REPO>          Optional repo filter forwarded to the router
      --vector <VECTOR>      Caller-supplied dense vector as a comma-separated float list. When provided together with `query`, both ANN and lexical branches run and their results are fused via RRF. Without a vector only the lexical FTS5 path runs
      --vector-stdin         Read a JSON `{ "embedding": [..] }` payload from stdin and use it as the dense vector for the context lookup. Mutually exclusive with reading the query from stdin
  -h, --help                 Print help

Examples:
  # Headline lookup for a symbol name, JSON envelope
  comemory context run_migration --json

  # Pin the bundle width to the top 3 hits
  comemory context "advisory lock" --k 3

  # ANN-assisted context with a caller-supplied vector
  comemory context "advisory lock" --vector 0.1,0.2,...

Code refs in the bundle are ranked by graph priors (PageRank, recency,
working-set affinity, feedback); each resolved ref carries a rank_parts
breakdown in --json mode. The working-set affinity boost applies only
when context runs inside the referenced repo's checkout (the CWD is used
to detect dirty/recent files) AND the repo label used at index time
(`index-code --repo`) matches the --repo flag — or, when --repo is
omitted, the checkout directory's basename.
```

---

## comemory prune

```
Detect (and optionally soft-delete) stale memories

Usage: comemory prune [OPTIONS]

Options:
      --apply                Execute the cleanup (soft-delete low-value memories, drop orphan edges + stale code symbols). Without this flag prune only scans and reports
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --limit <LIMIT>        Maximum number of results to return. `0` means "all" (no limit) [default: 50]
      --offset <OFFSET>      Number of leading results to skip before the window starts [default: 0]
  -h, --help                 Print help

Examples:
  # Default is a dry run: inspect candidates without mutating anything
  comemory prune

  # Apply: soft-delete low-value memories (markdown -> memories/.trash/)
  # and clean up orphan edges + stale code symbols
  comemory prune --apply

  # Page the dry-run lists (window applies to display only; --apply is
  # always full-set): second page of 20 candidates
  comemory prune --limit 20 --offset 20

  # JSON output for CI/automation; Report fields:
  #   low_value_memories / stale_code_files — Page envelopes
  #     ({items, limit, offset, total, has_more}). low_value ids match ALL
  #     of: activation < COMEMORY_PRUNE_MIN_ACTIVATION (-2.0), Beta feedback
  #     <= COMEMORY_PRUNE_MIN_FEEDBACK (0.25), quality <=
  #     COMEMORY_PRUNE_BELOW_QUALITY (2), and zero incoming edges — OR
  #     superseded by a live memory with no access since the supersede edge.
  comemory prune --json
```

---

## comemory rebuild

```
Drop `comemory.db` and repopulate it from the markdown source of truth

Usage: comemory rebuild [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help
```

---

## comemory gc

```
Purge old `memories/.trash/` entries and learning telemetry past retention

Usage: comemory gc [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  # Hard-delete .trash entries and learning telemetry past retention
  comemory gc

  # Tighten the telemetry window (retrieval_log + feedback_events) to a week
  COMEMORY_LEARNING_RETENTION_DAYS=7 comemory gc

  # JSON output for CI/automation
  comemory gc --json
```

---

## comemory install-hooks

```
Install git hooks that trigger `comemory index-code` on `post-commit`, `post-merge`, and `post-checkout`

Usage: comemory install-hooks [OPTIONS]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --repo <REPO>          Repo root to install hooks into. Defaults to the current working directory [default: .]
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
      --force                Overwrite existing hook files. Without this flag the command refuses to clobber a pre-existing `post-commit`/`post-merge`/`post-checkout` to avoid surprising users with hand-written hooks
  -h, --help                 Print help

Examples:
  # Install into the current repo
  comemory install-hooks

  # Install into a specific repo path
  comemory install-hooks --repo /path/to/repo

  # Overwrite any hand-written hooks
  comemory install-hooks --force
```

---

## comemory completions

```
Emit a shell completion script for `bash`, `zsh`, `fish`, `powershell`, or `elvish`

Usage: comemory completions [OPTIONS] <SHELL>

Arguments:
  <SHELL>  Shell to emit a completion script for [possible values: bash, elvish, fish, powershell, zsh]

Options:
      --json                 Emit machine-readable JSON instead of a human TTY view
      --data-dir <DATA_DIR>  Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable [env: COMEMORY_DATA_DIR=]
  -h, --help                 Print help

Examples:
  # fish (autoloaded from this path)
  comemory completions fish > ~/.config/fish/completions/comemory.fish

  # zsh (homebrew site-functions path)
  comemory completions zsh > "$(brew --prefix)/share/zsh/site-functions/_comemory"

  # bash (homebrew bash-completion.d)
  comemory completions bash > "$(brew --prefix)/etc/bash_completion.d/comemory"

  # NOTE: scripts/dev-install.sh writes these automatically by default.
```

