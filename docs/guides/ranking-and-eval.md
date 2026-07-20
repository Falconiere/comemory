# Ranking and eval

Retrieval quality is something you measure, not guess at. `comemory` ships a
deterministic learning loop you drive from the command line:

```
measure  →  distill  →  tune / bandit  →  re-measure
 eval        mine      tune | bandit       eval
```

Auto-reinforcement also runs inside `index-code`: commit co-activation (and
search→edit when a memory was recently returned by `search`/`context`) mint
implicit `used` feedback without touching the golden set.

This guide walks that loop end to end: build a golden set, score it with
`comemory eval`, fold harvested reformulations into the lexical fallback with
`comemory mine`, grid-search the ranking knobs with `comemory tune`, then
re-measure to confirm the change paid off.

New to the tool? Start with [getting started](../getting-started.md). For the
full mechanics behind the blend, see [architecture](../architecture.md).

---

## Read a `search --json` hit

`comemory search --json` returns a `Page` envelope (`{ items, limit, offset,
total, has_more }`) whose `items` are hits. Each hit carries the ranking
contract plus navigation metadata so a caller can both inspect the score and
open the result:

```jsonc
{
  "memory_id": "a1b2c3d4",
  "score": 0.83,
  "source": "hybrid",
  "tier": 1,
  "score_parts": {
    "rrf": 1.0, "activation": 1.2, "feedback": 1.0,
    "quality": 1.1, "supersede": 1.0, "final_score": 0.83
  },
  "path": "/Users/me/.comemory/memories/a1b2c3d4-postgres-analytics.md",
  "title": "Use Postgres for analytics rollups",
  "repo": "comemory",
  "kind": "decision",
  "tags": ["database", "postgres"],
  "references": { "symbols": [], "files": [] }
}
```

The navigation fields are **additive** — `score_parts` is unchanged, and the
existing `memory_id` / `score` / `source` / `tier` / `superseded_by` fields
keep their meaning. The new fields are:

| Field | Meaning |
|-------|---------|
| `path` | Absolute path to the memory's markdown file (open it, or `cat` it). Empty if the row's metadata couldn't be resolved (raced soft-delete / rebuild). |
| `title` | First non-empty line of the body — a human-readable label. Empty when the body is blank. |
| `repo` | Repo the memory belongs to; omitted when unset. |
| `kind` | `decision` \| `bug` \| `convention` \| `discovery` \| `pattern` \| `note`. Empty when metadata couldn't be resolved. |
| `tags` | Tag list from the frontmatter. |
| `references` | Code references harvested from the body: `{ symbols, files }`. |

`score_parts` remains the stable explainability contract (`comemory tune`
reads it); see [architecture](../architecture.md) for what each multiplier
means.

---

## Build a golden set

A golden set is the ground truth eval scores against: each entry is a query
plus the memory ids a correct retrieval should surface. It is plain YAML — a
list of `query` / `relevant` pairs:

```yaml
- query: postgres analytics decision
  relevant: [a1b2c3d4, e5f6a7b8]
- query: frontmatter parsing
  relevant: [9c8d7e6f]
  repo: comemory          # optional — replays the originating --repo filter
  kind: discovery         # optional — replays the originating --kind filter
```

The memory ids are the 8-hex ids printed by `comemory search` (and stored as
`id:` in each markdown frontmatter). `repo` and `kind` are optional; when
present, eval replays them as filters, because the same query under different
filters is a different retrieval problem.

You don't have to hand-write everything. `comemory` harvests golden pairs from
your recorded feedback automatically: every `(query, repo, kind)` you marked
with `comemory feedback <query_id> --used <ids>` becomes a pair. Hand-written
file pairs and harvested pairs merge, and the file wins on a duplicate key.
Implicit auto-reinforcement feedback (provenance `auto_coactivation`) is
**excluded** from the harvest — only real queries with a `retrieval_log` row
qualify, so the synthetic co-activation rewards never leak into ground truth.

Use `--golden-only` to score a file in isolation and skip the harvest entirely.

---

## Measure

`comemory eval` scores the golden set against the live retrieval pipeline and
reports **recall@k** (did the relevant ids land in the top *k*?) and **MRR**
(how high did the first relevant id rank?). Tracking is off during eval, so
scoring never pollutes the feedback you're measuring against.

```bash
# Score against feedback-harvested golden pairs (recall@3, the default)
comemory eval

# Merge a hand-written file over the harvest (file wins on duplicate query)
comemory eval --golden golden.yaml

# File only, recall@5, machine-readable report
comemory eval --golden golden.yaml --golden-only --k 5 --json
```

`--k` sets the recall cut (default `3`). This number is your baseline — write
it down before you change anything.

---

## Mine reformulations

`comemory mine` distills query reformulations from the query log into
`query_expansions` — the **tier-4 lexical fallback** the router reaches for only
when stricter tiers (strict → word-OR → subtoken-OR) find nothing. When a user
who searched "fe" later succeeded with "frontmatter", that reformulation is
mined into an expansion so the next bare "fe" can recover.

```bash
# Report the mined expansion mappings without touching retrieval
comemory mine

# Rebuild the query_expansions table from the current retrieval_log
comemory mine --apply

# Machine-readable report
comemory mine --json
```

Run `comemory mine --apply` to commit the rebuild; without it the command only
reports. Then re-run `comemory eval` to confirm the new expansions lifted
recall on the queries that previously fell through.

---

## Tune the knobs

`comemory tune` runs a deterministic grid search over the ranking knobs against
the golden set and reports the winning configuration. With `--apply` it writes
the winner into `config.toml` — but **only when the winner strictly beats your
current config**, so a tie never churns the file.

```bash
# Grid-search the configured [tune] grid against the merged golden set (report)
comemory tune

# File-only golden set, recall@5, machine-readable report
comemory tune --golden golden.yaml --golden-only --k 5 --json

# Write the winning knobs into config.toml (atomic; comments are dropped)
comemory tune --golden golden.yaml --apply
```

### Online bandit (same knobs, one sample)

`comemory bandit` Thompson-samples one arm from the same `[tune]` grid,
confirms it against the golden set, and with `--apply` writes `config.toml`
only when that sample strictly beats baseline (same predicate as `tune`).
Set `[bandit] enabled = false` to keep report-only mode from applying.

```bash
comemory bandit
comemory bandit --golden golden.yaml --apply --json
```

The search space is the `[tune]` grid in `config.toml`: `tune.rrf_k_grid`,
`tune.decay_grid`, `tune.mmr_lambda_grid`, and `tune.bm25_grid`. These grid
knobs are **file-only** — there is no environment override for them. The
default grid is 81 configurations.

`--apply` re-renders `config.toml` from parsed TOML via an atomic rename, so any
comments in the existing file are dropped. Commit the result so the tuned blend
travels with the repo.

---

## Knob reference

The knobs `tune` searches are the same ranking parameters you can override per
run via environment variables. Compact view (defaults and full descriptions
live in the [configuration env table](../configuration.md)):

| Knob | Env override | Grid knob | What it moves |
|------|--------------|-----------|---------------|
| ACT-R decay | `COMEMORY_RANK_DECAY` | `tune.decay_grid` | how fast older memories fade |
| Prior clamp | `COMEMORY_RANK_PRIOR_CLAMP` | — | `lo,hi` bounds on activation/feedback/quality multipliers |
| MMR lambda | `COMEMORY_RANK_MMR_LAMBDA` | `tune.mmr_lambda_grid` | relevance ↔ diversity in `[0,1]` |
| RRF constant | `COMEMORY_RETRIEVAL_RRF_K` | `tune.rrf_k_grid` | fusion constant blending FTS5 + vector ranks |
| Memory BM25 weights | `COMEMORY_RETRIEVAL_BM25_WEIGHTS` | `tune.bm25_grid` | `body,tags` column weights |
| Code BM25 weights | `COMEMORY_RETRIEVAL_CODE_BM25_WEIGHTS` | — | `symbol,snippet,path_tokens` column weights |
| Memory cosine floor | `COMEMORY_RETRIEVAL_MEMORY_THRESHOLD` | — | min similarity for the memory vector leg |
| Code cosine floor | `COMEMORY_RETRIEVAL_CODE_THRESHOLD` | — | min similarity for the code vector leg |

The `[tune]` grid knobs are file-only (no env override); the env variables let
you probe a single setting by hand before committing it to the grid.

---

## Re-measure

Close the loop. After `comemory mine --apply` and `comemory tune --apply`, run
`comemory eval` again with the same `--k` and golden set you measured at the
start:

```bash
comemory eval --golden golden.yaml --k 3
```

Compare recall@k and MRR against the baseline you recorded. If they rose, the
change earned its place — commit `config.toml`. If they didn't, revert: the
loop is deterministic, so the same inputs always reproduce the same scores.

---

## See also

- [CLI reference](../cli-reference.md) — every flag for `eval`, `mine`, and
  `tune`.
- [Configuration](../configuration.md) — the full environment-variable table.
- [Getting started](../getting-started.md) — install, save, search, index.
- [Architecture](../architecture.md) — the RRF fusion, rerank priors, and
  lexical fallback ladder behind these knobs.
