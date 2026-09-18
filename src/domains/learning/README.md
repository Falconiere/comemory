# `src/domains/learning/`

**What belongs here:** the learning capability end to end — the feedback
counters and the provenance policy that decides which value a verdict is
stored under (`feedback_tracking`, `code_feedback`), the shared `comemory.db`
handle those writers borrow their transactions from (`telemetry`), the
evaluation and ranking-search algorithms ([`evaluation/`](evaluation/README.md)),
and the seven command cores `cli::` and `serve::routes::` call (`feedback`,
`eval`, `mine`, `tune`, `bandit`, plus the console-only `console` and
`learning_proposals`).

**What does NOT belong here:** SQL, rendering, and the shared telemetry
vocabulary. Every SQL string lives in `store::{feedback, code_feedback,
retrieval_log, eval_runs, query_expansions, bandit_arms, gc_learning}`; this
folder keeps the rules those statements serve — the chunk-to-parent identity a
code counter is keyed by, and each batch's transaction boundary. The persisted
`retrieval_log.source` / `feedback_events.{target_kind,provenance}` words and
the `q-<yyyymmdd>-<8hex>` query-id contract are shared leaves in
`utilities::telemetry` and `utilities::query_id` (#166), so retrieval, graph
and store consume them directly and never depend on this capability for them.
Turning stored counters into a ranking multiplier is likewise not ours: that is
`retrieval::score` and `retrieval::code_prior`.

No file here may import `cli` (the `cli::output` writers included) or `serve`. The
one capability this folder depends on is `domains::retrieval`, through
`evaluation::runner` alone, which drives the real pipeline exactly as a CLI
caller would. `domains::graph` depends on us in the other direction, for the
co-activation reward.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `benchmark.rs` | `Request` | Shared middle of `comemory benchmark` — load a reviewed versioned set, capture every task's candidate pool through the real retrieval legs, score every arm over that one snapshot. CLI-only; writes nothing to the database |
| `bandit.rs` | `Request` | Shared middle of `comemory bandit` / `POST /api/v1/bandit` — always mutating, always confirm-gated |
| `code_feedback.rs` | `record_code_with_provenance` | Per-symbol `used`/`irrelevant` counters. Owns the rowid→identity resolution (an unknown id is a loud error; a chunk walks one hop to its parent, because the `name#n` key a chunk-keyed counter would carry is one the scoring join can never match) and the transaction; the SQL is `store::code_feedback` |
| `console.rs` | `Summary` | Console-only: `GET /api/v1/learning/{summary,evals,golden-set,expansions}`. Was `api::learning`, renamed for the collision with its own capability |
| `eval.rs` | `Request` | Shared middle of `comemory eval` / `POST /api/v1/eval` — read class, no confirm gate — plus `history` behind `eval --history` and `GET /eval/history` |
| `evaluation.rs` | — | Declares [`evaluation/`](evaluation/README.md); named for the collision with the `eval` core |
| `feedback.rs` | `Request` | Shared middle of `comemory feedback` / `POST /api/v1/feedback` (and the per-hit search route). Validates the query id, parses `Source`, and writes memory and code verdicts through the two recorders |
| `feedback_tracking.rs` | `Source` | Per-memory `used`/`irrelevant` counters, plus `Source` — the routes' `explicit`\|`implicit` request word and its one mapping onto the stored `manual`\|`implicit` provenance. Also `record_implicit_used`, which takes a bare `&Connection` so the caller keeps its own transaction |
| `judge.rs` | `Request` | Shared middle of `comemory judge` — resolve typed verdicts against a captured observation through `evaluation::judgment`, refusing a pool-recall miss, a stale content version and an unresolvable candidate, all-or-nothing; `target_of` is the identity→`JudgmentTarget` inverse #210 renders reviewed sets with |
| `learning_proposals.rs` | `Proposal` | Console-only: knob proposals derived from unapplied `tune`/`bandit` runs — list, apply (writes `config.toml`), discard |
| `mine.rs` | `Request` | Shared middle of `comemory mine` / `POST /api/v1/mine` — a bounded scan, not confirm-gated |
| `observation_capture.rs` | `record` | Opt-in, bounded capture of a real `find` run's candidate pool: `armed` (config AND a run allowed to write telemetry), `find_filters`, and the best-effort write that warns rather than failing a search |
| `telemetry.rs` | `StatsDb` | The shared `comemory.db` connection handle, opened through `store::connection`. Owns no table: the two recorders borrow `conn_mut()` for their transactions |
| `tune.rs` | `Request` | Shared middle of `comemory tune` / `POST /api/v1/tune` — mutating only when `apply`, and confirm-gated only then |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/domains/learning.rs`
(`pub mod <name>;`) and callers import concrete paths.
