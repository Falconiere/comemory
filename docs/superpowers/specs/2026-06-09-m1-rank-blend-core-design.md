# M1 — Rank-Blend Core: Design

**Date:** 2026-06-09
**Status:** Approved design, pending implementation plan
**North star:** recall quality — the right memory or code symbol surfaces at the right moment, with deterministic, explainable scoring.

## Background

comemory's retrieval today is RRF fusion of FTS5 (default ordering, no BM25
weighting) and optional `sqlite-vec` KNN. Feedback (`used`/`irrelevant`
counts) is recorded but never influences ranking. SimHash exists but is
unused at query and save time. Prune's low-value detection relies mainly on
calendar age. Identifier-style queries (`VecDimMismatch`) miss memories
written in prose ("dim mismatch").

A research pass over current agent-memory and code-retrieval systems
(Generative Agents, Zep/Graphiti, HippoRAG, mem0, Aider repo-map, Sourcegraph
Cody, cAST, ACT-R, click models, judgment-list tuning) identified a set of
LLM-free, pure SQL+Rust techniques. CodeRAG-Bench confirms BM25 is
competitive with embeddings for code search, validating the lexical-max
stance.

### Decided constraints

- **Lexical-max only.** No in-process LLM, no in-process embedder, no
  network code. Vectors stay BYO (`--vector` / `--vector-stdin`) exactly as
  today.
- **Binary ≤ 10 MB** (currently ~8.97 MB). All M1 techniques are pure
  Rust + SQL with ~zero size cost.
- **Deterministic, explainable scoring.** Every score decomposes into
  inspectable parts.
- Both consumers matter equally: coding agents (JSON) and humans (TTY).

### Milestone roadmap (this spec covers M1 only)

| Milestone | Scope |
|-----------|-------|
| **M1 — Rank-blend core** (this spec) | identifier tokenization, BM25 column weights, ACT-R activation, Beta-smoothed feedback, supersede-aware reranking, MMR diversity, save-time dup warning, prune rewire |
| M2 — Learning loop | query logging, reformulation mining, golden-set harvesting, `comemory eval`, grid-search auto-tuning of blend weights |
| M3 — Code graph | PageRank prior over symbol graph, git co-change edges, cAST chunking, incremental reindex hardening |
| M4 — Examples rewrite | README / cli-reference / `--help` examples rebuilt around real agent + human workflows |

## Section 1 — Data model

All changes live in `comemory.db`. Markdown frontmatter is untouched;
`comemory rebuild` remains the total-recovery path.

1. **Access tracking columns** on `memories` and `code_symbols`:
   `access_count INTEGER NOT NULL DEFAULT 0` and `last_accessed TEXT`
   (defaults to `created`). One `UPDATE` per retrieval hit. Activation uses
   the Petrov approximation of ACT-R base-level learning:

   ```
   activation ≈ ln(access_count) − d·ln(days_since_last_access + 1)
   ```

   with decay exponent `d = 0.5` (configurable). Time is measured in days
   (the `+ 1` keeps the formula finite for same-day access). A brand-new memory counts
   `created` as its first access (`n = 1`), so it ranks on relevance and
   quality alone. Power-law decay means frequently-confirmed knowledge stays
   warm while one-off notes sink. No event table, no gc burden.

2. **Supersede handling** is query-level, not schema-level: the existing
   `supersedes` relation drives a rank penalty and a result annotation
   (Section 2). No rows are hidden or deleted.

3. **Custom FTS5 tokenizer** (registered in Rust via rusqlite on every
   connection open) replaces the stock tokenizer for `memory_fts` and
   `code_fts`. It emits **both** the original token and its
   camelCase / snake_case / digit-boundary subtokens at the same position
   (`VecDimMismatch` → `vecdimmismatch`, `vec`, `dim`, `mismatch`).
   Query-side consistency is automatic because the same tokenizer parses
   queries. Exact-identifier matching is preserved by the dual emit.

## Section 2 — Retrieval pipeline

```
query → router (existing) → candidates → rerank → diversify → emit
```

1. **Candidate stage** (extends `retrieval/hybrid.rs`):
   - FTS5 ranking via `bm25()` with per-column weights — tags and kind
     weighted above body (memories have no title field; the slug line in the
     body carries that signal). Weights are hardcoded constants in M1; M2's
     grid search decides whether they become config.
   - Automatic prefix match (`tok*`) on the last query term.
   - Corrective fallback (`retrieval/corrective.rs`) gains tiered
     relaxation: strict AND → OR-terms. (M2 adds learned expansion as a
     third tier.)
   - Pulls top-50 candidates instead of top-k.

2. **Rerank stage** (new `retrieval/rerank.rs`), multiplicative priors over
   the fused relevance score:

   ```
   score = rrf · activation_boost · feedback_boost · quality_boost · supersede_penalty
   ```

   - Each boost is a bounded multiplier clamped to `[0.5, 2.0]` so no prior
     can drown relevance.
   - `feedback_boost` derives from the Beta-smoothed posterior mean
     `(used + 1) / (used + irrelevant + 4)` over the existing feedback
     table — `irrelevant` votes finally count.
   - `quality_boost` maps the frontmatter `quality` field (1–5).
   - Superseded results get a fixed `×0.2` penalty plus a
     `superseded_by: <id>` annotation. Chains stay visible; recall is never
     silently lost.
   - All multipliers are emitted as `score_parts` in `--json` output
     (rrf, activation, feedback, quality, supersede, final). Explainability
     is a feature contract, not debug output — M2's tuning depends on it.

3. **Diversity stage** (new `retrieval/diversify.rs`):
   - MMR with token-set Jaccard similarity, `λ = 0.7` (configurable):
     greedily pick the next result maximizing
     `λ·score − (1−λ)·max_similarity_to_picked`.
   - SimHash near-duplicate collapse: Hamming distance ≤ 3 keeps only the
     highest-scored member.
   - Cut to top-k (default 12, existing knob).

4. **Save-time duplicate warning**: `comemory save` runs a SimHash check
   against existing memories. On a near-match it prints a TTY warning and
   emits a `duplicate_of: <id>` hint in JSON. The save always proceeds —
   the caller decides whether to supersede instead.

`router.rs` keeps its shape; `rank.rs` is absorbed into the rerank stage;
`bundle.rs` consumes annotated results. No new subcommands in M1.

### Stale-data handling (cross-cutting)

Four layers, no TTL/expiry dates — calendar age is a poor staleness proxy;
usage signals and explicit supersede decide:

1. **Passive decay** — ACT-R activation sinks untouched memories gradually;
   they never disappear.
2. **Explicit supersede** — `×0.2` penalty + annotation, caller-declared,
   reversible.
3. **Feedback** — repeated `irrelevant` votes sink a memory organically.
4. **Prune (active)** — low-value detection rewired to the same signals:

   ```
   low_value = activation < θ_act          # default −2.0
           AND beta_feedback ≤ θ_fb        # default 0.25 (≤ so never-used old
                                           # memories stay prunable, as today)
           AND quality ≤ 2
           AND no incoming edges
   ```

   plus an independent rule: superseded, superseding memory alive, and zero
   accesses since the supersede → prune candidate. Dry-run by default,
   `--apply` soft-deletes to trash, `gc` hard-deletes after 30 days
   (machinery unchanged).

## Section 3 — Config, migration, error handling

**New config keys** (layered defaults → file → env, as today):

| Key | Default | Purpose |
|-----|---------|---------|
| `COMEMORY_RANK_DECAY` | `0.5` | ACT-R decay exponent `d` |
| `COMEMORY_RANK_PRIOR_CLAMP` | `0.5,2.0` | rerank multiplier bounds |
| `COMEMORY_RANK_MMR_LAMBDA` | `0.7` | relevance-vs-diversity trade-off |
| `COMEMORY_PRUNE_MIN_ACTIVATION` | `-2.0` | prune activation threshold (≈ a single access ~55 days ago) |
| `COMEMORY_PRUNE_MIN_FEEDBACK` | `0.25` | prune feedback threshold (zero-feedback memories sit exactly at 0.25) |

The existing low-value knobs (`below_quality`, `unused_since_days`) also get
env wiring (today they are config-file only). Knob restraint is deliberate:
every knob is a tuning liability until `eval` (M2) exists.

**Migration `0003_v3_rank.sql`** (versioned, idempotent, like existing):
- Adds `access_count` / `last_accessed` columns (defaults `0` / `created`).
- Rebuilds `memory_fts` and `code_fts` with the custom tokenizer (an FTS5
  tokenizer change requires a table rebuild; content re-derives from
  `memories` / `code_symbols`, so the rebuild is safe and repeatable).
- The existing `schema_meta` version guard blocks old binaries from a v3 DB.

**Tokenizer registration** happens on every connection open in
`store/connection.rs`; registration failure is a hard error at open (the DB
is unusable without it — fail fast).

**Error handling:**
- The post-retrieval access-count `UPDATE` is best-effort: on failure,
  `tracing::warn` and return results anyway. Ranking telemetry never breaks
  reads.
- The save-time SimHash check is a pure read: on failure, warn and proceed
  with the save.
- Score math is `f64` end-to-end; multiplicative clamps prevent inf/NaN
  propagation.
- `comemory rebuild` reconstructs a v3 DB (including FTS) from markdown.
  Access counts and feedback reset on rebuild — they are runtime stats, not
  source of truth. Documented behavior.

## Section 4 — Testing

Per project rules: all tests in `tests/` mirroring `src/` 1:1, no mock-data
tests — real binary, real data dirs, realistic memory content.

**Property tests (proptest):**
- Tokenizer: never panics on arbitrary UTF-8; subtokens lowercase-normalized;
  `parseHTML` / `parse_html` / `VecDimMismatch` produce expected subtokens;
  the original token is always preserved (dual-emit invariant).
- Score monotonicity: more accesses never decreases activation; more
  `irrelevant` votes never increase the final score; clamp bounds always
  hold; no NaN/inf for any input combination.

**Integration (assert_cmd, real binary, tempdir data-dir):**
- Seeded corpus of ~20 realistic memories (real decision/bug/convention
  phrasing). This corpus doubles as source material for M4's examples.
- Identifier query `VecDimMismatch` finds a memory written as "dim mismatch".
- Feedback loop: mark a result irrelevant ×3 → repeat query shows
  reordering.
- Supersede: penalty applied + `superseded_by` present in JSON.
- Near-duplicate save: warning printed + `duplicate_of` hint in JSON.
- Prune dry-run flags only the seeded low-value memory.
- Migration: committed v2 fixture DB → binary migrates to v3 → search works,
  access columns present.
- Rebuild parity: `rebuild` from markdown produces the same search results
  as the migrated DB (stats reset accepted).

**Snapshot (insta):** `score_parts` JSON contract, TTY result rendering,
dup-warning format.

**Ranking smoke set:** ~10 `(query → expected-top-id)` pairs over the seeded
corpus, asserted as recall@3 — the hand-curated M1 floor that M2's `eval`
command generalizes into user-facing golden-set machinery.

## Out of scope for M1

- Query expansion / reformulation mining, `comemory eval`, weight
  auto-tuning (M2).
- PageRank symbol prior, git co-change edges, cAST chunking, blob-OID
  incremental reindex (M3).
- Full examples/docs rewrite (M4) — M1 only updates `--help` text it
  touches.
- Any HTTP auto-embed integration (explicitly rejected: lexical-max only).
- TTL/expiry for memories (explicitly rejected: usage signals over clocks).

## Research references

- Generative Agents (recency·importance·relevance blend) — arXiv 2304.03442
- ACT-R base-level activation, Petrov approximation — iccm06 PetrovICCM06
- Zep/Graphiti bi-temporal edges — arXiv 2501.13956
- HippoRAG personalized PageRank — arXiv 2405.14831 (M3)
- Aider repo-map symbol PageRank (M3)
- cAST AST-aware chunking — arXiv 2506.15655 (M3)
- CodeRAG-Bench (BM25 competitive for code) — arXiv 2406.14497
- Beta-smoothed CTR / empirical Bayes feedback scoring
- Cascade click model (position-bias-aware skip counting, M2)
- Judgment lists / golden-query tuning (Turnbull; Elastic) (M2)
