# Learned reranking

comemory's ranking is deterministic: RRF over FTS5 and `sqlite-vec`, ACT-R activation, feedback,
quality and supersede priors, MMR diversification, and weighted cross-domain fusion. An operator can
additionally put one **optional** learned ordering stage after that ranking and before the page.

It is **off by default**, and a build that leaves it off launches no process, adds no dependency and
ranks exactly as it did before this existed. The only JSON difference is one `"learned": null` on
`comemory find` and on the console `GET|POST /api/v1/search`, alongside the nulls they already
emitted; `search`, `search-code` and `context` are byte-identical. Everything below is opt-in.

Design contract: [`docs/designs/2026-09-18-learned-rerank-integration.md`](../designs/2026-09-18-learned-rerank-integration.md).
Wire protocol: [`docs/designs/2026-09-18-reranker-command-protocol.md`](../designs/2026-09-18-reranker-command-protocol.md).
Reference backend: [`docs/designs/2026-09-18-reranker-reference-backend.md`](../designs/2026-09-18-reranker-reference-backend.md).

## Configuration

`[rerank]` lives in `config.toml` and is **file-only**, like `[tune]`. There are no
`COMEMORY_RERANK_*` environment variables: a command vector has no readable single-string encoding,
and the rest is machine configuration an operator sets once rather than per invocation.

```toml
[rerank]
# Off by default. This is the kill switch.
enabled = true

# Program and arguments, passed to the operating system separately and never
# through a shell. Both come from this file and from nowhere else.
command = [
  "python3",
  "/abs/path/to/comemory/integrations/reranker/comemory_rerank.py",
  "score",
  "--scoring", "cross-encoder",
]

# The immutable model identity the scorer must echo back, byte for byte.
model = "cross-encoder/ms-marco-MiniLM-L6-v2@233902d25c440f23af6f7d6e94d2946bac0bee0a"

# The adapter identity, or omit the key (or leave it blank) for the base model.
adapter = "lora-v1"

# How many leading candidates of the deterministic ranking are scored. The tail
# below this keeps its deterministic order exactly.
prefix = 50

# End-to-end budget for one scorer child, in milliseconds.
timeout_ms = 20000

# Per-candidate text bound, in bytes, cut at a UTF-8 character boundary.
max_candidate_text_bytes = 4096
```

Validation runs at config load, whether or not the stage is enabled, so a typo is reported when it is
written rather than the first time someone flips `enabled`:

| Rule | Checked |
| --- | --- |
| `prefix >= 1` | always |
| `timeout_ms >= 1` | always |
| `max_candidate_text_bytes >= 1` | always |
| `prefix × max_candidate_text_bytes <= 8 MiB` | always — this is the scorer request ceiling |
| `command` names a program | only when `enabled` |
| `model` is non-blank | only when `enabled` |

The remaining bounds are derived rather than configured. The protocol's `max_candidates` is set to
`prefix`, and the request, stdout and stderr byte caps keep the process runner's defaults (8 MiB,
8 MiB, 16 KiB). A second byte knob would let an operator configure a combination the `prefix` bound
already forbids.

## What it does

```text
one requested search  (search | search-code | find | context)
  |
  +-- domain filters applied first
  +-- route -> rerank -> diversify -> (fuse, for find)      the deterministic ranking
  +-- a bounded candidate universe: retrieval.max_page_window, NOT the page
  +-- candidate identity, content version and bounded text, materialized once
  +-- ONE scorer child process, per requested search
  +-- reordered leading prefix + untouched tail
  +-- pagination, telemetry, context assembly
```

The stage runs **once per requested search**, at each of the four surfaces, and never inside a
retrieval leg — so `comemory find` over all three domains launches exactly one scorer, not three.

## Rollback

Three levers, in increasing order of bluntness.

1. **Point `command` at a base-model invocation and send no adapter.** A LoRA adapter that turns out
   to hurt is rolled back by removing `adapter` and dropping `--adapter` from `command`. This is the
   supported way to fall back from an adapter to its base model: wire protocol v1 cannot express an
   in-process fallback, because a response has to echo the `adapter` it was asked for, and answering
   a `lora-v1` request with base scores while echoing `lora-v1` is a lie the format cannot tell.
2. **Set `enabled = false`.** The next command ranks deterministically and launches nothing.
3. **Do nothing.** Any failure already falls back — see below.

## Failure is always a fallback, never an error

`comemory` never fails a search because a scorer did. Every one of these restores the **entire**
deterministic candidate order and reports itself in the response:

| Situation | What the caller sees |
| --- | --- |
| The program does not exist, or cannot be started | `learned.applied = false`, `learned.fallback` naming the OS error |
| The scorer exits non-zero — including the reference backend's identity gate (exit 65) | `learned.fallback = "reranker exited with Some(65)"` |
| The scorer exceeds `timeout_ms` | `learned.fallback = "reranker timed out after …ms"` |
| The response is malformed, echoes a different model, adapter or request id, omits a score, repeats one, or returns a non-finite value | `learned.fallback` naming the exact divergence |
| The scorer writes more than 8 MiB | `learned.fallback` naming the output limit |

Nothing partial is ever applied: the response is validated in full before a single hit moves.

## API explainability

Every affected payload — `comemory search`, `search-code`, `find` and `context` on both `--json` and
`/api/v1`, plus the console `GET|POST /api/v1/search`, which funnels into `find` — gains exactly one
key, `learned`, and each spells "no stage ran" the way that payload already spells an absent field. `search`, `search-code` and `context` share an envelope whose
optional fields are skipped, so the key is **absent entirely** and their JSON is byte-identical to a
build without this feature. `find` and the console search build their objects by hand and spell absent
fields as explicit nulls — `find` already does for `query_id` and `observation_id` — so there it is
**present and `null`**.

```json
"learned": {
  "applied": true,
  "model": "lexical-overlap@1",
  "adapter": null,
  "request_id": "rr-20260918-1a2b3c4d",
  "pool": 77,
  "prefix": 50,
  "elapsed_ms": 35,
  "scores": [
    {"candidate_id": "memory:7129d7c5", "candidate_ref": "memory:7129d7c5:7129d7c5…",
     "rank": 1, "deterministic_rank": 2, "score": 1.0},
    {"candidate_id": "memory:0bc9e9bf", "candidate_ref": "memory:0bc9e9bf:0bc9e9bf…",
     "rank": 2, "deterministic_rank": 1, "score": 0.75}
  ]
}
```

- `applied` / `fallback` — the fallback status. `fallback` is present only when `applied` is false.
- `model` / `adapter` — the model identity that was asked for and that a valid response echoed.
- `pool` — the deterministic candidate universe this run ranked.
- `prefix` — how many leading candidates were submitted. The rest kept their deterministic order.
- `scores[].rank` — the effective order.
- `scores[].deterministic_rank` — the order that was submitted, so what the deterministic ranking
  produced stays visible beside what the model did with it.
- `scores[].candidate_id` — the opaque id that was on the wire: `<domain>:<id>`, the key the
  candidate pool is keyed by, and therefore unique within one request.
- `scores[].candidate_ref` — the candidate observation contract's reference string, carrying the
  stable identity and the content version. Reported for a reader, never matched on: unlike
  `candidate_id` it is **not unique within a pool**. Its code form is `(repo, path, symbol,
  blob_oid)` while `code_symbols` is unique on `(repo, path, symbol, line_start)`, so two same-named
  functions in one file share one — which is exactly why it is not what goes on the wire. #211 refuses
  a request with a repeated candidate id, and submitting references would decline the whole stage for
  an ordinary code query.

Each hit's own `score_parts` is untouched: the deterministic breakdown is a stable contract and the
learned stage adds to it rather than overwriting it. The TTY view prints one extra line:

```text
learned: lexical-overlap@1  prefix 50/77  35ms
learned: cross-encoder/…@233902d2 (lora-v1)  fallback: reranker timed out after 300ms
```

## Process trust boundary

- **The program and its arguments come from `config.toml` and from nowhere else.** They are passed to
  the operating system as separate arguments and are never assembled into a shell command line. No
  request body, query string or CLI flag can name a program.
- **Query and candidate text reach the child only as bytes on its stdin**, inside one JSON object.
  They are data, never a command and never a shell fragment.
- **One child per request, under one end-to-end deadline** that starts before the spawn and covers
  startup, the pipes and the exit. The child is killed and reaped on every path.
- **Every stream is byte-bounded**: 8 MiB in, 8 MiB out, a 16 KiB stderr excerpt. stderr is
  diagnostic only and is never parsed.
- **Descendants are not reaped.** comemory kills and reaps the process it started; it deliberately
  does not signal a process group, because `kill(-pgid)` can reach processes comemory never started.
  A scorer that forks a descendant holding its stdout is still bounded: the deadline fires, the run
  is refused, and the deterministic ranking answers.
- **A read-only `comemory serve` still reranks** — reranking is a read. It writes no telemetry and
  bumps no access counts, exactly as every other read does there.

## Latency

Reranking adds two costs, and they are separable.

**The fixed candidate window.** With the stage enabled, every request ranks
`retrieval.max_page_window` candidates instead of `clamp(offset + 2·limit, 50, max_page_window)`.
Measured on a real 80-memory store: 7.1 ms to 11.5 ms of retrieval, a fixed cost that no longer grows
with the requested page. The full table is in the design doc's *Measured cost* section.

**Inference.** Whatever the scorer takes, bounded by `timeout_ms`. It dominates: even the trivial
`lexical-overlap` backend costs ~35 ms, almost all of it Python startup, and a real cross-encoder over
50 candidates is far more. Lower `prefix` to lower it.

`retrieval_log.duration_ms` now covers the child, so a latency budget is readable from the query log.

## The extension to the deterministic-ranking contract

Three things change for an operator who enables the stage, and all three are visible.

1. **`total` and `has_more` describe a larger window.** They report the length of the ranked list the
   page was sliced from, which is now the fixed universe rather than a page-proportional pool. On the
   measured corpus a default `search` reports `total: 77` enabled and `total: 48` disabled. Neither
   number is invented; each run really ranked that many candidates.
2. **The deterministic ranking is still what decides the candidate set.** The stage reorders; it never
   admits, drops or rescores a candidate. `score_parts` is exactly what the deterministic pipeline
   produced, and the tail below `prefix` is byte-identical to a disabled run.
3. **Access tracking is unchanged.** Only a head window bumps `memories.access_count` and
   `code_symbols.access_count` — including `comemory context`'s separate code-reference bumps — while
   `retrieval_log` still covers the returned page at every offset (#201). Reranking changes *which*
   rows the head contains, never *that only a head is reinforced*.

## The limits of stateless paging

With a fixed corpus, a fixed configuration, a fixed model and frozen ranking state, walking every page
of a query concatenates to exactly the single full-window run. That is asserted, at several page
sizes, across the prefix/tail boundary, at deep offsets, with `limit = 0` and on an empty result.

It stops holding the moment any of those moves, and comemory does not pretend otherwise:

- **The corpus changed.** A save, a delete or a re-index between two pages changes the candidate set.
- **Ranking state changed.** Activation reads `access_count` and the wall clock. A head query
  re-reinforces its own rows, and `rank.decay > 0` means the same query drifts day to day. Pin
  `rank.decay = 0.0` to remove the time term; nothing removes the reinforcement term except not
  tracking.
- **The model or the backend changed.** A different adapter, a different model revision, or a scorer
  that is available for one page and not the next, produces a different order for the same query.
  `learned.model`, `learned.adapter` and `learned.applied` are in the response precisely so a consumer
  can tell.
- **The configuration was hot-swapped.** A `tune --apply` job can replace `config.toml` between two
  requests, or between the two halves of one reranked request on a running server.

**A fixed model window does not freeze ACT-R or MMR state across separate requests.** That stronger
guarantee needs a snapshot cursor — a server-side handle that pins the ranked list for the life of a
pagination walk — and it is deliberately out of scope here. It is tracked as
[issue #206](https://github.com/Falconiere/comemory/issues/206).

## Candidate observation capture is suppressed while reranking

`[observations] enabled = true` and `[rerank] enabled = true` are mutually exclusive at run time: with
a learned stage active, `comemory find` reports `observation_id: null`, writes no observation row, and
warns once per run.

This is deliberate. The candidate observation contract fixes `pool_position` as the order retrieval
produced *before any arm reordered anything*, and the training dataset is built from it. Recording a
pool the model has already reordered would feed the model's own output back into the set it is trained
from. Collect data with reranking off; serve with it on.

## Offline measurement is never reranked

`comemory eval`, `tune`, `bandit` and `benchmark` keep the deterministic path whatever `[rerank]`
says. `tune` grid-searches deterministic knobs, so measuring them through a model would confound the
search, and `comemory benchmark` is the harness that compares a reranked arm against a baseline.
