# Configuration

comemory's settings are layered: built-in defaults → an optional `config.toml`
→ environment variables (the last wins). The CLI also takes the global
`--data-dir` and `--json` flags (see [CLI reference](cli-reference.md)).

## Agent integration

`comemory install claude|codex` honors `CLAUDE_CONFIG_DIR` or `CODEX_HOME`,
respectively; `--config-dir` overrides that destination. Installed hooks read
`comemory.json` in the host configuration directory and the repository's
`.claude/` or `.codex/` directory, with project values taking precedence.
See [Agent skills and hooks](guides/agent-integration.md) for the disable controls,
project-skill settings, and migration from toolu.

## Environment variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `COMEMORY_DATA_DIR` | Root data directory (`memories/` + `comemory.db`). | `~/.comemory` |
| `COMEMORY_API` | Platform API base for `comemory auth` (trailing slash stripped). Overridden by `auth login` / `auth status --api-url`. | `https://api.comemory.io` |
| `COMEMORY_API_KEY` | Optional override of the `auth.json` device secret (CI / scripting without writing the file). | unset |
| `COMEMORY_INDEXING_AUTO_REINDEX` | `lazy` \| `hook` \| `off` — the search-time code-index refresh. Installed git hooks, and the `sync --action auto` pass they fire, run in every mode. See [Keep the code index fresh](guides/auto-reindex.md). | `lazy` |
| `COMEMORY_RETRIEVAL_TOP_K` | Results returned by the hybrid router (also the default page size for `search` / `search-code` / `context`). | `12` |
| `COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW` | Maximum depth pagination can reach into the ranked list; `has_more` is forced false at this ceiling. Validated `> 0`. | `200` |
| `COMEMORY_RETRIEVAL_MEMORY_THRESHOLD` | Minimum cosine similarity for the memory table. | `0.55` |
| `COMEMORY_RETRIEVAL_CODE_THRESHOLD` | Minimum cosine similarity for the code table (ANN leg of `search-code`, range `[0.0, 1.0]`). | `0.50` |
| `COMEMORY_RETRIEVAL_RRF_K` | RRF fusion constant for hybrid scoring. | `60.0` |
| `COMEMORY_RETRIEVAL_GRAPH_HOPS` | Maximum hop depth of the graph-expansion leg — the `edges` walk seeded from the provisional top hits of memory `search`. Validated `≤ 4`; `0` disables the leg. | `2` |
| `COMEMORY_RETRIEVAL_GRAPH_SEEDS` | How many provisional top hits seed that walk. Validated `≥ 1`. | `8` |
| `COMEMORY_RETRIEVAL_BM25_WEIGHTS` | `"body,tags"` BM25 column weights for `memory_fts` (both finite ≥ 0, at least one > 0). | `1.0,3.0` |
| `COMEMORY_RETRIEVAL_CODE_BM25_WEIGHTS` | `"symbol,snippet,path_tokens"` BM25 column weights for `code_fts` (all finite ≥ 0, at least one > 0). | `2.0,1.0,1.5` |
| `COMEMORY_LEARNING_RETENTION_DAYS` | `comemory gc` retention window (days) for raw `retrieval_log`, `feedback_events` and `activity_log` rows; aggregated `feedback` counters and mined `query_expansions` never expire. | `90` |
| `COMEMORY_ACTOR` | Caller label a terminal run reports as the `actor` of the rows it records in the activity feed (`GET /api/v1/activity`). Trimmed; blank means no actor. Env-only: it names the wrapper or agent invoking one process, which a shared config file cannot. | unset |
| `COMEMORY_ACTIVITY_ENABLED` | `false` turns the activity feed's recording off entirely; the routes still serve what is already stored. | `true` |
| `COMEMORY_ACTIVITY_SUMMARIES` | `false` records each run without its per-command summary, so no query text or memory title is persisted. | `true` |
| `COMEMORY_ACTIVITY_STREAM_POLL_MS` | How often `GET /api/v1/activity/events` polls for rows above its cursor. Validated `> 0`. | `500` |
| `COMEMORY_RELEASES_URL` | Test hook: the release base `comemory upgrade` and `install.sh` resolve `latest` and download assets from; the suite points it at a loopback stand-in. Not a user knob. | `https://github.com/Falconiere/comemory/releases` |
| `COMEMORY_TUNE_MIN_GOLDEN` | Test hook lowering `comemory tune` / `comemory bandit` minimum-golden-pairs floor; not a tuning knob. | `10` |
| `COMEMORY_REINFORCE_SEARCH_EDIT_DAYS` | Lookback (days) for search→edit auto-reinforcement: a memory that appeared on a recent `search`/`context` page earns `auto_search_edit` provenance when a referenced file is touched. Must be `≥ 1`. | `7` |
| `COMEMORY_GIT_AUTO_SYNC` | `true`/`1` to enable best-effort git commit + push after a save. Also settable as `[git] auto_sync` (with `[git] remote`) in `config.toml` — the console's `PATCH /api/v1/memory-stores/default` writes those keys; the env var still wins. Distinct from cloud `[sync]`. | `false` |
| `COMEMORY_API_KEY` | Overrides the device secret in `$COMEMORY_DATA_DIR/auth.json` for platform sync/auth clients. | unset |
| `COMEMORY_DAEMON_SUPERVISOR` | Force the required sync daemon's backend: `launchd` \| `systemd` \| `process` \| `external`. `process` is what a container or a headless host with no session bus falls back to on its own; `external` verifies only and never starts anything (bring your own supervisor). Unset picks the OS-native backend. | unset |
| `COMEMORY_SYNC_DAEMON` | Test/CI hook: `0` skips every command's daemon preflight and keeps the pre-#257 in-process pass/pull paths. Not a user knob — leaving the required daemon out is not a supported mode. | unset |
| `COMEMORY_EMBED_HINT` | Free-form identifier of the embedder you used (e.g. `ollama:nomic-embed-text`). Surfaced by `comemory doctor`; never consumed as a switch. | unset |
| `COMEMORY_EMBED_CMD` | Embed command (`sh -c <cmd>`, text on stdin, `{"embedding":[..]}` on stdout) used by `comemory serve`'s `POST /api/v1/doctor/reembed`. `serve --embed-cmd` overrides it. | unset |
| `COMEMORY_RANK_DECAY` | ACT-R decay exponent `d` in `ln(n) − d·ln(days+1)`. Must be ≥ 0. Higher → older memories decay faster. | `0.5` |
| `COMEMORY_RANK_PRIOR_CLAMP` | `"lo,hi"` bounds applied to the activation, feedback, and quality boost multipliers (the fixed `0.2` supersede penalty bypasses the clamp). Both finite; lo > 0, lo ≤ hi. | `0.5,2.0` |
| `COMEMORY_RANK_MMR_LAMBDA` | MMR relevance-vs-diversity trade-off in `[0.0, 1.0]`. `1.0` = pure relevance; `0.0` = pure diversity. | `0.7` |
| `COMEMORY_RANK_NEAR_DUP_HAMMING` | SimHash Hamming radius for near-dup detection (save-time advisory + diversify collapse). Must be ≤ 64. | `8` |
| `COMEMORY_PRUNE_MIN_ACTIVATION` | Activation floor (ACT-R scale) below which a memory is prune-eligible. | `-2.0` |
| `COMEMORY_PRUNE_MIN_FEEDBACK` | Beta-feedback ceiling (range `[0.0, 1.0]`) at or below which a memory is prune-eligible. | `0.25` |
| `COMEMORY_PRUNE_BELOW_QUALITY` | Quality threshold (1..=5); memories at or below this value are prune candidates. | `2` |
| `COMEMORY_PRUNE_SUPERSEDED_GRACE_DAYS` | Grace window (days) before a superseded-and-never-accessed memory becomes prune-eligible. | `7` |
| `COMEMORY_SKIP_MIGRATION_BACKUP` | Truthy (`1`/`true`) skips the automatic pre-migration `VACUUM INTO` snapshot taken before any pending schema migration — a failed snapshot only refuses the upgrade when a pending migration is destructive, and merely warns otherwise. | `false` |

The ranking knobs (`COMEMORY_RANK_*`, `COMEMORY_RETRIEVAL_*`) are explained in
[Measure and tune ranking](guides/ranking-and-eval.md); the `COMEMORY_PRUNE_*`
floors in [Prune, rebuild, and gc](guides/prune-and-gc.md);
`COMEMORY_SKIP_MIGRATION_BACKUP` in [Upgrading comemory](guides/upgrading.md).
`COMEMORY_API` / `COMEMORY_API_KEY` back device login — see
[CLI reference: auth](cli-reference.md#comemory-auth) and README § Cloud auth.

## Config-file-only knobs

Set these in `config.toml`; they have **no** environment override.

| Knob | Purpose | Default |
|------|---------|---------|
| `prune.trash_retention_days` | Days a soft-deleted memory stays in `memories/.trash/` before `comemory gc` reaps it. Must be ≥ 1. Editable live through `PUT /api/v1/gc/policy`. | `30` |
| `indexing.auto_reindex_threshold_ms` | Debounce (ms) that suppresses spawning bursts of `lazy` auto-reindex processes during rapid successive searches: a new background `index-code` is only spawned if at least this long elapsed since the last trigger. | `200` |
| `tune.rrf_k_grid` / `tune.decay_grid` / `tune.mmr_lambda_grid` / `tune.bm25_grid` | The `[tune]` grid-search axes consumed by `comemory tune` and `comemory bandit`. | — |
| `bandit.enabled` | When `false`, `comemory bandit --apply` refuses; report still works. | `true` |
| `reinforce.search_edit_days` | File overlay for the search→edit lookback (same as `COMEMORY_REINFORCE_SEARCH_EDIT_DAYS`). | `7` |
| `sync.push_on_save` | Push the outbox inline after `save` / `delete`, so continuous sync needs no resident process. Env: `COMEMORY_SYNC_PUSH_ON_SAVE`. | `true` |
| `sync.push_on_save_timeout` | Budget for that inline push only — far below the 30s every other platform call takes, because a save must return on a captive-portal network. Validated `> 0`; disable the hook with `push_on_save = false` rather than a zero budget. Env: `COMEMORY_SYNC_PUSH_ON_SAVE_TIMEOUT`. | `"2s"` |
| `sync.after_save` | **Deprecated, ignored.** Superseded by `sync.push_on_save`. | `false` |
| `sync.pull_before_context_after` | **Deprecated, ignored.** Use `comemory watch` for live pulls. | `""` |
| `sync.daemon_interval` | Sleep between the required resident coordinator's reconciliation ticks (#257; every ordinary command's preflight ensures one). Re-read every tick, so a change takes effect on the next one with no restart. A tick whose pass ends with `more` runs the next pass at once instead of sleeping. | `"5s"` |
| `sync.request_timeout` | Budget for every sync request except the inline push after a save (which keeps `push_on_save_timeout`). A request that times out, cannot connect or answers `5xx` is retried at most twice within the pass, then the key backs off. | `"30s"` |
| `sync.pass_budget` | Time after which an unattended pass (a git or agent hook, the daemon, `comemory watch`) starts no new batch and reports `more: true`, so the next pass runs at once and a busy machine keeps yielding. A person's `comemory sync` has no budget. | `"30s"` |
| `sync.max_request_bytes` | Largest serialized push request on `replica-v1`. A batch stops before it would exceed this; a single operation larger than it crosses through the staged `stage`/`activate` upload. Lower it behind a proxy with a small request-body limit. | `4194304` |
| `sync.verify_every` | Interval hint for `comemory sync --action verify` and the daemon. | `"7d"` |
| `sync.skip_repos` | Globs over the trimmed, lowercased local `repo` label; a match keeps that memory and code index local in addition to the platform's repository approval policy. An invalid glob fails at config load. | `[]` |
| `sync.code_index` | Push the code index of every indexed repo alongside memories: file paths and blob OIDs, symbol names with kinds and line ranges, the `imports` and `co_changed` edges — never source text. On by default so the console's code graph fills in right after `comemory auth login`; `false` keeps every index on this machine. Env: `COMEMORY_SYNC_CODE_INDEX`. | `true` |
| `sync.allowlist_ttl`, `sync.repos`, `sync.default_workspace` | **Deprecated, parsed and ignored**, each with a warning naming it. Kept declared for one release because `[sync]` is `deny_unknown_fields`, so deleting them outright would stop every existing `config.toml` that sets them from loading at all. Remove them. | — |
| `embed.model` | Model id recorded for sync vector import compatibility. | `""` |

## Vector dimensions (not configurable)

The memory and code vector dims (`1024` and `768`) are baked into the
`memory_vec` / `code_vec` `vec0` DDL (`migrations/0002_v2_tables.sql`) at
migration time and are **not** env-configurable: a divergent value would
disagree with the vtab and surface as `VecDimMismatch` at first insert. Change
the DDL literal if you need a different dim. See
[Bring your own vectors](guides/byo-vectors.md).

## Pagination envelope

Data-returning commands accept `--limit` / `--offset` (retrieval commands —
`search`, `search-code`, `context` — alias `--limit` to `--k`). With `--json`
they emit a shared `Page` envelope:

```json
{ "items": [ ], "limit": 50, "offset": 0, "total": 123, "has_more": true }
```

- `limit: 0` is the sentinel for "all" (no slicing).
- `total` may be `null` when not counted; for retrieval commands it is the
  in-window ranked count, not a global match count.
- `has_more` is `false` at the end of the window. Ranked retrieval pages stably
  within `COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW` — see the
  [architecture notes](architecture.md) on the retrieval pipeline.
