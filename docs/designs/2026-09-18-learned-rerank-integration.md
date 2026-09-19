# Optional learned reranking across retrieval — Design

**Date:** 2026-09-18   **Status:** Approved   **Author:** Falconiere R. Barbosa
**Topic:** One opt-in learned ordering stage shared by dedicated search, unified search and context
assembly, with page-independent candidates, a bounded prefix, and a total fallback to the
deterministic ranking.

Issue: https://github.com/Falconiere/comemory/issues/213
Tracking issue: https://github.com/Falconiere/comemory/issues/207
Dependencies: #208 (candidate observation contract), #209 (candidate materialization),
#211 (wire protocol and bounded runner), #212 (reference inference backend).

## Problem

comemory ranks deterministically: RRF over FTS5 and `sqlite-vec`, ACT-R activation and feedback
priors, MMR diversification, and weighted cross-domain fusion. #211 and #212 landed a bounded
process protocol and a reference scorer, but nothing in the retrieval path calls them. There is no
way to evaluate a learned reranker on real queries, and #214 cannot qualify an adapter end to end
without a serving path to qualify it through.

Wiring a scorer in naively breaks four existing contracts at once.

1. **The candidate pool depends on the requested page.** `pipeline::pool_size` returns
   `clamp(offset + 2·limit, CANDIDATE_POOL, max_page_window)`. Paging deeper grows the pool. That is
   safe today only because RRF and MMR are prefix-stable: appending lower-ranked tail candidates
   never reorders the head. A neural scorer has no such property — a candidate that only enters the
   pool at offset 24 can be scored above the item that was rank 1 at offset 0, so page 1 would
   change when the caller asks for page 3.
2. **The server holds one connection mutex for the whole command.** `serve::routes::query_response`
   locks `AppState`'s shared `Mutex<Connection>` and runs the entire command core inside the guard.
   A twenty-second model call inside that closure stalls every other request, including reads that
   never touch the reranked query.
3. **Access tracking is prefix-only by #201.** Only a head window may bump `memories.access_count`
   and `code_symbols.access_count`, because activation is a positive factor of the ranking and a
   gapped bump set floats above the rows it was behind. Reranking changes *which* rows the head
   contains; it must not change *that only a head is reinforced*.
4. **Single-domain `find` must order identically to the dedicated command.** `fuse_domains::fuse`
   preserves a leg's internal order, so `find --domain memory` equals `search` today. A stage
   applied to one surface and not the other silently breaks that.

Evidence:
[`src/domains/retrieval/pipeline.rs`](../../src/domains/retrieval/pipeline.rs),
[`src/domains/retrieval/unified.rs`](../../src/domains/retrieval/unified.rs),
[`src/domains/retrieval/unified/fuse_domains.rs`](../../src/domains/retrieval/unified/fuse_domains.rs),
[`src/serve/routes.rs`](../../src/serve/routes.rs).

## Non-Goals

1. **No adapter, and no reranking, enabled by default.** The default configuration launches no
   process, adds no dependency, and changes no ranking byte. `[rerank] enabled` defaults to `false`
   and a build with no `[rerank]` section behaves exactly as v0.36.0 does.
2. **No training and no dataset export.** #214 owns training and qualification; #210 owns the
   reviewed dataset export. This change touches neither path.
3. **No cursor-stable pagination.** A fixed candidate window does not freeze ACT-R activation or MMR
   state across separate requests. The stronger guarantee needs a snapshot cursor, which is
   [#206](https://github.com/Falconiere/comemory/issues/206) and the maintainer's decision.
4. **No protocol change.** Wire protocol v1 is consumed verbatim. In-process adapter-to-base
   fallback stays inexpressible — a response must echo `adapter` byte for byte, so answering a
   `lora-v1` request with base scores is a lie the format cannot tell. Rollback selects the base
   backend by configuration, or disables reranking. A `scored_with` pair is #212's recorded v2
   candidate and is not introduced here.
5. **No candidate observation capture while reranking is active.** #208's contract fixes
   `pool_position` as "the order retrieval produced it, before any arm reorders anything", and
   #210/#214 train on that order. Capturing a pool the model has already reordered would feed the
   model's own output back into its training set. The two modes are mutually exclusive by
   construction — see § *Capture and reranking are mutually exclusive*.
6. **No descendant process reaping.** #211's Non-Goal 6 stands. See § *Descendant processes*.
7. **No pinning of the code leg's working-set affinity prior.** See § *The working-set affinity
   prior*.
8. **No new CLI flag and no new environment variable.** `[rerank]` is a file-only config section,
   like `[tune]`. `docs/cli-reference.md` and `docs/scenarios/` are therefore unchanged.
9. **No warm-socket or long-lived scorer process.** One child process per requested search, exactly
   as #211 specifies. #212's `serve`/`client` pair is reachable purely by configuring `command`,
   with no comemory-side lifecycle.
10. **No reranking in offline measurement.** `comemory eval`, `tune`, `bandit` and `benchmark` keep
    the deterministic path. `pipeline::search` — which `evaluation::runner` and `benches/retrieval.rs`
    are the only remaining callers of — is left exactly as it is, with no stage and with
    `pool_size` rather than `candidate_pool`. `tune` grid-searches deterministic knobs, so measuring
    them through a model would confound the search; #208's `benchmark` is the harness that compares
    a reranked arm against a baseline, and it reaches `unified::run_legs` / `fuse_legs` directly.
    `scripts/eval-check.sh`'s pinned `recall@3` therefore cannot move.
11. **No reranking on `comemory search --only document`.** `cli::search_only` is the document-only
    interim path that never joined the unified pipeline; it has no `retrieval::` command core and is
    left untouched. `comemory find --domain document` is the reranked way to reach the document leg.

## Architecture

### The shape

```text
one requested search (search | search-code | find | context)
  |
  +-- PHASE 1, under the connection lock ------------------------------------
  |     domain filters applied first (repo / kind / lang / path globs / scope)
  |     route -> rerank -> diversify  (memory)        \
  |     route -> rerank -> coalesce   (code)           }  candidate universe =
  |     BM25                          (documents)     /   max_page_window, NOT
  |                                                        a function of the page
  |     fuse (find only)  ->  the deterministic ranking
  |     candidate_facts::collect_parts -> identity + content version + bounded text
  |     LearnedStage::plan -> LearnedCall { RerankRunner, RerankRequest }
  |     ... owned data only; nothing below borrows the connection
  |
  +-- PHASE 2, NO connection lock, NO open transaction ----------------------
  |     RerankRunner::rerank -> RerankOutcome::{Applied, Declined}
  |
  +-- PHASE 3, under the connection lock ------------------------------------
        learned_rerank::apply  -> reordered prefix + untouched tail
        pipeline::paginate     -> the page
        telemetry              -> retrieval_log at every offset,
                                  access bumps on a head window only (#201)
        context only           -> bundle assembly + its own code-ref bumps
```

The CLI runs all three phases back to back on one owned connection. `comemory serve` runs phase 1
and phase 3 in two separate `spawn_blocking` closures and phase 2 in a third that holds no guard at
all, so the shared mutex is genuinely released for the duration of inference.

### Decisive trade-offs

**The candidate universe becomes the configured maximum window, and stops being a function of the
page.** With reranking enabled, `pipeline::candidate_pool` returns `retrieval.max_page_window`
(200 by default) for every request, whatever `offset` and `limit` are. This is the issue's own
initial proposal and it is what makes stateless paging survive a non-prefix-stable stage: every page
of the same query, against the same corpus and config, scores the same candidate set in the same
submitted order, so page 3 cannot rewrite page 1. The cost is a fixed retrieval cost per query
instead of a page-proportional one; § *Measured cost* records it.

**Only a fixed leading prefix is scored, and the tail is preserved verbatim.** `[rerank] prefix`
(50 by default) bounds both inference cost and blast radius. Candidates at deterministic positions
`prefix..pool` keep their exact order and their exact deterministic score breakdown. A reranked run
and a disabled run therefore agree byte-for-byte from position `prefix` onward, which is directly
assertable.

**The stage sits at exactly four call sites, all of them a surface's own `begin`, and never inside a
leg.** `search::begin`, `search_code::begin`, `context::begin` and `find::begin` each invoke it
exactly once. `pipeline::search` is *not* one of them: it is split into `pipeline::rank` and
`pipeline::complete`, the two surfaces that used it compose those halves around the stage
themselves, and `pipeline::search` itself survives unchanged as `rank` + `complete` for its two
remaining callers, `evaluation::runner` and `benches/retrieval.rs` (Non-Goal 10).
`unified::memory_leg`, `code_search::search_code_hits` and `doc_route::route_documents` are
untouched, so a unified query cannot score twice: there is no code path from a leg to the stage.
Suppression inside legs is a structural property, not a flag.

**A fixed candidate universe changes `total`, and that is the honest number.** `paginate` reports
`total` as the length of the ranked list it sliced. With reranking enabled that list is the whole
`max_page_window`, so a default `search` reports `total: 200` where a disabled run reports `24`, and
`has_more` is true further into the list. Nothing is invented: the enabled run really did rank 200
candidates. This is a visible contract difference between an enabled and a disabled build, it is
documented as such, and AC-18 pins it.

**Parity is preserved by construction, not by matching two implementations.** `search` and
`find --domain memory` both apply the stage to the same deterministic list, with the same pool, the
same prefix and the same candidate identities, before pagination. The same holds for `search-code`
and `find --domain code`. Because the candidate id on the wire is `<domain>:<id>` — the pool key,
which says nothing about the surface — the two runs submit byte-identical requests and receive
byte-identical orders.

**A refusal is applied through the same code path as a success.** `RerankOutcome::order_ids()`
returns the reranked order when applied and the *submitted* order when declined.
`learned_rerank::apply` reorders by `order_ids()` unconditionally, so the declined path restores the
entire original candidate order because it is the identity permutation, not because a second branch
remembered to. There is one ordering implementation and it cannot drift.

**The wire id is `<domain>:<id>`, and #208's `candidate_ref` rides beside it in the report.** The
reference string is the obvious candidate and is wrong: its code form is `(repo, path, symbol,
blob_oid)` while `code_symbols` is unique on `(repo, path, symbol, line_start)` and the extractor
captures a bare identifier, so two same-named functions in one file — `fn new` twice, which
comemory's own tree has — produce byte-identical references. #211 refuses a request with a repeated
candidate id, so submitting references would decline the whole stage for an ordinary code query.
`(domain, id)` is what keys the candidate pool in the first place and is unique within one request by
construction. What #214 actually needs from train/serve parity is the TEXT, and that is still
materialized through #209's `candidate_facts` either way; the reference is reported so a reader can
still see which candidate a score belongs to.

**Capture and reranking are mutually exclusive.** See § *Capture and reranking are mutually
exclusive* below. This is the only arrangement that cannot contaminate the training set.

### Where the code lives

| Path | Role |
| --- | --- |
| `src/config/rerank.rs` | The `[rerank]` section: `RerankConfig`, its file overlay and its validation |
| `src/domains/retrieval/learned_rerank.rs` | The one learned ordering stage: `LearnedStage`, `LearnedCall`, `LearnedPlan`, `apply` |
| `src/domains/retrieval/learned_report.rs` | `LearnedOrdering` / `LearnedScore` — the explainability surface both adapters serialize |
| `src/domains/retrieval/staged.rs` | `Staged` / `Paused` / `FinishStep` / `resolve` — the pause point between the deterministic ranking and the learned order |
| `src/serve/routes/staged.rs` | `staged_query_response` — three blocking phases, the connection guard held for the first and third only |
| `src/domains/retrieval/{pipeline,search,search_code,context,find,unified}.rs` | `candidate_pool`, `pipeline::{rank, complete}`, and the `begin` / `run` split at each surface |
| `src/serve/routes/{find,code,search,memories/search}.rs` | Four handlers move onto `staged_query_response`. `GET\|POST /api/v1/search` funnels into `retrieval::find::run` too and is included |
| `src/domains/learning/evaluation/candidate_facts.rs` | `collect_parts`, so a single-leg surface materializes through the same rules `find` does |
| `integrations/reranker/comemory_rerank.py` | Unchanged. Reached only through `[rerank] command` |

### Capture and reranking are mutually exclusive

`observation_capture::armed` gains one more condition: capture also requires that no learned
ordering stage is active. When `[rerank] enabled = true`, `comemory find` returns
`observation_id: null` and emits one `tracing::warn!` naming the suppression.

The alternative — capturing the reranked pool — cannot be made correct inside this slice.
`benchmark_observe::observe` derives `returned_position` from the pool index and the window, so it
is right only while the page is the window's slice of the pool in pool order. Passing it the
reranked pool fixes `returned_position` and corrupts `pool_position`; passing it the deterministic
pool does the reverse. Recording the model's own output as `pool_position` would train #214 on a
pool the model already reordered. Suppression costs nothing operationally: capture is a
dataset-collection mode and reranking is a serving mode, and #208's benchmark reaches
`unified::run_legs` / `fuse_legs` directly, with no stage in between, so the dataset path is
untouched either way.

### The working-set affinity prior

`code_search::search_code_hits` builds `WorkingSet::from_cwd(repo)` from the process CWD and does
not hand it back. #208 recorded this and explicitly left the decision here.

**Decision: it is not pinned, and its signature does not change.** Pinning it would change a
production code path — the affinity prior is a deliberate feature, not an accident, and #208's
Non-Goal 3 refused to change production ranking for the same reason. Handing it back would push
`search_code_hits` to eight parameters, which `clippy::too_many_arguments` denies, and would ripple
a new return shape through `LegRows` for a value no acceptance criterion needs.

What changes is that the input stops being invisible. `LearnedOrdering::scores` carries
`deterministic_rank` beside the learned `rank` for every scored candidate, so the order the stage
*submitted* is visible in every reranked response. Two machines that disagree on the affinity prior
now disagree visibly, in the response body, instead of silently. The existing mitigations stand and
are documented: `--repo` pins the label the working set is built under, and a process whose CWD is
outside a git work tree gets `WorkingSet::default()` — the empty, neutral set a `comemory serve`
started outside a checkout already receives.

The residual risk this change does add is recorded honestly: because the affinity prior selects
which candidates occupy the reranked prefix, a machine-dependent input can now move the *learned*
order as well as the deterministic one. Pinning it belongs with a snapshot cursor (#206) or its own
issue, because it is a production ranking change.

### Descendant processes

**Decision: #211's Non-Goal 6 stands; the runner still does not signal a process group.** The stated
failure mode — a scorer that spawns a descendant holding stdout, so the direct child exits but the
pipe never reaches EOF — is already *bounded*: the deadline fires, the run is refused as
`RerankFailure::TimedOut`, the direct child is killed and reaped, and the caller's deterministic
ranking stands. No request hangs. Implementing group kill would add `libc` and an `unsafe` block
governed by D1's SAFETY rule, and `kill(-pgid)` without `process_group(0)` can signal processes
comemory never started, while `process_group(0)` changes terminal signal delivery for
`utilities::embed`'s existing users. The blast radius of a stray group signal is unbounded; the
blast radius of a leaked short-lived descendant is one process. No acceptance criterion requires
reaping.

This is proven rather than asserted: AC-14 drives a real scorer that forks a descendant holding the
pipe and asserts the search returns the deterministic order within the configured budget.

## Interfaces / Schema

### `[rerank]` configuration (file-only, `config.toml`)

```toml
[rerank]
enabled = false
command = ["python3", "/abs/path/integrations/reranker/comemory_rerank.py", "score"]
model = "cross-encoder/ms-marco-MiniLM-L6-v2@233902d25c440f23af6f7d6e94d2946bac0bee0a"
adapter = "lora-v1"
prefix = 50
timeout_ms = 20000
max_candidate_text_bytes = 4096
```

```rust
/// src/config/rerank.rs
pub struct RerankConfig {
    /// Whether a learned ordering stage runs. Default `false`.
    pub enabled: bool,
    /// Program and arguments, passed to `Command` separately and never
    /// through a shell. Validated non-empty when `enabled`.
    pub command: Vec<String>,
    /// The immutable model identity the response must echo byte for byte.
    /// Validated non-empty when `enabled`.
    pub model: String,
    /// The adapter identity, `None` for the base model. An empty string in
    /// the file is read as `None`.
    pub adapter: Option<String>,
    /// How many leading candidates are scored. Validated `> 0`. Default `50`.
    pub prefix: usize,
    /// End-to-end budget for one child. Validated `> 0`. Default `20000`.
    pub timeout_ms: u64,
    /// Per-candidate text bound handed to `BoundedText::bound`. Validated
    /// `> 0`. Default `4096`.
    pub max_candidate_text_bytes: usize,
}
```

Validation, in `Config::validate`, and all of it enforced whether or not `enabled` is set except
where noted:

| Rule | Failure |
| --- | --- |
| `prefix > 0` | `rerank.prefix must be > 0` |
| `timeout_ms > 0` | `rerank.timeout_ms must be > 0` |
| `max_candidate_text_bytes > 0` | `rerank.max_candidate_text_bytes must be > 0` |
| `enabled` implies `!command.is_empty()` | `rerank.command must name a program when rerank.enabled is true` |
| `enabled` implies `!model.trim().is_empty()` | `rerank.model must name the model identity when rerank.enabled is true` |
| `prefix * max_candidate_text_bytes <= RerankLimits::default().max_request_bytes` | `rerank.prefix × rerank.max_candidate_text_bytes exceeds the 8388608-byte request limit` |

The 8 MiB request ceiling is DERIVED from `RerankLimits::default().max_request_bytes` rather than
restated in `config`, so the two cannot drift; `config::defaults` already reaches into `utilities`
the same way for `simhash::NEAR_DUP_HAMMING`. The remaining bounds are derived too, not configured: `max_candidates` is set to `prefix`
(lowering #211's 256 default, as its Open Question 2 anticipated), and `max_request_bytes`,
`max_stdout_bytes` and `max_stderr_bytes` keep `RerankLimits::default()` (8 MiB / 8 MiB / 16 KiB).
A second byte knob would let an operator configure a combination the prefix bound already forbids.

### The stage

```rust
/// src/domains/retrieval/learned_rerank.rs

/// One configured learned ordering stage. Built once per requested search.
pub struct LearnedStage { /* private */ }

impl LearnedStage {
    /// The stage this configuration implies, or `None` when reranking is off.
    /// `None` is the whole of the default-disabled guarantee: no runner is
    /// constructed, so no process can be launched.
    pub fn from_config(cfg: &Config) -> Option<Self>;

    /// The per-candidate text bound this stage materializes at.
    pub fn text_bytes(&self) -> usize;

    /// Build the scoring call for the leading prefix of `keys`, or `None`
    /// when there is nothing to score (an empty ranking).
    pub fn plan(
        &self,
        query: &str,
        keys: &[(CandidateDomain, String)],
        facts: &FactsByHit,
        now: OffsetDateTime,
    ) -> Option<LearnedCall>;
}

/// A fully owned scoring call: the runner, the request, and nothing borrowed.
pub struct LearnedCall { /* private */ }

impl LearnedCall {
    /// Run the child. Never returns `Err`: a failure is a `Declined`.
    pub fn score(&self) -> RerankOutcome;
    /// The metadata `apply` needs, without holding the call itself.
    pub fn plan(&self) -> LearnedPlan;
}

/// What was submitted, kept so the outcome can be applied after the call
/// has been consumed.
pub struct LearnedPlan {
    /// Candidate references in submitted (deterministic) order.
    pub submitted: Vec<String>,
    /// The model identity that was asked for.
    pub model: String,
    /// The adapter identity that was asked for.
    pub adapter: Option<String>,
    /// The request id both sides agreed on.
    pub request_id: String,
}

/// Reorder the leading `plan.submitted.len()` entries of `ranked` into
/// `outcome`'s order and keep the tail exactly where the deterministic
/// ranking put it.
///
/// Declined and applied share this one path: `RerankOutcome::order_ids`
/// yields the submitted order on a refusal, so a refusal restores the
/// complete original ranking as the identity permutation.
pub fn apply<T>(
    ranked: Vec<T>,
    plan: &LearnedPlan,
    outcome: &RerankOutcome,
) -> (Vec<T>, LearnedOrdering);
```

### The explainability surface

```rust
/// src/domains/retrieval/learned_report.rs
#[derive(Debug, Clone, Serialize)]
pub struct LearnedOrdering {
    /// Whether the scorer's response was applied.
    pub applied: bool,
    /// The model identity that was asked for, echoed byte for byte on success.
    pub model: String,
    /// The adapter identity that was asked for; `null` for the base model.
    pub adapter: Option<String>,
    /// `rr-<yyyymmdd>-<8hex>` of this run's request.
    pub request_id: String,
    /// The deterministic candidate universe this run built.
    pub pool: usize,
    /// How many leading candidates were submitted for scoring.
    pub prefix: usize,
    /// Wall-clock time of the child run, in milliseconds. `0` when nothing ran.
    pub elapsed_ms: u64,
    /// Why the deterministic order stands, when `applied` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    /// The learned order, one entry per submitted candidate. Empty on a refusal.
    pub scores: Vec<LearnedScore>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LearnedScore {
    /// The opaque id that was on the wire: `<domain>:<id>`, the key the
    /// candidate pool is keyed by and therefore unique within one request.
    pub candidate_id: String,
    /// #208's candidate reference string, reported beside it. NOT the wire id:
    /// it is not injective over `code_symbols`, whose uniqueness key is
    /// `(repo, path, symbol, line_start)` while the reference's code form is
    /// `(repo, path, symbol, blob_oid)` — two same-named functions in one file
    /// share one, and #211 refuses a repeated candidate id.
    pub candidate_ref: String,
    /// 1-based position in the learned (effective) order.
    pub rank: usize,
    /// 1-based position in the deterministic order that was submitted.
    pub deterministic_rank: usize,
    /// The finite score the scorer returned.
    pub score: f64,
}
```

JSON: every affected payload gains exactly one key, `learned`, and each spells "no stage ran" the
way that payload already spells an absent field.

- `search`, `search-code` and `context` build theirs from a shared `Envelope`, where every optional
  field carries `skip_serializing_if`. `learned` does too, so the key is **absent entirely** and the
  two `insta` envelope snapshots do not move.
- `find` assembles its object by hand at both call sites and spells every absent field as an explicit
  `null` — `query_id` and `observation_id` already do. `learned` follows suit: **present and `null`**.

Both shapes are pinned by `tests/learned_rerank.rs::disabled_by_default_changes_nothing`. A disabled
build's `search` / `search-code` / `context` JSON is byte-identical to v0.36.0; its `find` JSON gains
one `"learned": null` alongside the two nulls it already emitted.

```json
{
  "hits": [ ... ],
  "query_id": "q-20260918-0a1b2c3d",
  "limit": 12, "offset": 0, "has_more": true, "total": 200,
  "learned": {
    "applied": true,
    "model": "lexical-overlap@1",
    "adapter": null,
    "request_id": "rr-20260918-1a2b3c4d",
    "pool": 200,
    "prefix": 50,
    "elapsed_ms": 83,
    "scores": [
      {"candidate_id": "memory:679929eb", "candidate_ref": "memory:679929eb:9f2c…",
       "rank": 1, "deterministic_rank": 7, "score": 0.42},
      {"candidate_id": "memory:5a9f19bc", "candidate_ref": "memory:5a9f19bc:3ab1…",
       "rank": 2, "deterministic_rank": 1, "score": 0.31}
    ]
  }
}
```

A refusal:

```json
"learned": {
  "applied": false,
  "model": "cross-encoder/ms-marco-MiniLM-L6-v2@233902d2…",
  "adapter": "lora-v1",
  "request_id": "rr-20260918-1a2b3c4d",
  "pool": 200, "prefix": 50, "elapsed_ms": 312,
  "fallback": "reranker timed out after 300ms",
  "scores": []
}
```

Carrying `learned` on all four surfaces:

```rust
pub struct SearchResult     { /* … */ pub learned: Option<LearnedOrdering> }
pub struct SearchCodeResult { /* … */ pub learned: Option<LearnedOrdering> }
pub struct ContextResult    { /* … */ pub learned: Option<LearnedOrdering> }
pub struct FindResult       { /* … */ pub learned: Option<LearnedOrdering> }
```

`search_result::envelope`, `code_search_result::envelope` and `context_result::envelope` each take
one more argument, `learned: Option<&LearnedOrdering>`, and are shared verbatim by the CLI `--json`
writer and the `/api/v1` handler, so CLI/HTTP parity is structural. `find`'s JSON is assembled at
both call sites today and gains the same key at each, as does the console `GET|POST /api/v1/search`,
which funnels into `retrieval::find` and reshapes its hits.

### The pause point

```rust
/// src/domains/retrieval/staged.rs

/// A command core paused between its deterministic ranking and the learned
/// order, so a caller that holds a shared database lock can release it for
/// the duration of the model call.
pub enum Staged<T> {
    /// No stage ran; the result is final.
    Ready(T),
    /// Inference is pending. Nothing here borrows a connection.
    Paused(Paused<T>),
}

pub struct Paused<T> { /* private */ }

impl<T> Paused<T> {
    /// Split into the connection-free call and the step that needs the
    /// connection back.
    pub fn into_parts(self) -> (LearnedCall, FinishStep<T>);
}

/// The remainder of a paused command core.
pub struct FinishStep<T>(/* private */);

impl<T> FinishStep<T> {
    pub fn new(
        f: impl for<'a, 'b> FnOnce(&'b mut Ctx<'a>, RerankOutcome) -> Result<T> + Send + 'static,
    ) -> Self;
    pub fn run(self, ctx: &mut Ctx<'_>, outcome: RerankOutcome) -> Result<T>;
}

impl<T> Staged<T> {
    /// Project the eventual result, on either arm. The `Ready` arm applies `f`
    /// at once; the `Paused` arm composes it into the continuation, so a
    /// failure there surfaces after inference rather than before it. This is
    /// the ONE `map` — a `Paused::map` and a `FinishStep::map` beside it were
    /// three near-identical bodies the duplication ratchet counted.
    pub fn map<U>(self, f: impl FnOnce(T) -> Result<U> + Send + 'static) -> Result<Staged<U>>;
}

/// Run a staged core to completion on one connection — what the CLI does.
pub fn resolve<T>(ctx: &mut Ctx<'_>, staged: Staged<T>) -> Result<T>;
```

`FinishStep<T>`'s closure is `Send + 'static`, which decides four function bodies: a phase-3
continuation may capture only owned values. `Filters<'_>` borrows its `TimeScope`, so each `begin`
moves the owned `Request` and `TimeScope` into the closure and rebuilds `Filters` inside phase 3
rather than capturing it.

Each surface gains a `begin` beside its existing `run`, and `run` becomes
`staged::resolve(ctx, begin(ctx, req, track)?)`:

```rust
pub fn begin(ctx: &mut Ctx<'_>, req: Request, track: bool) -> Result<Staged<SearchResult>>;
pub fn begin(ctx: &mut Ctx<'_>, req: Request, track: bool) -> Result<Staged<SearchCodeResult>>;
pub fn begin(ctx: &mut Ctx<'_>, req: Request, track: bool) -> Result<Staged<ContextResult>>;
pub fn begin(ctx: &mut Ctx<'_>, req: Request, track: bool) -> Result<Staged<FindResult>>;
```

```rust
/// src/serve/routes/staged.rs
/// Run a staged command core across three blocking phases, holding the shared
/// connection guard for the first and the third only.
pub(crate) async fn staged_query_response<T, B>(
    state: AppState,
    command: &str,
    begin: B,
) -> Response
where
    B: FnOnce(&mut Ctx<'_>) -> Result<Staged<T>> + Send + 'static,
    T: Serialize + Send + 'static;
```

### The candidate universe

```rust
/// src/domains/retrieval/pipeline.rs
/// The deterministic ranking for a memory query: route -> rerank -> diversify,
/// cut at `pool`. Split out of [`search`] so a surface can insert the learned
/// ordering stage between the ranking and the page.
pub fn rank(
    cfg: &Config, conn: &Connection, query: &str, vec: Option<&[f32]>,
    filters: Filters<'_>, pool: usize,
) -> Result<Vec<Reranked>>;

/// Slice `ranked` to `opts.window` and record best-effort telemetry. The
/// second half of [`search`], and the last step of every memory-domain surface.
pub fn complete(
    conn: &Connection, query: &str, filters: Filters<'_>,
    opts: SearchOptions, ranked: Vec<Reranked>, started: Instant,
) -> SearchRun;

/// The candidate universe for one run.
///
/// With a learned ordering stage active this is `retrieval.max_page_window`,
/// independent of the requested page: a neural scorer is not prefix-stable, so
/// a page-proportional pool would let a deeper page rewrite a shallower one.
/// Without one it is [`pool_size`], unchanged.
pub fn candidate_pool(cfg: &Config, window: PageWindow) -> usize;
```

Called by `search::begin`, `search_code::begin`, `context::begin` and `unified::run_legs` (which
`find::begin` drives). `pipeline::search` keeps calling `pool_size` directly, so the offline
measurement path's pool is unchanged (Non-Goal 10).

### The TTY view

The non-`--json` renderers for `search`, `search-code`, `find` and `context` print one extra line
when a stage ran, and nothing at all when none did:

```text
learned: lexical-overlap@1  prefix 50/200  83ms
learned: cross-encoder/ms-marco-MiniLM-L6-v2@233902d2 (lora-v1)  fallback: reranker timed out after 300ms
```

### Materialization

```rust
/// src/domains/learning/evaluation/candidate_facts.rs
/// [`collect`] over loose leg slices, so a single-domain surface materializes
/// through exactly the rules `find` does. `collect` delegates to it.
pub fn collect_parts(
    conn: &Connection,
    memory: &[Reranked],
    code: &[CodeReranked],
    documents: &[DocHit],
    max_text_bytes: usize,
) -> Result<FactsByHit>;
```

Per surface: `search` and `context` pass `(&ranked, &[], &[])`; `search-code` passes
`(&[], &ranked, &[])`; `find` keeps `collect(conn, &legs, …)`.

### Structural budget

Measured with the gate's own counter (`scripts/guardrails/checks/file-size.sh`, code lines only) at
`9507b3d0`: `pipeline.rs` 204/300, `find.rs` 202/300, `unified.rs` 136/300, `search_code.rs`
126/300, `search_result.rs` 96/300, `code_rerank.rs` 256/300. `rank` and `complete` are *extracted*
from `search`, so `pipeline.rs` grows only by `candidate_pool` and the two new doc blocks. Nothing
this change adds touches `code_rerank.rs`, the tightest file. If `pipeline.rs` or `find.rs` does
cross the ceiling, the split is `src/domains/retrieval/search_telemetry.rs` taking
`record_telemetry` / `record_access` / `record_query` / `log_retrieval`; that is a contingency, not
a planned file.

Two `insta` snapshots serialize the whole `search` and `search-code` envelopes
(`tests/snapshots/output__search__search_json_envelope_contract.snap` and its `search_code` twin).
`learned` is `Option` with `skip_serializing_if = "Option::is_none"` and both fixtures leave it
`None`, so neither snapshot moves. `tests/ranking_invariance.rs` replays a committed golden that
must never be regenerated; it runs with reranking absent from its config and is unaffected. Both are
assertions AC-1 makes, not assumptions.

## Failure modes and edge cases

| Situation | Observable behavior |
| --- | --- |
| `[rerank]` absent, or `enabled = false` | `LearnedStage::from_config` returns `None`. No `RerankRunner` is constructed, no process is spawned, `candidate_pool` is `pool_size` unchanged, the `learned` key is absent from the `search` / `search-code` / `context` envelopes and `null` on `find` and the console search, and ranking is byte-identical to v0.36.0 |
| `enabled = true`, `command` empty or `model` blank | Config load fails with a named `Error::Config`. The binary refuses to run any command, rather than silently searching unreranked |
| Zero candidates (empty corpus, or every filter excluded everything) | `plan` returns `None`; no process is spawned; `learned` is absent; the empty page is returned |
| Fewer candidates than `prefix` | The whole ranking is submitted; the tail is empty |
| `limit = 0` | `candidate_pool` is `max_page_window` either way; the whole reranked window is returned from `offset` onward |
| Scorer binary missing | `RerankFailure::Spawn` → `Declined` → deterministic order, `learned.applied = false`, `learned.fallback` names the OS error. The search succeeds |
| Scorer exits non-zero (#212's identity gate, exit 65) | `RerankFailure::NonZeroExit` → `Declined` → deterministic order. The stderr excerpt reaches `tracing::warn!` |
| Scorer exceeds `timeout_ms` | `RerankFailure::TimedOut` → `Declined` → deterministic order. The direct child is killed and reaped |
| Scorer spawns a descendant holding stdout | Same as above. The descendant is neither signalled nor waited for (Non-Goal 6); the reader thread holds one bounded pipe and exits when the last writer closes |
| Response is malformed, echoes a different model/adapter/request id, omits a score, repeats one, or returns a non-finite value | `validate` refuses it → `Declined` → the complete original order. Nothing partial is ever applied |
| Scorer writes more than 8 MiB | `RerankFailure::OutputTooLarge`, killed as soon as the cap is passed → `Declined` |
| A code or document row is re-indexed away between the ranking and the facts read | `candidate_facts` yields `BoundedText::unavailable()` — empty text, `text_available = false`. The candidate keeps its position and is still submitted, so the pool never silently shrinks |
| A row is re-indexed away between phase 1 and phase 3 on HTTP (the window the released lock opens) | The reranked order is unchanged (it is computed from owned data); the page's navigation metadata degrades exactly as it already does for a raced soft-delete — empty path/kind/tags |
| `config.toml` is hot-swapped by a `tune --apply` job between phase 1 and phase 3 | Phase 3 reads the newer `Config`. Only pagination and telemetry read it at that point; the ranking was already decided. Documented, not defended against |
| Read-only `comemory serve` | `track` is `false`, so no telemetry and no access bumps. Reranking still runs: it is a read |
| `[observations] enabled = true` together with `[rerank] enabled = true` | Capture is suppressed, `observation_id` is `null`, one `tracing::warn!` per run (Non-Goal 5) |
| Deep offset beyond `max_page_window` | Unchanged: `paginate` forces `has_more = false` at the ceiling |
| `total` / `has_more` with a stage active | Computed over the fixed window, so an enabled run reports the whole ranked window (200 at the defaults) where a disabled run reports the page-proportional pool (24). Honest, not inflated — the enabled run really ranked that many candidates — and pinned by AC-18 |
| `retrieval_log.duration_ms` with a stage active | Includes the child's wall-clock time, because the timer starts before the ranking and stops after the page is cut. That is the latency the caller actually waited, and it is what a latency budget must be read from |
| Two concurrent HTTP searches, both reranking | Both run their own child. `RerankRunner` is `Send + Sync` and borrows `&self`; neither holds the connection guard during inference |

## Acceptance criteria

- **AC-1:** With no `[rerank]` section, `comemory search` / `search-code` / `context` `--json` output
  contains no `learned` key at all and `comemory find`'s contains `"learned": null` beside the
  `query_id` and `observation_id` nulls it already emitted; every surface reports the legacy
  page-proportional `total`
  (`pool_size(0, 12, 200) == 24` on a 60-memory corpus), and the whole pre-existing suite —
  `tests/cli__search*.rs`, `tests/output__*.rs`, `tests/ranking_invariance.rs`, `scripts/eval-check.sh`
  and the three `insta` snapshots — passes unchanged.
- **AC-2:** With `[rerank] enabled = false` but `command` pointing at a script that creates a
  sentinel file, running all four commands leaves the sentinel absent.
- **AC-3:** `[rerank] enabled = true` with `command` empty, or with `model` blank, fails config load
  with an error naming the offending key; `prefix = 0`, `timeout_ms = 0` and
  `max_candidate_text_bytes = 0` each fail the same way; and a `prefix × max_candidate_text_bytes`
  product over 8 MiB fails naming the request limit.
- **AC-4:** With the shipped `integrations/reranker/comemory_rerank.py score --scoring
  lexical-overlap` backend configured, `comemory search` over a memory-only corpus returns the hits
  ordered by the Jaccard score the test computes independently from the same memory bodies it
  seeded, and `learned.applied` is `true` with `learned.model == "lexical-overlap@1"`.
- **AC-5:** One requested `comemory find --domain all` over a corpus with memory, code and document
  hits launches exactly one scorer process, proven by a wrapper script that appends one line per
  invocation.
- **AC-6:** With reranking enabled, the candidate pool is `retrieval.max_page_window` for
  `--offset 0 --k 5`, for `--offset 120 --k 5` and for `--k 0`, and the concatenation of every page
  at sizes 1, 5 and 12 equals the single `--k 0` full-window run, position for position, with
  `rank.decay = 0.0` and `COMEMORY_DISABLE_ACCESS_TRACKING=true`.
- **AC-7:** With `prefix = 3` over a 10-candidate window, positions 4 through 10 of the reranked run
  are identical — ids and full `score_parts` — to positions 4 through 10 of the same query with
  reranking disabled.
- **AC-8:** A scorer that exits 65, one that prints malformed JSON, one that echoes the wrong
  `model`, one that omits a score and one that exceeds `timeout_ms` each produce a result whose hit
  order and `score_parts` are identical to the disabled run, with `learned.applied == false` and a
  `learned.fallback` message naming the failure.
- **AC-9:** `comemory search "<q>"` and `comemory find "<q>" --domain memory` return the same memory
  ids in the same order with reranking enabled; `comemory search-code "<q>"` and
  `comemory find "<q>" --domain code` likewise.
- **AC-10:** Against a real `comemory serve` whose scorer sleeps 3 seconds, a `GET /api/v1/find`
  issued first and a `GET /api/v1/stats` issued 200 ms later both succeed, and `stats` returns
  before `find` does.
- **AC-11:** With reranking enabled and tracking on, `comemory search --offset 0` bumps
  `memories.access_count` for exactly the returned page's ids; `--offset 12` bumps nothing and still
  writes a `retrieval_log` row whose `returned_ids` is the page it returned. `comemory context`
  bumps `code_symbols.access_count` for the bundle's resolved code refs on a head window only. No
  candidate outside the returned page is ever bumped.
- **AC-12:** CLI `--json` and the matching `/api/v1` `data` payload carry an identical `learned`
  object for the same query against the same store, with `learned.scores[*].deterministic_rank`
  present and distinct from `rank` for at least one candidate.
- **AC-13:** With `[observations] enabled = true` and `[rerank] enabled = true`,
  `comemory find --json` reports `observation_id: null` and writes no
  `candidate_query_observations` row; with reranking disabled the same command writes one.
- **AC-14:** A scorer that forks a descendant holding stdout open past the budget yields
  `learned.applied == false` with a `timed out` fallback, and the command returns within twice
  `timeout_ms`.
- **AC-15:** Domain filters are applied before materialization: `find --repo R --kind decision
  --lang rust --path 'docs/**'` with reranking enabled submits only candidates that satisfy the
  per-leg filters, proven by reading the request body off a recording proxy — a script that tees its
  stdin to a file and then executes the real shipped backend, so the search under test is a real
  end-to-end run and not a stand-in.
- **AC-16:** A `find` whose code index is rebuilt between the ranking and the facts read still
  returns the full pool, with the vanished candidate submitted with empty text and kept at its
  position.
- **AC-17:** The extra retrieval cost of the fixed window is measured and recorded: pool size and
  wall-clock for a default `search` with and without reranking, on the same real corpus, written into
  § *Measured cost* of this document.
- **AC-18:** With reranking enabled, a default `comemory search --json` over a 60-memory corpus
  reports `total` equal to the ranked window rather than the page-proportional pool, and the same
  query with reranking disabled reports the page-proportional pool — the two numbers differ, and both
  equal the length of the list their run actually sliced.

## Acceptance evidence

| AC | Real input / fixture | Expected observable result | Boundary / failure case | Runnable check |
| --- | --- | --- | --- | --- |
| AC-1 | Real store seeded by `tests/common/`; no `[rerank]` in `config.toml` | `--json` output equal to the pre-change baseline captured in the same test run | Empty corpus | `tests/learned_rerank.rs::disabled_by_default_changes_nothing` |
| AC-2 | `enabled = false`, `command = ["sh", "-c", "touch $SENTINEL"]` | Sentinel path does not exist after four commands | Command configured but unreachable | `tests/learned_rerank.rs::disabled_launches_no_process` |
| AC-3 | Six malformed `config.toml` files | `comemory search` exits non-zero, stderr names the key | Product bound exactly at 8 MiB passes | `src/config/tests/rerank.rs`, `tests/learned_rerank.rs::invalid_rerank_config_is_refused` |
| AC-4 | Real memories whose BM25-plus-quality order and Jaccard order differ; real `comemory_rerank.py` child | Returned order equals the independently computed Jaccard order, and differs from the deterministic one | Two candidates with equal Jaccard → submitted rank breaks the tie (`search_code_reranks_over_colliding_symbol_references`) | `tests/learned_rerank.rs::applies_the_backend_order` |
| AC-5 | Corpus with all three domains; wrapper script appending to a log | Log has exactly one line | `--domain memory` also exactly one | `tests/learned_rerank.rs::one_scorer_process_per_search` |
| AC-6 | 60-memory corpus, `rank.decay = 0.0`, access tracking disabled | Page concatenation equals the `--k 0` run | `--offset` past `max_page_window`; `limit = 0`; empty result set | `tests/learned_rerank.rs::paging_matches_the_full_window` |
| AC-7 | `prefix = 3`, 10 candidates | Tail identical including `score_parts` | `prefix` ≥ pool size → empty tail | `tests/learned_rerank.rs::tail_is_preserved` |
| AC-8 | Five stub scorers, each a real child process | Order and `score_parts` equal the disabled run | Non-finite score; duplicate score | `tests/learned_rerank.rs::every_refusal_restores_the_order` |
| AC-9 | One corpus, one query, memory and code | Identical id sequences | Zero-hit query | `tests/learned_rerank.rs::{dedicated_and_single_domain_find_agree, search_code_reranks_over_colliding_symbol_references}` |
| AC-10 | Real `comemory serve`; scorer `python3 -c "…sleep(3)…"` | `stats` completes while `find` is in flight | Two concurrent reranked requests | `tests/learned_rerank_serve.rs::unrelated_read_completes_during_inference` |
| AC-11 | Real store, tracking on, clock pinned via `rank.decay = 0.0` | `access_count` deltas exactly on the returned head page | Deep offset bumps nothing; `context`'s separate code-reference bumps over real `references_symbol` edges | `tests/learned_rerank.rs::{access_tracking_policy_is_preserved, context_reinforces_only_a_head_window, context_code_reference_bumps_survive_reranking}` |
| AC-12 | Same store driven by CLI and by `comemory serve`, over all four surfaces plus the console one | `learned` objects equal after removing `elapsed_ms` and `request_id` | Declined run on both surfaces; a code pool whose observation references collide | `tests/learned_rerank_serve.rs::{cli_and_http_learned_payload_match, a_declined_scorer_degrades_identically_on_both_surfaces, the_console_search_carries_the_learned_object}` |
| AC-13 | `[observations] enabled = true`, `[rerank] enabled = true` | `observation_id` null, zero rows | Reranking disabled → one row | `tests/learned_rerank.rs::capture_is_suppressed_while_reranking` |
| AC-14 | `python3 -c` scorer forking a `sleep 5` descendant that inherits stdout | `applied == false`, fallback names the timeout | Budget 300 ms | `tests/learned_rerank.rs::descendant_holding_the_pipe_times_out` |
| AC-15 | Multi-repo, multi-language, multi-document corpus | Captured request contains only in-filter candidate refs | A filter that excludes a whole leg | `tests/learned_rerank.rs::filters_apply_before_materialization` |
| AC-16 | A real `code_symbols` row purged between the ranking and the facts read — the exact shape `index_code`'s purge-and-reinsert produces | Pool length unchanged; the vanished candidate is reported unavailable, keeps its position and is still submitted with empty text | A candidate with no facts entry at all | `src/domains/retrieval/tests/learned_rerank.rs::{a_code_row_reindexed_away_is_still_submitted_at_its_position, a_candidate_whose_row_vanished_keeps_its_place_with_empty_text}` |
| AC-17 | Real corpus, `--json` timing over 10 runs | Recorded in § *Measured cost* of this document | — | `tests/learned_rerank.rs::records_the_extra_retrieval_cost` |
| AC-18 | 60-memory corpus, default `top_k` | `total` differs between the enabled and disabled runs, each equal to its own ranked-window length | Corpus smaller than the window → both equal the corpus size | `tests/learned_rerank.rs::fixed_window_reports_its_own_total` |

## Measured cost

Measured on 2026-09-18, aarch64 macOS, `--release` build, over a real store of 80 saved memories all
matching the query. Each figure is the median of ten full `comemory --json …` process runs with
`COMEMORY_DISABLE_ACCESS_TRACKING=true` and `[rank] decay = 0.0`, so it includes binary startup and
the SQLite open. `[rerank] prefix = 50`, `retrieval.max_page_window = 200`, scorer =
`integrations/reranker/comemory_rerank.py score --scoring lexical-overlap`.

| Run | Ranked window | Candidates scored | Total | Inference | Retrieval (total − inference) |
| --- | --- | --- | --- | --- | --- |
| `search`, disabled | 48 | 0 | 7.1 ms | — | 7.1 ms |
| `search`, enabled | 77 | 50 | 46.5 ms | 35 ms | 11.5 ms |
| `find --domain all`, disabled | 48 | 0 | 7.5 ms | — | 7.5 ms |
| `find --domain all`, enabled | 77 | 50 | 46.0 ms | 36 ms | 10.0 ms |
| `search --offset 120 --k 5`, disabled | 77 | 0 | 8.2 ms | — | 8.2 ms |
| `search --offset 120 --k 5`, enabled | 77 | 50 | 46.8 ms | 36 ms | 10.8 ms |

Three things the table says.

**The retrieval cost this design adds on its own is about 4 ms on this corpus — 7.1 ms to 11.5 ms.**
That is the price of ranking the whole `max_page_window` instead of `pool_size(0, 12, 200) = 50`: a
48-row ranked window becomes a 77-row one, and the routing, reranking and diversification behind it
grow with it. It is a fixed cost per query rather than a page-proportional one, which is exactly the
point: the last row shows the disabled run already paying it at `--offset 120`, where its own pool
has grown to 124.

**Inference dominates, and this backend is the cheap case.** The 35 ms is almost entirely Python
interpreter startup for a scorer that computes Jaccard similarity; a real cross-encoder over 50
candidates is orders of magnitude more, which is why `[rerank] timeout_ms` defaults to 20 000 and why
`prefix` exists at all. A latency budget must be read from `retrieval_log.duration_ms`, which now
covers the child.

**The ranked window really does stop depending on the page.** 77 at the head and 77 at offset 120
with the stage on; 48 at the head and 77 at offset 120 with it off. That difference is what
`tests/learned_rerank.rs::fixed_window_reports_its_own_total` asserts, and reverting
`pipeline::candidate_pool` to `pool_size` turns it red.

## Documentation impact

- `README.md` — a `[rerank]` subsection under configuration: opt-in, what it costs, how to roll back.
- `AGENTS.md` — `[rerank]` named in the file-only config paragraph beside `[tune]`; the module map
  rows for the new files; the "no in-process LLM" architecture line qualified to name the optional
  out-of-process scorer.
- `docs/guides/learned-reranking.md` — new: configuration, the process trust boundary (program and
  arguments come from local config only, payload text reaches the child only as stdin bytes, no
  shell), latency and the fixed-window cost, the API explainability surface, the fallback ladder, and
  the explicit limits of stateless paging.
- `docs/architecture.md` — the retrieval pipeline diagram gains the optional stage.
- `src/config/README.md`, `src/domains/retrieval/README.md`, `src/serve/routes/README.md` — one row
  per new file.
- `docs/designs/2026-09-17-domain-first-migration-inventory.md` — one inventory row per new
  production `src/**/*.rs`, with the `bridge` column listing each `#[path]` colocated suite.
- `docs/cli-reference.md` and `docs/scenarios/` — unchanged: no CLI flag or subcommand is added.

## Open Questions

1. **Should `[rerank]` gain environment-variable overrides?** Owner: this issue, resolved as *no*.
   The section is file-only, like `[tune]`, because a command vector has no readable single-string
   env encoding and because nine new `COMEMORY_RERANK_*` rows would document a surface an operator
   configures once per machine. Non-blocking; adding overrides later is additive.
2. **Should capture and reranking ever be combinable?** Owner: the maintainer. It needs a #208
   contract extension — an explicit ranking-arm field on the observation, and a `returned_position`
   that is decoupled from `pool_position` — which is an `OBSERVATION_VERSION` bump. Out of scope
   here (Non-Goal 5). Non-blocking.
3. **Should the code leg's working-set affinity prior be pinned?** Owner: this issue, resolved as
   *not pinned*; see § *The working-set affinity prior*. Non-blocking, and recorded as a production
   ranking change that belongs with #206 or its own issue.
4. **Should the runner kill the child's process group?** Owner: this issue, resolved as *no*; see
   § *Descendant processes*. Non-blocking, and now evidenced rather than asserted (AC-14).
5. **Does a warm scorer process belong here?** Owner: #214. #212 ships `serve`/`client`, and
   pointing `command` at `client` needs no comemory change, so this is configuration rather than
   code. Non-blocking.
