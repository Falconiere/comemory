# CLAUDE.md

## Project Overview

`comemory` is a Rust CLI that fuses engram-style developer memory, grepai-style
semantic code search, and ast-grep AST patterns into a single binary, knit
together by a SQLite-backed store (memory + code rows + edges). It is a
**standalone agentic-RAG toolbox** invoked from the command line — not a
Claude Code MCP plugin or in-process LLM. Everything runs locally: markdown
is the source of truth and one SQLite file (`comemory.db`) backs FTS5 +
`sqlite-vec` + edges.

## Architecture

- **Source of truth:** markdown files with YAML frontmatter at
  `~/.comemory/memories/{id}-{slug}.md` (override with `COMEMORY_DATA_DIR`).
- **Single SQLite file:** `~/.comemory/comemory.db` with `memories`,
  `memory_fts` (FTS5), `memory_vec` (`sqlite-vec` `vec0`), `code_symbols`,
  `code_fts`, `code_vec`, `edges`, `schema_meta`, plus stats / repo-marker
  tables. `rusqlite 0.32` with `bundled` + `load_extension` features.
- **Edges:** flat `(src_kind, src_id, edge_kind, dst_kind, dst_id)` rows
  (plus an integer `weight`) in the `edges` table replace the v0.1 kuzu
  graph. v6 adds code-graph kinds: `co_changed` (mined from git history)
  and `imports` (per-language import resolution), feeding a materialized
  PageRank on `code_symbols.rank_score`. Recursive CTEs handle multi-hop
  traversal.
- **AST extraction:** `ast-grep-core 0.38` + `ast-grep-language 0.38` (rust,
  typescript, javascript, python, go only).
- **Vectors are BYO.** No in-process embedder. Callers pass vectors via
  `--vector` (CSV) or `--vector-stdin` (JSON `{"embedding":[..]}`). A sample
  Ollama wrapper ships in `scripts/comemory-embed.sh`.
- **Output:** TTY via `owo-colors`, JSON via `serde_json`. Exit codes follow
  `sysexits.h`.
- **No in-process LLM.** All ranking is deterministic (RRF fusion of FTS5 +
  `sqlite-vec`, a tiered lexical fallback ladder ending in mined learned
  expansions, edge walks).

## Key Commands

```bash
cargo install --path .          # build + install the binary locally
just check                      # umbrella gate (alias of scripts/check-all.sh)
just test                       # cargo nextest run --all-features
just qa                         # check-all + cargo-deny + dup-check + machete
just e2e                        # real-binary end-to-end harness
bash scripts/check-all.sh       # the umbrella gate (CI parity)
cargo nextest run --all-features
comemory doctor                    # runtime health check
comemory search-code "query"       # ranked code search (BM25 + graph priors)
comemory eval                      # score retrieval (recall@k, MRR) vs golden set
comemory mine --apply              # distill query reformulations into expansions
comemory tune --apply              # grid-search ranking knobs into config.toml
```

## Binding Rules (apply to every contribution)

These are reproduced verbatim from the implementation plan header and are
enforced by `scripts/check-all.sh`. Every PR must satisfy all five.

1. **No duplication / redundancy.** Shared logic is extracted into a helper.
   Enforced by `scripts/dup-check.sh` and reviewer scrutiny.
2. **Very modular modules.** Each `src/<module>/` directory contains
   narrow, single-purpose files. Files that change together live together.
3. **≤300 code lines per file in `src/` (blanks/comments excluded).**
   Enforced by `scripts/module-size-check.sh`.
4. **Zero errors, zero warnings.** No `#[allow(...)]` overrides, no
   `// clippy::allow`, no `.unwrap()` outside `tests/`, no `.expect(...)`
   (any, with or without a message) in `src/`, no `println!` / `eprintln!` /
   `todo!()` / `unimplemented!()` / `panic!` in `src/`, no `unsafe { … }`
   without an adjacent `// SAFETY:` comment within 3 lines above. Enforced by
   `scripts/no-bypass-check.sh`.
5. **Tests strictly in `tests/` mirroring `src/` 1:1, FLAT.** No
   `#[cfg(test)] mod tests { … }` block ever appears inside any file in
   `src/`. Items needing tests are exposed via `pub(crate)`. The mirror is
   flat and dunder-joined: `src/<path>.rs` maps to a single
   `tests/<dunder-path>.rs` (e.g. `src/store/tokenizer/split.rs` ↔
   `tests/store__tokenizer__split.rs`). Each flat file is its own
   integration-test binary — there are no `tests/<module>.rs` shims and no
   nested `tests/<module>/` submodule directories. An oversized test file is
   split into `<base>.rs` + `<base>_2.rs`. Shared helpers/fixtures live in
   `tests/common/`. Enforced by `scripts/test-placement-check.sh` and
   `scripts/tests-mirror-check.sh`.

## Code Style

- `rustfmt` defaults — **4-space indent**, 100-column line length
  (`rustfmt.toml`).
- **≤300 code lines per `src/` file** (blanks/comments excluded; see
  Binding Rule 3) — split into submodules before crossing it.
- `cargo clippy --all-targets --all-features -- -D warnings`.
- Doc comments (`///`) on every public item.
- `Result<T>` alias from `crate::prelude::*`; errors flow through the
  `Error` enum in `src/errors.rs`.
- Use `tracing` for diagnostics, never `println!` / `eprintln!`.

## Module Map

| Module | Responsibility |
|--------|---------------|
| `cli/` | clap subcommand entry points + the top-level dispatcher in `mod.rs` |
| `tui/` | read-only interactive terminal explorer (`comemory tui`): ratatui front end + async `EventStream`/`tokio::select!` loop (`mod.rs`), pure state (`app`) + key map (`event`), a dedicated-thread DB-worker that owns the connection (`worker`), the lexical/semantic request bridge (`search`), preview text (`preview`), RAII terminal guard (`terminal`), and pure ratatui widgets (`view/`). Embed shell-out lives in the shared `embed/` module |
| `embed/` | shared embed-command shell-out (`mod.rs`) — runs `COMEMORY_EMBED_CMD` / `--embed-cmd` as `sh -c <cmd>`, feeds the query on stdin, parses `{"embedding":[..]}`. Consumed by `tui` (Ctrl-S semantic enrich) and `serve` (the `/api/search` hybrid leg). Moved here out of `tui` |
| `serve/` | loopback web viewer (`comemory serve`): axum `router` + `handlers`, embedded SPA `assets`, on-disk `fileio` (`PUT /api/file`, gated by `--read-only` → 405), `search` (the `GET /api/search` handler — calls `retrieval::code_search` and coalesces symbol hits to file-level `{node_id, repo, path, score, top_symbol}`), `repo_root` resolution, `security` (session token), `error` |
| `memory/` | markdown I/O, `Frontmatter`, slug, id (8-hex SHA-256), atomic save / load / soft-delete / list |
| `store/` | central SQLite layer — `connection` (pooled rusqlite + `sqlite-vec` loader), `schema`, `migrate` (versioned + idempotent), `vector` (`vec0` insert/KNN with dim guard), `fts` (FTS5 helpers), `embed` (`to_vec_blob`, dim helpers), `memory_meta` (`fetch_meta` — batched per-memory metadata: path/repo/kind/tags/references backing the enriched `search --json` rows), `tokenizer` (custom FTS5 identifier tokenizer: camelCase/snake_case split + FFI registration) |
| `simhash.rs` | 64-bit SimHash + Hamming distance over tokenized memory bodies (siphasher-based) |
| `graph/` | SQL-backed `edges` table upserts, recursive-CTE walks, `cross_link` reference extraction, `cochange` (git-history co-change mining), `imports` (per-language import edges), `pagerank` (deterministic weighted PageRank), `materialize` (writes `rank_score` onto `code_symbols`) |
| `retrieval/` | `router` (candidates + 4-tier lexical ladder: strict → word-OR → subtoken-OR → tier-4 learned expansion from mined `query_expansions`), `score` (ACT-R/Beta scoring primitives), `rerank` (multiplicative priors over the max-normalized relevance: activation × feedback × quality × supersede), `diversify` (SimHash near-dup collapse + MMR), `pipeline` (orchestration + access tracking), `fuse` (RRF), `bundle` (context lookup with graph-prior-ranked code refs), `code_route` (code candidates: BM25 + thresholded ANN + RRF, chunk→parent coalesce), `code_rerank` (four-prior code rerank), `code_prior` (PageRank / recency / working-set affinity / feedback priors), `code_search` (`search_code_hits` — the shared code-search entry point used by both the `search-code` CLI and the `serve` `/api/search` handler) |
| `eval/` | learning loop — `golden` (YAML golden sets + feedback harvest), `metrics` (recall@k, MRR), `runner` (eval over the real pipeline, tracking off), `mine` (reformulation mining → `query_expansions`), `tune` (deterministic grid search over the blend knobs) |
| `ast/` | `extractor` (symbol enumeration via tree-sitter through ast-grep — rust/ts/js/py/go only), `chunk` (cAST split of oversized symbols into child rows at AST boundaries), `pattern` (user-facing `comemory ast`), per-language wiring |
| `stats/` | usage / feedback / `code_feedback` (per-symbol counters) / repo-marker tables (lives inside `comemory.db`) |
| `config/` | layered config (defaults → file → env) and `Paths` (data-dir layout) |
| `output/` | TTY (`owo-colors`) and JSON (`serde_json`) emitters, shared between subcommands |
| `prune/` | orphan / low-value / stale-code detection plus soft-delete & gc |
| `git_utils.rs` | repo + author auto-detection, blob OID lookup, git-hook installation helpers |
| `errors.rs` | `thiserror`-derived `Error` enum and `Result<T>` alias |
| `prelude.rs` | crate-internal prelude (`Error`, `Result`, common imports) |
| `lib.rs` / `main.rs` | library surface + binary entry that parses `Cli` and calls `cli::run` |

## Environment Variables

Values are layered: defaults (`Config::defaults`) → optional config file →
environment (`Config::with_env`, in `src/config/env.rs`).

| Variable | Purpose | Default |
|----------|---------|---------|
| `COMEMORY_DATA_DIR` | Root data directory (`memories/` + `comemory.db`) | `~/.comemory` |
| `COMEMORY_INDEXING_AUTO_REINDEX` | `lazy` \| `hook` \| `off` — automatic code-index refresh. `lazy` (wired in `src/cli/lazy_reindex.rs`): `search-code`/`context` spawn a detached, non-blocking `index-code` when the repo HEAD moved since the last index, then search the current index immediately; `hook` relies on installed git hooks; `off` is manual-only | `lazy` |
| `COMEMORY_RETRIEVAL_TOP_K` | Number of results returned by the hybrid router (also the default page size for `search` / `search-code` / `context` when `--k`/`--limit` is omitted) | `12` |
| `COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW` | Maximum depth pagination can reach into the ranked result list. `search` / `search-code` / `context` fetch a candidate pool sized `clamp(offset + k + k, CANDIDATE_POOL, max_page_window)`, run the full fuse → rerank → diversify pipeline over it, then slice `[offset, offset+k]`; `has_more` is forced false once this ceiling is hit (deeper results require refining the query). Validated `> 0`. | `200` |
| `COMEMORY_RETRIEVAL_MEMORY_THRESHOLD` | Minimum cosine similarity for the memory table | `0.55` |
| `COMEMORY_RETRIEVAL_CODE_THRESHOLD` | Minimum cosine similarity for the code table (ANN leg of `search-code`, range `[0.0, 1.0]`) | `0.50` |
| `COMEMORY_RETRIEVAL_RRF_K` | RRF fusion constant for hybrid scoring | `60.0` |
| `COMEMORY_RETRIEVAL_BM25_WEIGHTS` | `"body,tags"` BM25 column weights for `memory_fts` (both finite ≥ 0, at least one > 0) | `1.0,3.0` |
| `COMEMORY_RETRIEVAL_CODE_BM25_WEIGHTS` | `"symbol,snippet,path_tokens"` BM25 column weights for `code_fts` (all finite ≥ 0, at least one > 0) | `2.0,1.0,1.5` |
| `COMEMORY_LEARNING_RETENTION_DAYS` | `comemory gc` retention window (days) for raw `retrieval_log` + `feedback_events` rows; aggregated `feedback` counters and mined `query_expansions` never expire | `90` |
| `COMEMORY_TUNE_MIN_GOLDEN` | Test hook lowering `comemory tune` / `comemory bandit`'s minimum-golden-pairs floor; not a tuning knob | `10` |
| `COMEMORY_REINFORCE_SEARCH_EDIT_DAYS` | Lookback days for search→edit auto-reinforcement provenance (`auto_search_edit`). Validated `≥ 1` | `7` |
| `COMEMORY_DISABLE_ACCESS_TRACKING` | Test hook (truthy) disabling `search` / `context` access tracking + `retrieval_log` writes for one run, so a stability harness can drive the binary repeatedly without each query mutating `access_count` / `last_accessed` (which feeds ACT-R activation and reorders ranking between calls); not a user knob | `false` |
| `COMEMORY_GIT_AUTO_SYNC` | `true`/`1` to enable best-effort git commit + push after a save | `false` |
| `COMEMORY_EMBED_HINT` | Free-form identifier of the embedder you used (e.g. `ollama:nomic-embed-text`). Surfaced by `comemory doctor`; never consumed as a switch. | unset |
| `COMEMORY_EMBED_CMD` | Embed command used by `comemory tui`'s Memory-tab semantic enrich (Ctrl-S) **and** by `comemory serve`'s `/api/search` to upgrade lexical file search to hybrid. Run as `sh -c <cmd>`; reads the query on stdin, must emit `{"embedding":[..]}` on stdout. The per-command `--embed-cmd` flag (on `tui` and `serve`) overrides it. Unset → semantic enrich is a no-op / serve search stays lexical; lexical search always works. | unset |
| `COMEMORY_RANK_DECAY` | ACT-R decay exponent `d` in `ln(n) − d·ln(days+1)`. Must be ≥ 0. Higher → older memories decay faster. | `0.5` |
| `COMEMORY_RANK_PRIOR_CLAMP` | `"lo,hi"` bounds applied to the activation, feedback, and quality boost multipliers (the fixed `0.2` supersede penalty intentionally bypasses the clamp). Both finite; lo > 0, lo ≤ hi. | `0.5,2.0` |
| `COMEMORY_RANK_MMR_LAMBDA` | MMR relevance-vs-diversity trade-off in `[0.0, 1.0]`. `1.0` = pure relevance; `0.0` = pure diversity. | `0.7` |
| `COMEMORY_RANK_NEAR_DUP_HAMMING` | SimHash Hamming radius for near-dup detection (save-time advisory + diversify collapse). Must be ≤ 64 (SimHash is 64-bit). | `8` |
| `COMEMORY_PRUNE_MIN_ACTIVATION` | Activation floor (ACT-R scale) below which a memory is prune-eligible. | `-2.0` |
| `COMEMORY_PRUNE_MIN_FEEDBACK` | Beta-feedback ceiling (range `[0.0, 1.0]`) at or below which a memory is prune-eligible. | `0.25` |
| `COMEMORY_PRUNE_BELOW_QUALITY` | Quality threshold (1..=5); memories at or below this value are prune candidates (used together with activation + feedback floors). | `2` |
| `COMEMORY_PRUNE_SUPERSEDED_GRACE_DAYS` | Grace window (days) before a superseded-and-never-accessed memory becomes prune-eligible; protects freshly-rebuilt DBs whose supersede edges all carry rebuild-time timestamps. | `7` |

The `[tune]` grid knobs (`tune.rrf_k_grid`, `tune.decay_grid`,
`tune.mmr_lambda_grid`, `tune.bm25_grid`) are file-only — set them in
`config.toml`; they have no env override.

The memory and code vector dims (1024 and 768) are baked into the
`memory_vec` / `code_vec` vec0 DDL (`src/store/sql/0002_v2_tables.sql`)
at migration time and are not env-configurable: a divergent env value
would silently disagree with the vtab and surface as `VecDimMismatch`
at first insert. Change the literal in the DDL if you need a different
dim.

CLI flags `--data-dir` and `--json` are global and can appear before or
after the subcommand.

## Memory Data Model

Frontmatter schema v1, defined by `src/memory/frontmatter.rs::Frontmatter`:

```yaml
---
id: a1b2c3d4                  # 8-hex prefix of SHA-256(body.trim_end())
kind: decision                # decision|bug|convention|discovery|pattern|note
repo: qwick-backend           # single repo string (not a list)
tags: [database, postgres]
author: falconiere
created: 2026-05-17T14:30:00Z
quality: 4                    # 1..=5, default 3
schema: 1
content_hash: <64-hex SHA-256 of body.trim_end()>
references:
  symbols: []                 # qualified symbol ids: <repo>:<path>:<name>
  files:   []                 # qualified file paths:  <repo>:<path>
relations:
  supersedes:     []          # memory ids this one replaces
  conflicts_with: []          # memory ids this one contradicts
  derived_from:   []          # memory ids this one builds on
---

Markdown body lives here.
```

## Save Flow (BYO-vector, current implementation)

`comemory save` runs:

1. Parse args, resolve repo/author defaults, build `Frontmatter` with
   `schema: 1` and `content_hash = sha256(body.trim_end())`.
2. If `--vector` (CSV) or `--vector-stdin` (JSON `{"embedding":[..]}`) is
   set, parse it into a `Vec<f32>` and run the `store::embed::dim_guard`
   against `schema_meta` so a mismatched embedder fails fast with
   `Error::VecDimMismatch`. With neither flag, the save is lexical-only —
   no `memory_vec` row is written.
2a. **Near-duplicate check** (best-effort, advisory): scan live `memories`
   rows for a SimHash Hamming distance within `NEAR_DUP_HAMMING`. If a
   near-duplicate is found its id is recorded as `duplicate_of`. The save
   always proceeds; the caller decides whether to re-save with
   `--supersedes <id>`. TTY
   mode prints a `warning: similar memory <id> exists` to stderr;
   `--json` mode includes `"duplicate_of": "<id>"` in the output object.
   Self-matches (re-save of the same body, same content-hash-derived id)
   are filtered out.
3. Atomic stage: write `memories/.{id}.tmp`, then `fs::rename` to
   `memories/{id}-{slug}.md`. On failure between stage and rename, the tmp
   file is removed.
4. Single `store` transaction:
   - upsert `memories` row (frontmatter + body + simhash)
   - upsert `memory_fts` row (FTS5)
   - upsert `memory_vec` row (`vec0`) when a vector was supplied
   - frontmatter `relations.{supersedes,conflicts_with,derived_from}` ids
     (`supersedes` is populated by `--supersedes`; the others come from
     hand-edited markdown) are materialized as memory→memory `edges` rows.
     Targets may dangle — every consumer (rerank's supersede penalty,
     prune's superseded-rule, `supersedes_chain`) joins on live `memories`
     rows. `comemory rebuild` re-materializes these edges from markdown.
   - `cross_link::extract_refs` walks the body for backtick-fenced
     `<repo>:<path>` / `<repo>:<path>:<symbol>` mentions and writes
     `ReferencesFile` / `ReferencesSymbol` rows into `edges`. Missing
     `code_symbols` rows are tolerated — `comemory index-code` fills them
     in later.
5. Best-effort git auto-sync via `git_utils`, only when
   `COMEMORY_GIT_AUTO_SYNC` is enabled.

If the SQLite mirror transaction fails, the markdown file is **kept** (it
was already written as the source of truth) and the error wraps the
markdown path with a hint to run `comemory rebuild`, which can always
reconstruct `comemory.db` from `memories/*.md`. See the README
"BYO-Vector workflow" section and `scripts/comemory-embed.sh` for the
recommended caller pattern.

## Testing

- Runner: `cargo nextest run --all-features` (alias `just test`).
- `tests/` mirrors `src/` 1:1 with a **flat, dunder-joined** layout:
  `src/<path>.rs` ↔ `tests/<dunder-path>.rs` (e.g.
  `src/store/tokenizer/split.rs` ↔ `tests/store__tokenizer__split.rs`).
  Each flat file is its own integration-test binary — there are no
  `tests/<module>.rs` shims and no nested `tests/<module>/` directories. An
  oversized test file is split into `<base>.rs` + `<base>_2.rs`.
- `tests/common/` carries shared fixtures (temp data-dir builders, gold
  memory samples).
- CLI integration tests use `assert_cmd` against the real `comemory` binary.
- Snapshot tests use `insta` (`tests/snapshots/`).
- Property tests use `proptest`.
- `.config/nextest.toml` serializes the `embedder` test group
  (`embedder|memory_index|code_index`) to `max-threads = 1` so the fastembed
  model download cannot race.

## Quality Gates

`bash scripts/check-all.sh` is the umbrella gate. It runs, in order:

```
scripts/fmt-check.sh             # cargo fmt --check
scripts/type-check.sh            # cargo check --all-targets --all-features
scripts/lint-check.sh            # cargo clippy --all-targets --all-features -- -D warnings
scripts/test-placement-check.sh  # no #[cfg(test)] mod tests in src/
scripts/no-bypass-check.sh       # no allow/unwrap/println!/unsafe-without-SAFETY/etc.
scripts/module-size-check.sh     # no src/ file > 300 code lines (blanks/comments excluded)
scripts/tests-mirror-check.sh    # every src/ file has a mirror in tests/
scripts/typos-check.sh           # typos
```

Additional gates wired into `just qa`: `scripts/deny-check.sh`
(`cargo deny check`) and `scripts/dup-check.sh`. `scripts/test-run.sh`
runs the nextest suite. A task is not "done" until `scripts/check-all.sh`
exits 0.

## Distribution

- `cargo install --path .` (build from a local checkout; not published to
  crates.io).
- `brew install Falconiere/tap/comemory` (Homebrew tap
  `Falconiere/homebrew-tap`, published by `cargo-dist`).
- Prebuilt tarballs for `aarch64-apple-darwin`,
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` attached to
  [GitHub Releases](https://github.com/Falconiere/comemory/releases).

`cargo-dist` is configured in `[workspace.metadata.dist]` in `Cargo.toml`.
PRs get a dry-run plan; only version tags (e.g. `vX.Y.Z`) publish artifacts.

Releases are driven by the **release-plz** bot (`.github/workflows/release-plz.yml`,
config `release-plz.toml`): a push to `main` opens/updates a "release PR" that bumps
the version + rewrites `CHANGELOG.md` from conventional commits; merging it pushes the
`vX.Y.Z` tag, which fires `release.yml`. release-plz owns version + changelog + tag;
cargo-dist owns build + GitHub Release + Homebrew (`git_release_enable=false`,
`publish=false` — crates.io stays off). The bot is gated behind the
`RELEASE_PLZ_ENABLED` repo variable and needs a fine-grained PAT
(`RELEASE_PLZ_TOKEN`, Contents + Pull requests read/write) so the pushed tag
triggers downstream workflows. The `just release` recipe remains a manual
fallback. See `docs/release.md`.

## Claude Code Hooks

`.claude/hooks/` is adapted from `qwick-business-app` and delegates rule
logic to the same gate scripts.

- **PreTool hooks** (`pre-tools/modules/`):
  - `bash-commands.sh` blocks `npm`/`bun`/`yarn`/`pnpm`/`pip`/`uv`/`poetry`
    (this is a Rust project), destructive commands
    (`rm -rf`, `git push --force`, `git reset --hard`, `git checkout .`,
    `chmod -R 777`), bypass flags (`--no-verify`, `--no-gpg-sign`), and
    direct `rustfmt` / `cargo fmt` / `cargo clippy` invocation outside
    `scripts/` or `just`.
  - `code-edit-rules.sh` rejects edits to `src/*.rs` that introduce
    forbidden patterns: `#[allow(...)]`, `// clippy::allow`,
    `#[cfg(test)] mod tests`, `.unwrap()`, `.expect(...)` (any, with or
    without a message), `println!`/`eprintln!`, `todo!()`/`unimplemented!()`,
    `panic!()`, or `unsafe { … }` without a nearby `// SAFETY:` comment.
    Mirrors `scripts/no-bypass-check.sh`.
  - `protected-files.sh` guards generated artifacts and config the agent
    must not edit casually.
- **PostTool hooks** (`post-tools/modules/`):
  - `auto-format.sh` re-runs `rustfmt` on touched files.
  - `gate-status.sh` records which gates are currently green for the
    session.
- **Stop hook** (`session-end.sh`) runs `fmt-check`,
  `test-placement-check`, `no-bypass-check`, and `module-size-check` at
  end-of-conversation so regressions surface immediately.

User-facing docs live under `docs/`, organized in Diátaxis tiers and indexed by
`docs/README.md`: the `docs/getting-started.md` tutorial, the task-oriented
`docs/guides/` how-tos (byo-vectors, auto-reindex, ranking-and-eval, serve-web,
prune-and-gc), the `docs/cli-reference.md` reference (every subcommand, flag, and
env var), and the `docs/architecture.md` explanation. The README is a front door
that links into them.
