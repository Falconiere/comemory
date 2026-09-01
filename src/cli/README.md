# cli/

**What belongs here:** clap subcommand entry points — one file per
`comemory <subcommand>`, each owning its own `Args` shape, thin orchestration
(`run`), and output rendering hookup — plus the top-level dispatcher
(`Cli`/`Cmd` in `src/cli.rs`) and the small cross-cutting flag layers shared by
several subcommands (`when`, `pagination`, `ref_args`, `embedding_input`,
`search_only`).

**What does NOT belong here:** business logic. A `cli/*.rs` file parses flags,
loads `Config`, calls into `retrieval::`, `graph::`, `store::`, `memory::`, or
`prune::` to do the real work, and hands the result to `output::` to render.
Keeping the logic out of `cli/` is what lets `eval::runner` and tests exercise
the same pipelines without going through argument parsing.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `ast.rs` | `Args` | `comemory ast` — run an ast-grep pattern against a source file |
| `bandit.rs` | `Args` | `comemory bandit` — Thompson-sample the `[tune]` grid, confirm with offline eval |
| `completions.rs` | `Args` | `comemory completions <shell>` — emit a shell completion script |
| `consolidate.rs` | `Args` | `comemory consolidate` — advisory near-duplicate cluster report |
| `context.rs` | `Args` | `comemory context` — headline memory + code bundle lookup |
| `delete.rs` | `Args` | `comemory delete` — soft-delete a memory by id |
| `doctor.rs` | `Args` | `comemory doctor` — runtime health check against the SQLite storage stack |
| `edges.rs` | `Args` | `comemory edges` — lexical search over the relation graph |
| `embedding_input.rs` | `EmbeddingPayload` | Shared `--vector` / `--vector-stdin` parsing for `save` and `search` |
| `eval.rs` | `GoldenSetArgs` | `comemory eval` — score retrieval quality (recall@k, MRR) against a golden set |
| `feedback.rs` | `Args` | `comemory feedback` — record used/irrelevant feedback into the stats DB |
| `gc.rs` | `run` | `comemory gc` — purge `.trash/` and evict expired learning telemetry |
| `find.rs` | `Args` | `comemory find` — one ranked list over memories, code, and documents |
| `graph.rs` | `Format` | `comemory graph` — export the file-level code-connection graph; node assembly lives in `graph/nodes.rs` |
| `hooks.rs` | `Args` | `comemory hooks` — report and toggle the git reindex hooks individually |
| `index.rs` | `Args` | `comemory index <PATH>...` — register document sources and reconcile them |
| `index_code.rs` | `Args` | `comemory index-code` — incremental symbol extraction over a git repo |
| `ingest_code.rs` | `Args` | `comemory ingest-code` — bulk pre-embedded code-symbol ingestion from stdin |
| `install_hooks.rs` | `Args` | `comemory install-hooks` — install git hooks that trigger `index-code` |
| `lazy_reindex.rs` | `RepoContext` | Detached, non-blocking auto-reindex trigger behind `indexing.auto_reindex = lazy` |
| `list.rs` | `Args` | `comemory list` — page live memories with `--repo` / `--kind` filters |
| `mine.rs` | `Args` | `comemory mine` — distill query reformulations from `retrieval_log` into expansions |
| `pagination.rs` | `PaginationArgs` | Shared `--k` / `--offset` window flags, flattened into paginated commands |
| `prune.rs` | `Args` | `comemory prune` — surface deletion candidates against the SQLite mirror |
| `rebuild.rs` | `Args` | `comemory rebuild` — atomically rebuild the SQLite mirror from markdown |
| `ref_args.rs` | `collect` | Parse `--ref-file` / `--ref-symbol` into a `References` block |
| `save.rs` | `Args` | `comemory save` — atomic markdown write + SQLite-mirror upsert |
| `search.rs` | `Args` | `comemory search` — natural-language search over the memory store |
| `search_code.rs` | `Args` | `comemory search-code` — ranked search over indexed `code_symbols` |
| `search_only.rs` | `OnlyDomain` | `--only`/`--path` domain-scope resolution shared by `search` |
| `serve.rs` | `Args` | `comemory serve` — launch the local web viewer + in-browser code editor |
| `repos.rs` | `Args` | `comemory repos` — indexed code repositories and their index freshness |
| `show.rs` | `Args` | `comemory show` — one memory in full: body, frontmatter, activation, refs |
| `sources.rs` | `Args` | `comemory sources` — list registered document sources with status counts |
| `stats.rs` | `Args` | `comemory stats` — corpus counters and `comemory.db` size |
| `tui.rs` | `Args` | `comemory tui` — launch the read-only interactive terminal explorer |
| `tune.rs` | `Args` | `comemory tune` — deterministic/sampled search over the blend knobs |
| `unindex.rs` | `Args` | `comemory unindex <SOURCE_ID\|PATH>` — unregister a document source |
| `when.rs` | `DayEdge` | `--since`/`--until`/`--as-of` value parsing shared by `search` and `context` |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/cli.rs` (`pub mod <name>;`)
and the dispatcher (`Cmd`) imports concrete paths.
