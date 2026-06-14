# Changelog

## 0.8.4 — 2026-06-14 (test-confidence + lang-quality migration)

Internal test infrastructure, CI, and conventions. The published binary is
functionally unchanged from 0.8.3 — the only `src/` change is a panic-free
regex in `graph::cross_link` that preserves behavior.

### Added
- **Test-confidence program.** Measure-first tooling —
  `scripts/{coverage-check,mutation-check,eval-check}.sh` (a cargo-llvm-cov
  line-coverage floor, diff-scoped/nightly cargo-mutants, lexical eval) with
  `just coverage|mutation|eval` and committed baselines (85% line coverage,
  the mutant-survivor list), plus real-data tests that kill 12 of 14 baseline
  survivors in graph/memory/output.
- **CI gates.** `test.yml` (check-all → coverage floor → eval) and
  `mutation.yml` (diff-scoped report-only on PRs + a nightly full run).

### Changed
- **Adopted the lang-quality Rust standard repo-wide.** Flattened the nested
  test layout to flat dunder mirrors (`tests/<a>__<b>.rs`, 100 binaries) with
  every test file ≤300 code lines; re-pointed the gate scripts
  (`tests-mirror-check` flat mapping, `module-size-check` 300-code-line `src/`
  cap, `no-bypass-check` full `.expect()` ban) and `CLAUDE.md`. Limits set via
  `.claude/claudness.config.json`.
- Purged the one `src/` `.expect()` (the `cross_link` regex now compiles into a
  non-panicking `Lazy<Option<Regex>>`); no behavior change.

## 0.8.3 — 2026-06-13 (docs accuracy)

Docs only. The published binary is unchanged from 0.8.0–0.8.2.

### Fixed
- **Documentation audit.** Corrected stale/misleading docs: `architecture.md`
  §8 now states the `lazy` auto-reindex mode is unwired (behaves like `off`)
  and drops the false `comemory doctor` staleness-report claim; added the v7
  `serve`/`graph` subsystems and `repo_marker.root_path` to the component
  map; removed two dead `docs/superpowers/` links. Rewrote `docs/bench/`
  (it described a removed v0.1 LanceDB/nomic harness). Fixed `CLAUDE.md`
  (`[workspace.metadata.dist]`, dropped the non-existent `auto-lint.sh`
  hook, `just qa` + machete, unpublished `cargo install comemory` →
  `cargo install --path .`). `README.md` install-hooks wording + a measured
  v0.8 binary-size row.

## 0.8.2 — 2026-06-13 (remove Claude Code plugin)

Repo tooling only. The published binary is unchanged from 0.8.0/0.8.1.

### Removed
- **Claude Code plugin** (`integrations/claude-code/`) and its
  `just claude-plugin-*` recipes. comemory is a standalone CLI again and
  carries no editor/agent integration. The 0.8.1 plugin remains available
  at the `v0.8.1` tag for anyone who wants it.

## 0.8.1 — 2026-06-12 (Claude Code plugin)

Repo tooling only. The published binary is unchanged from 0.8.0 — the
plugin lives entirely under `integrations/` and is not compiled into the
crate.

### Added
- **Claude Code plugin** (`integrations/claude-code/`) wrapping the
  `comemory` CLI: a single `comemory.sh` wrapper (sole authority for
  git-repo scoping + missing-binary fail-soft), a SessionStart auto-recall
  hook, `save` / `recall` / `search-code` skills, an `uninstall.sh` with a
  typed-confirmation data purge, and `bats` tests against the real binary.
  `just claude-plugin-*` recipes install/remove/test it.

### Changed
- Documented the plugin in `docs/architecture.md`, `docs/cli-reference.md`,
  and the README.

## 0.8.0 — 2026-06-12 (edition 2024 + leaner release builds)

Build, packaging, and toolchain hardening. No CLI behavior changes.

### Changed
- **`[profile.dist]` now uses fat LTO** (was thin), so prebuilt release
  tarballs and the Homebrew bottle match local `cargo build --release`
  codegen quality (`codegen-units = 1`, `strip = "symbols"`,
  `panic = "abort"`, all inherited from `[profile.release]`). Trade-off:
  release CI is slower per target.
- Migrated the crate to **Rust edition 2024** (`rust-version = "1.95"`
  clears the edition's 1.85 rustc floor). Mechanical migration only:
  import reordering, let-chains, and `unsafe { … }` around
  `std::env::set_var/remove_var` in tests as edition 2024 requires.

### Removed
- **Dropped the `x86_64-apple-darwin` (Intel macOS) prebuilt target.**
  Supported prebuilt targets are now `aarch64-apple-darwin`,
  `x86_64-unknown-linux-gnu`, and `aarch64-unknown-linux-gnu`. Intel-Mac
  users install from source via `cargo install --path .`.

## 0.7.0 — 2026-06-12 (interactive web viewer + editor)

Adds `comemory serve`: a loopback-only web app for exploring the code
graph and viewing, editing, and saving indexed source files — a live
complement to the static `graph --format html` export, served from a
single binary with no Node toolchain or network at runtime.

### Added
- **`comemory serve`.** An axum server bound to `127.0.0.1` on an
  ephemeral port (override with `--port`), hosting a React/Vite/Tailwind
  single-page app embedded in the binary via `rust-embed`. The graph
  payload reuses a shared `cli::graph::build_code_graph`, so the served
  and static renderers cannot drift. The access URL (carrying a
  per-session token) prints via the output module — visible without
  `RUST_LOG`, machine-readable under `--json`. `--open` launches a
  browser; `--read-only` disables writes (PUT → 405).
- **In-browser editing.** CodeMirror 6 editor with `If-Match` optimistic
  concurrency keyed on the git blob OID, an editable-extension allowlist,
  a 5 MiB size cap, and atomic temp-file writes that preserve the original
  file's permissions.
- **`--root <repo>=<path>`** on `serve` overrides the persisted working-tree
  root (and covers pre-v7 repos whose root is `NULL`).

### Schema
- **v7 migration:** a nullable `repo_marker.root_path` column persisting the
  absolute working-tree root at index time (`canonicalize(--path)`) — the
  exact base `code_symbols.path` is relative to, so `serve` can resolve
  `file:<repo>:<path>` ids back to on-disk files. Idempotent; pre-v7 repos
  read `NULL` and rely on `--root`.

### Security
- Loopback-only bind, a 256-bit per-session token required on `/` and
  `/api/*` (compared in constant time), a `Host`-header guard
  (DNS-rebinding defense), default-deny CORS, and a single
  canonicalize-and-contain chokepoint (`id_to_abs_path`) that rejects
  `..`/absolute/symlink escapes. The token is set as an HttpOnly
  `SameSite=Strict` cookie and stripped from the URL after first load
  (`history.replaceState` + `Referrer-Policy: no-referrer`) so it does not
  persist in history or `Referer`. The raw embedded shell (with its
  `__COMEMORY_TOKEN__` sentinel) is never served — `GET /index.html`
  redirects to the token-substituted `/`.

### Internal
- Embedded frontend assets are `rust-embed` gzip-compressed
  (`compression` feature) to offset the bundle's contribution to binary
  size.

## 0.6.0 — 2026-06-12 (WebGL graph viewer)

Replaces the `comemory graph --format html` viewer with a WebGL-rendered
stack for smooth interaction on larger graphs. No CLI, schema, or data
change — same JSON payload inlined into the page.

### Changed
- **HTML graph viewer → sigma.js v3 + graphology + ForceAtlas2.** WebGL
  rendering (pan/zoom/hover stay at 60fps with headroom to ~100k nodes)
  replaces the cytoscape.js Canvas viewer. ForceAtlas2 runs on
  `requestAnimationFrame` so layout animates without blocking the main
  thread. WebGL2 is chosen over WebGPU libraries (still browser-gated)
  and over GPU-sim libraries that trade away styling/labels/interaction.
  The `imports`/`co_changed` toggles, click-to-focus neighborhood
  dimming, and PageRank-scaled node sizing are preserved. The Rust
  `to_html` path is unchanged — the graph JSON is still inlined at
  `__GRAPH_DATA__` with the same `</`, U+2028/U+2029 escaping.
- `co_changed` edges now render solid orange rather than dashed (sigma
  core has no dashed-edge program); layout force-animates then settles
  over a fixed frame budget instead of laying out instantly.

### Security
- The three CDN `<script>` tags now carry Subresource Integrity
  (`integrity` sha384 + `crossorigin="anonymous"`): a tampered or cached
  bundle fails the integrity check instead of executing arbitrary JS.

### Fixed
- ForceAtlas2 is loaded from the `graphology-library` UMD bundle
  (`graphologyLibrary.layoutForceAtlas2`); the standalone
  `graphology-layout-forceatlas2` package ships no UMD build, so its
  `dist/` URL 404'd and the viewer fell through to the error overlay.
- The viewer surfaces a clear `#err` overlay (and logs via
  `console.error`) on a CDN-load failure or a mid-render Sigma/WebGL
  init error, instead of leaving a blank canvas.

## 0.5.0 — 2026-06-12 (code-graph export)

Surfaces the code-connection graph mined by `index-code` (the `imports` +
`co_changed` edges, with file nodes weighted by the materialized PageRank)
as a first-class export. Pure read over `comemory.db` — no re-indexing,
no schema change.

### Added
- **`comemory graph`** — export the file-level code-connection graph as
  machine-readable JSON, Graphviz DOT (`dot -Tsvg`), or an interactive
  HTML viewer (cytoscape.js, loaded from a CDN). Flags: `--repo` (scope
  to one repo label, gating both edge endpoints in SQL), `--rel`
  (`all` | `imports` | `co-changed`), `--format` (`json` | `dot` |
  `html`), and `--min-weight` (drop weak `co_changed` links; `imports`
  are untouched). The global `--json` flag forces JSON output.
  Edge endpoints with no `code_symbols` row (stale edges to deleted
  files) still render as zero-rank nodes so edges are never orphaned.
  DOT labels and the inlined HTML JSON payload are escaped (`\`, `"`,
  newlines for DOT; `</`, U+2028/U+2029 for HTML) so paths can never
  break the output.

### Docs
- `docs/cli-reference.md` gains a generated `## comemory graph` section.

## 0.4.0 — 2026-06-11 (M3 code graph + code-aware retrieval)

The code layer becomes a graph. A v6 migration adds code-graph edges,
PageRank/chunk columns, and a `code_feedback` table; the database
auto-migrates v5 → v6 on first open and markdown files are untouched.

### Added
- **`comemory search-code`** — ranked code search (weighted BM25 + a
  thresholded ANN leg fused with RRF, chunk→parent coalesce) reranked
  by four graph priors (PageRank, recency, working-set affinity,
  feedback), with per-query logging and a `query_id` for feedback.
- **Code-graph edges**: `co_changed` (mined from git history with a
  sliding window, mega-commit guard, and resumable cursor) and
  `imports` (conservative per-language import resolution for rust /
  typescript / javascript / python / go).
- **Deterministic weighted PageRank** over the code graph, materialized
  onto `code_symbols.rank_score` by `comemory index-code`.
- **cAST chunking**: oversized symbols split into child rows at AST
  boundaries so large definitions stay retrievable.
- `comemory feedback` accepts code targets and records per-query
  provenance for code results.
- `comemory context` ranks referenced code symbols by graph priors.
- `comemory ast` / the extractor now capture `pub` / `export`-modified
  definitions.
- Config: code BM25 weights and `COMEMORY_RETRIEVAL_CODE_THRESHOLD`
  (re-consumed for the `search-code` ANN leg), plus configurable
  rank / prune / tune constants and matching `COMEMORY_*` env vars.

### Changed
- `comemory eval` replays each golden query's originating repo / kind
  filters so measurement matches production retrieval.
- The learning loop logs search filters and source; mining ignores
  code searches so reformulation expansions stay memory-scoped.
- `comemory index-code` mines co-change + imports and materializes
  PageRank as part of the indexing pass.

### Fixed
- Retrieval skips working-set discovery when the context query returns
  no hits.
- Feedback resolves chunk ids to their parent symbol identity.
- PageRank edge load is ordered by logical graph keys for determinism.

## 0.3.0 — 2026-06-11 (Rank-blend retrieval + learning loop)

Two milestones in one release: M1 (rank-blend core, PR #4) and M2
(learning loop, PR #5). The database auto-migrates v2/v3/v4 → v5 on
first open; markdown files are untouched.

### Breaking
- `comemory feedback` requires the `q-<yyyymmdd>-<8hex>` query id
  printed by `comemory search` / `comemory context`; free-form ids are
  rejected.
- `score_parts.rrf` in `--json` output is now max-normalized relevance
  in `[0, 1]` (pool max → 1.0), no longer the raw fused score. The
  product invariant `final_score == rrf × activation × feedback ×
  quality × supersede` still holds.
- `comemory prune` reports by default; pass `--apply` to soft-delete.
- The unused `search_stats` table is dropped and the unconsumed
  `COMEMORY_RETRIEVAL_CODE_THRESHOLD` knob is removed.
- `comemory gc` now loads the layered config and errors on invalid
  values, like every other subcommand.

### Added
- **Learning loop**: every search logs to `retrieval_log` and emits a
  `query_id`; `comemory feedback <query_id>` records per-query
  provenance in `feedback_events`.
- `comemory eval` — recall@k / MRR against golden pairs harvested from
  feedback and/or a `--golden` YAML file (runs with tracking off, so
  measurement never pollutes its own signal).
- `comemory mine` — mines failed→reworded query pairs into
  `query_expansions`; the lexical ladder gains a learned-expansion
  tier (support ≥ 2, ≤ 2 expansions per term), surfaced via the new
  `tier` field.
- `comemory tune` — deterministic 81-point grid search over rrf_k,
  decay, MMR lambda, and BM25 weights; `--apply` rewrites config.toml
  atomically only on strict improvement (requires ≥ 10 golden pairs).
- `comemory search --kind` filters results to one memory kind.
- `comemory save --supersedes` records supersession; superseded
  memories rank with a 0.2 penalty and prune respects a 7-day grace.
- Save-time near-duplicate warning (64-bit SimHash, Hamming ≤ 8).
- Config: `[retrieval] bm25_weights` (body, tags), `[rank]` decay /
  prior_clamp / mmr_lambda, `[prune]` learning_retention_days, plus
  matching `COMEMORY_*` env vars.

### Changed
- Custom FTS5 `identifier` tokenizer: camelCase / snake_case / digit
  splitting with colocated whole tokens and diacritic folding —
  `VecDimMismatch` and "dim mismatch" reach each other.
- Retrieval is two-stage: weighted-BM25/ANN candidates (tiered
  relaxation ladder) → deterministic rerank (ACT-R activation,
  Beta-smoothed feedback, quality, supersede priors on a normalized
  relevance scale) → SimHash near-dup collapse + MMR diversity.
- `comemory rebuild` preserves learning state (feedback counters,
  events, query log, mined expansions) alongside the code index.
- `comemory gc` evicts learning telemetry older than 90 days
  (configurable); counters and expansions never expire.

## 0.2.0-rc.1 — 2026-06-09 (Pre-release dry-run)

Pre-release exercising the cargo-dist release pipeline before the
final 0.2.0 cut. No source changes vs. 0.2.0. Pre-release tag does
not update the Homebrew tap.

## 0.2.0 — 2026-06-09 (Lightweight refactor)

### Breaking
- Dropped `comemory serve` (axum web UI).
- Dropped the in-process embedder. Embedding is now the caller's
  responsibility; pass vectors via `--vector` or `--vector-stdin`.
- `~/.comemory/lancedb/` and `~/.comemory/kuzu/` directories are
  ignored. Run `comemory rebuild` to populate `~/.comemory/comemory.db`
  from `memories/*.md`.
- `--lang` on `comemory ast` now accepts only `rust`, `typescript`,
  `javascript`, `python`, `go`.

### Added
- `comemory ingest-code` reads pre-embedded JSONL into `code_symbols`
  and `code_vec`.
- `comemory rebuild` drops and reconstructs `comemory.db` from
  markdown.
- `scripts/comemory-embed.sh` — sample Ollama wrapper for the BYO
  contract.

### Changed
- Single `~/.comemory/comemory.db` SQLite file backs all storage
  (memories, FTS5, sqlite-vec, edges, stats).
- Release binary size: 117 MB → ~8 MB (after dropping the in-process
  embedder/lancedb/kuzu and trimming `ast-grep-language` to the
  rust/typescript/javascript/python/go tree-sitter parsers).
