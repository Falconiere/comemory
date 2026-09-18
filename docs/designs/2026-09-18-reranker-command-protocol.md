# Versioned reranker command protocol and bounded process runner — Design

**Date:** 2026-09-18   **Status:** Approved   **Author:** Auto
**Topic:** A versioned JSON stdin/stdout protocol for an external relevance
scorer, and the reusable, deadline-bounded process runner that carries it.

Issue: [#211](https://github.com/Falconiere/comemory/issues/211).
Tracking issue: [#207](https://github.com/Falconiere/comemory/issues/207).

## Problem

comemory needs a model-independent inference boundary before any reranking
work can proceed. The only precedent is `src/utilities/embed.rs`, and it is
subtly wrong for this purpose.

`embed_query_with_timeout` runs four steps in sequence:

1. `spawn(cmd)` — unbounded.
2. `write_stdin(&mut child, query)` — a **synchronous, blocking** `write_all`
   of the whole payload, unbounded.
3. `read_with_timeout(&mut child, timeout)` — the only bounded step.
4. `child.wait()` — unbounded.

The timeout therefore starts after the entire request has already been written
and stops before the child has been waited on. Three consequences matter here:

- **It is not an execution budget.** A child that spawns slowly, or that reads
  its input and then never exits, can pin the caller well past `timeout`.
- **A large payload deadlocks the parent.** Step 2 completes only when the
  child drains stdin. A child that writes its answer to stdout before reading
  stdin fills the stdout pipe (64 KiB on Linux, 16 KiB on macOS), blocks, and
  never drains stdin — while the parent blocks in `write_all` because nothing
  is draining it. Neither side moves and no timer is running. Embedding
  payloads are one query, so this is latent there; rerank payloads are a query
  plus up to a few hundred bounded candidate texts, so it is not.
- **Nothing is bounded but time.** Input size, stdout size and stderr size are
  all unbounded, so a misbehaving child can exhaust memory inside the budget.

This issue defines the wire contract and the runner. It does **not** enable
reranking on any search surface and adds no model.

## Non-Goals

1. **No model, no Python, no HuggingFace dependency.** The reference inference
   backend is [#212](https://github.com/Falconiere/comemory/issues/212).
2. **No retrieval integration and no configuration plumbing.** No search
   surface calls this code, no `[rerank]` config section, no environment
   variable and no CLI flag is added. That is
   [#213](https://github.com/Falconiere/comemory/issues/213). Nothing in this
   change adds a dependency or a behavior to the normal search path.
3. **No dataset, benchmark or judgment work.**
   [#208](https://github.com/Falconiere/comemory/issues/208),
   [#209](https://github.com/Falconiere/comemory/issues/209) and
   [#210](https://github.com/Falconiere/comemory/issues/210) own those.
4. **No per-domain candidate identity.** A candidate id is an opaque,
   domain-qualified string on this wire. What a memory, code or document id
   *means*, and how candidate text is bounded *semantically*, belong to #208's
   shared candidate observation contract.
5. **No SQL, no schema change, no migration.** Nothing here is persisted.
6. **No descendant process reaping.** The runner kills and reaps the process it
   owns. It deliberately does not signal a process group: that needs `libc` and
   an `unsafe` block, and it can kill processes comemory did not start.
7. **No Windows support.** The supported platforms are the three cargo-dist
   targets (see "Platforms"); all are Unix.
8. **No transport other than a local child process.** No socket, no HTTP, no
   long-lived server. One process per request.

## Architecture

Seven new flat files under `src/utilities/`, plus a refactor of the existing
`embed.rs` onto the shared runner. `src/utilities/` takes no subfolders
(`guardrails.config.json`'s `src.nested` allows only `tests` there), so every
file is a flat sibling and its colocated tests live in `src/utilities/tests/`.

```
src/utilities/process_runner.rs   ProcessRunner      — the bounded runner (no JSON)
src/utilities/process_pipes.rs    Pipes              — its worker threads and poll loop
src/utilities/rerank_protocol.rs  RerankRequest      — the wire types (no process I/O)
src/utilities/rerank_outcome.rs   RerankOutcome      — the result vocabulary (pure data)
src/utilities/rerank_validate.rs  validate           — response checking (pure function)
src/utilities/rerank_runner.rs    RerankRunner       — the four composed
src/utilities/dated_id.rs         dated_id           — the shared id shape
src/utilities/embed.rs            embed_query        — refactored onto ProcessRunner
src/utilities/query_id.rs         generate_query_id  — refactored onto dated_id
```

The split is by responsibility, not by size: `process_runner` knows nothing
about reranking and is what `embed` reuses; `process_pipes` isolates the
concurrency so `process_runner` is only the contract and the bounds;
`rerank_protocol` is pure wire data with no I/O, so a test can build a response
by hand; `rerank_outcome` holds the result and failure vocabulary that both the
validator and the runner produce, so neither file has to own the other's type;
`rerank_validate` is a pure function over `(&RerankRequest, &RerankResponse)`
so every rejection rule is unit-testable without a subprocess; `rerank_runner`
is the only file that composes a process with a protocol.

`dated_id` exists because the reranker request id is the same
`<prefix>-<yyyymmdd>-<8hex>` shape as the existing retrieval query id. Minting
it a second time would be the duplication Binding Rule 1 forbids and would show
up in the `dup-check` ratchet, so the shape is extracted once and
`query_id::generate_query_id` / `is_valid_query_id` become thin wrappers over
it with their signatures, their output and their existing tests unchanged.

The 300-code-line ceiling is comfortable here: the guardrails `file-size` check
excludes blank and `//`-prefixed lines, so the doc comments every `pub` item
needs do not count, and no single file carries more than roughly 150 lines of
actual code.

### The single deadline

`ProcessRunner::run` computes `deadline = Instant::now() + timeout` **before**
spawning and treats every subsequent step as competing for the remainder.
Startup, stdin writing, stdout reading, stderr reading and process exit are all
inside it.

Concurrency is three detached worker threads plus a polling loop on the calling
thread:

| Thread | Owns | Does |
| --- | --- | --- |
| writer | the child's stdin | `write_all(input)`, then drops the handle (EOF) |
| stdout reader | the child's stdout | reads to EOF, retaining at most `max_stdout_bytes` |
| stderr reader | the child's stderr | reads to EOF, retaining at most `max_stderr_bytes` |
| calling thread | the `Child` | polls `try_wait()` and the three channels until all four finish or the deadline passes |

The calling thread keeps `&mut Child`, which is what `kill()` needs, so there
is no thread that owns the child and blocks in `wait()`. `std` has no
`wait_timeout`, and the alternatives (`libc::kill`, a fourth thread holding the
child) either add `unsafe` or take away the ability to kill. Polling
`try_wait()` on a 2 ms interval is portable, allocation-free and costs
microseconds per poll.

This shape is what survives each hostile child:

- **Never reads stdin.** The writer thread blocks or gets `EPIPE`; the calling
  thread is not blocked, so stdout is still drained and the run completes on
  the child's own terms. `EPIPE` is the child's choice, not a failure — it is
  reported as `input_truncated` and does not by itself fail the run.
- **Writes before reading.** Both pipes are serviced concurrently, so neither
  side can fill.
- **Closes stdout but stays alive.** stdout reaches EOF, `try_wait()` keeps
  returning `None`, the deadline fires, the child is killed and reaped.
- **Spawns descendants holding the pipes.** The direct child exits but stdout
  never reaches EOF, so the payload can never be known to be complete. The
  deadline fires and the run is refused. The direct child is reaped; the reader
  thread stays blocked on a pipe a descendant still holds and is **not joined**
  — it holds one pipe handle and one `Sender`, consumes no CPU, is capped in
  memory by `max_stdout_bytes`, and exits when the last writer closes. Joining
  it is exactly the unbounded wait this design exists to avoid.

On every exit path — success, failure or timeout — the calling thread kills the
child if it has not already exited and then `wait()`s it, so no zombie is left
behind. `Child::wait` after `kill` returns as soon as the direct child is
reaped, regardless of descendants.

### The security boundary

`ProcessRunner::new(program, args)` takes a program path and an argument
vector. It never builds a shell command line and never accepts one string to be
word-split. Query and candidate text reach the child **only** as bytes on
stdin, inside the JSON request body.

`RerankRunner` — which holds the program, the arguments, the timeout and the
limits — derives no `Deserialize`, so it cannot be materialized from a request
body, an HTTP payload or a stored memory. The only way to obtain one is
`RerankRunner::new(program, args)` called by Rust code that read those values
from trusted local configuration. `RerankRequest`, the only value that crosses
the wire, has no command, program, argument, shell or environment field, so
there is nothing a caller could smuggle through.

`embed.rs` keeps its documented `sh -c <cmd>` contract — that is a
pre-existing, user-configured surface (`COMEMORY_EMBED_CMD` /
`serve --embed-cmd`) and its behavior must be preserved — but it now expresses
it as `ProcessRunner::new("sh", ["-c", cmd])`. The shell is the *program*,
selected by trusted local configuration; the runner itself still never
concatenates a command string.

### Reuse

- `crate::utilities::digest::sha256_hex` mints the request id, exactly as
  `crate::utilities::query_id::generate_query_id` mints `q-<yyyymmdd>-<8hex>`.
  No second entropy source and no new dependency.
- `crate::prelude::{Error, Result}` and `Error::Embedder` are reused unchanged;
  no new `Error` variant is added, because `RerankRunner` reports operational
  failure as a typed value rather than through `Error`.
- `serde` / `serde_json` are already direct dependencies.
- No new crate is added to `Cargo.toml`.

### Platforms

The supported targets are the three cargo-dist targets named in AGENTS.md
§ Distribution: `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu` and
`aarch64-unknown-linux-gnu`. All three are Unix, and the implementation uses
only `std::process`, `std::thread`, `std::sync::mpsc` and `std::time`, with no
`cfg(target_os)` branch, no `unsafe` and no `libc`. `Child::kill` is `SIGKILL`
on all three. The integration tests drive `/bin/sh`, which every target ships,
and the scripts are strict POSIX so they behave identically under macOS `sh`
and Debian `dash`.

## Interfaces / Schema

### `src/utilities/process_runner.rs`

```rust
/// Byte and time bounds one bounded child run is held to.
pub struct ProcessLimits {
    pub max_input_bytes: usize,   // default 8 MiB
    pub max_stdout_bytes: usize,  // default 8 MiB
    pub max_stderr_bytes: usize,  // default 16 KiB
}

/// A child process to run once, under one end-to-end deadline.
pub struct ProcessRunner { /* program, args, timeout, limits */ }

impl ProcessRunner {
    pub fn new(program: impl Into<OsString>, args: Vec<OsString>) -> Self;
    pub fn with_timeout(self, timeout: Duration) -> Self;
    pub fn with_limits(self, limits: ProcessLimits) -> Self;
    pub fn run(&self, input: &[u8]) -> ProcessResult<ProcessOutput>;
}

/// What a completed run produced.
pub struct ProcessOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// The child closed stdin before the whole request was written (EPIPE).
    pub input_truncated: bool,
    pub elapsed: Duration,
}

/// Why a bounded run did not produce a complete output.
pub enum ProcessFailure {
    InputTooLarge { bytes: usize, max: usize },
    Spawn(String),
    Io { phase: &'static str, message: String },
    StdoutTooLarge { max: usize },
    TimedOut { budget: Duration },
}

pub type ProcessResult<T> = std::result::Result<T, ProcessFailure>;
```

`run` returns `Ok(ProcessOutput)` for **any** completed run, including a
non-zero exit: interpreting the status belongs to the caller, because `embed`
and `rerank` word their errors differently. `ProcessFailure` is deliberately
its own type rather than `crate::Error`, so a caller can map it without
matching on message text; `embed` maps it to the exact `Error::Config` strings
it produces today, and ignores `input_truncated` exactly as it swallows
`BrokenPipe` today.

`run` is the only place the input-size bound is checked, so there is one
enforcer of it: the rerank runner does not re-check the serialized size, it
maps `ProcessFailure::InputTooLarge` onto `RerankFailure::RequestTooLarge`.

### `src/utilities/rerank_protocol.rs`

```rust
pub const RERANK_PROTOCOL_VERSION: u32 = 1;

pub struct RerankCandidate { pub id: String, pub rank: u32, pub text: String }
pub struct RerankRequest {
    pub protocol_version: u32,
    pub request_id: String,
    pub model: String,
    pub adapter: Option<String>,
    pub query: String,
    pub candidates: Vec<RerankCandidate>,
}
pub struct RerankScore { pub id: String, pub score: f64 }
pub enum ScoreDirection { HigherIsBetter, LowerIsBetter }
pub struct RerankResponse {
    pub protocol_version: u32,
    pub request_id: String,
    pub model: String,
    pub adapter: Option<String>,
    pub score_direction: ScoreDirection,
    pub scores: Vec<RerankScore>,
}

impl RerankRequest {
    /// Mint `rr-<yyyymmdd>-<8hex>` from the query and `now`, then build.
    pub fn new(model, adapter, query, candidates, now: OffsetDateTime) -> Self;
    /// Build with a caller-supplied request id (replay, tests, tracing).
    pub fn with_request_id(request_id, model, adapter, query, candidates) -> Self;
}

/// Validate the `rr-<yyyymmdd>-<8hex>` request-id shape.
pub fn is_valid_request_id(s: &str) -> bool;
```

Every struct derives `Serialize`, `Deserialize` and
`#[serde(deny_unknown_fields)]`. `ScoreDirection` is
`#[serde(rename_all = "snake_case")]`.

### `src/utilities/rerank_outcome.rs`

```rust
/// One candidate in the order the caller should now use.
pub struct RerankedCandidate { pub id: String, pub rank: u32, pub score: f64 }

/// Every way a rerank attempt can be refused. Typed, never message-only,
/// except where an underlying parser's text is the only useful detail.
pub enum RerankFailure {
    EmptyCandidates,
    TooManyCandidates { count: usize, max: usize },
    DuplicateCandidateId { id: String },
    CandidateTextTooLarge { id: String, bytes: usize, max: usize },
    RequestTooLarge { bytes: usize, max: usize },
    Spawn { message: String },
    Io { phase: String, message: String },
    TimedOut { budget_ms: u64 },
    NonZeroExit { code: Option<i32> },
    OutputTooLarge { max: usize },
    Malformed { message: String },
    VersionMismatch { expected: u32, actual: u32 },
    RequestIdMismatch { expected: String, actual: String },
    ModelMismatch { expected: String, actual: String },
    AdapterMismatch { expected: Option<String>, actual: Option<String> },
    MissingScores { ids: Vec<String> },
    DuplicateScore { id: String },
    UnknownScore { id: String },
    NonFiniteScore { id: String },
}

/// The reranked order, plus the identity the scorer confirmed.
pub struct RerankApplied {
    pub request_id: String,
    pub model: String,
    pub adapter: Option<String>,
    pub order: Vec<RerankedCandidate>,
    pub stderr_excerpt: String,
    pub elapsed: Duration,
}

/// Nothing was applied. `original_order` is the submitted order verbatim,
/// so a caller can restore the complete original ranking from the failure
/// alone, without holding its own copy.
pub struct RerankDeclined {
    pub request_id: String,
    pub failure: RerankFailure,
    pub original_order: Vec<String>,
    pub stderr_excerpt: String,
}

pub enum RerankOutcome { Applied(RerankApplied), Declined(RerankDeclined) }

impl RerankOutcome {
    /// The candidate ids in the order the caller should use, in both arms.
    pub fn order_ids(&self) -> Vec<&str>;
    pub fn is_applied(&self) -> bool;
    pub fn failure(&self) -> Option<&RerankFailure>;
}
```

`RerankFailure` derives `Debug` and implements `Display` so a caller can log
one line without matching every variant; it is **not** a `crate::Error` variant,
because a scorer failing must never be able to become a search failure by an
accidental `?`.

### `src/utilities/rerank_validate.rs`

```rust
/// Check `response` against `request` and return the reranked order.
/// Ties break by the submitted `rank`, ascending.
pub fn validate(
    request: &RerankRequest,
    response: &RerankResponse,
) -> std::result::Result<Vec<RerankedCandidate>, RerankFailure>;
```

`validate` checks in this fixed order, so the reported failure is the most
specific one available: protocol version, request id, model, adapter, then per
score — unknown id, duplicate id, non-finite score — then missing ids, then the
sort.

### `src/utilities/rerank_runner.rs`

```rust
/// Default end-to-end budget for one rerank child.
pub const DEFAULT_RERANK_TIMEOUT: Duration = Duration::from_secs(20);

/// Protocol-level bounds, on top of [`ProcessLimits`].
pub struct RerankLimits {
    pub max_candidates: usize,            // default 256
    pub max_candidate_text_bytes: usize,  // default 8 KiB
    pub max_request_bytes: usize,         // default 8 MiB — becomes
                                          // ProcessLimits::max_input_bytes
    pub max_stdout_bytes: usize,          // default 8 MiB
    pub max_stderr_bytes: usize,          // default 16 KiB
}

/// The trusted local configuration that selects the scorer executable.
/// No `Deserialize` derive: it can never be built from a request body.
pub struct RerankRunner { /* program, args, timeout, limits */ }

impl RerankRunner {
    pub fn new(program: impl Into<OsString>, args: Vec<OsString>) -> Self;
    pub fn with_timeout(self, timeout: Duration) -> Self;
    pub fn with_limits(self, limits: RerankLimits) -> Self;

    /// Run one request. Never returns `Err`: every operational failure is a
    /// `Declined` carrying the original order, so a retrieval caller has one
    /// branch and cannot propagate a scorer failure as a search failure.
    pub fn rerank(&self, request: &RerankRequest) -> RerankOutcome;
}
```

### Wire protocol

This section is the contract #212, #213 and #214 consume verbatim.

**Transport.** One child process per request. The request is one UTF-8 JSON
object written to the child's **stdin**, followed by EOF (the parent closes
stdin). The response is one UTF-8 JSON object written to the child's
**stdout**, followed by EOF, with exit status `0`. Neither side is
newline-delimited and neither is streamed: exactly one object each way. The
child's **stderr** is diagnostic only, is never parsed, and is captured up to
`max_stderr_bytes` for error reporting.

**Request object.** All keys are required and unknown keys are rejected.

| Field | JSON type | Rule |
| --- | --- | --- |
| `protocol_version` | integer | Currently `1`. The response must echo it exactly. |
| `request_id` | string | `rr-<yyyymmdd>-<8 lowercase hex>`, 20 characters. The response must echo it exactly. |
| `model` | string | Non-empty. The immutable model identity the caller expects. The response must echo it byte-for-byte. |
| `adapter` | string or `null` | The adapter (LoRA) identity, `null` for the base model. The key is always present. The response must echo it exactly, `null` included. |
| `query` | string | The search query, as data. Never a command, never a shell fragment. |
| `candidates` | array | Non-empty, at most `max_candidates` entries, ordered by `rank` ascending. |

**Candidate object.**

| Field | JSON type | Rule |
| --- | --- | --- |
| `id` | string | Opaque, domain-qualified, unique within the request. The protocol never parses it and attaches no meaning to its shape. #208 owns what it means. |
| `rank` | integer | The candidate's zero-based position in the caller's deterministic order; equal to its array index. Carried explicitly so the tie-break survives a scorer that reorders its echo. |
| `text` | string | The bounded candidate text, at most `max_candidate_text_bytes` **bytes** when UTF-8 encoded. The caller truncates; the protocol rejects an over-cap candidate rather than silently cutting it, because where to cut is #208's semantic decision. |

**Response object.** All keys are required and unknown keys are rejected.

| Field | JSON type | Rule |
| --- | --- | --- |
| `protocol_version` | integer | Must equal the request's, or the response is refused (`VersionMismatch`). |
| `request_id` | string | Must equal the request's (`RequestIdMismatch`). |
| `model` | string | Must equal the request's byte-for-byte (`ModelMismatch`). |
| `adapter` | string or `null` | Must equal the request's exactly (`AdapterMismatch`). |
| `score_direction` | string | `"higher_is_better"` or `"lower_is_better"`. Declared per response; never assumed. |
| `scores` | array | Exactly one entry per request candidate. Array order is irrelevant. |

**Score object.**

| Field | JSON type | Rule |
| --- | --- | --- |
| `id` | string | Must be one of the request's candidate ids. Unknown → `UnknownScore`; repeated → `DuplicateScore`; absent → `MissingScores`. |
| `score` | number | Must deserialize to a **finite** `f64`. `NaN`, `+Inf` and `-Inf` → `NonFiniteScore`. JSON has no literal for them, so they can only arrive via overflow or a future transport; the check is unconditional. |

**Worked example.** Request:

```json
{"protocol_version":1,"request_id":"rr-20260918-1a2b3c4d","model":"bge-reranker-base","adapter":null,"query":"bounded subprocess","candidates":[{"id":"memory:679929eb","rank":0,"text":"..."},{"id":"code:comemory:src/utilities/embed.rs:embed_query","rank":1,"text":"..."}]}
```

Response:

```json
{"protocol_version":1,"request_id":"rr-20260918-1a2b3c4d","model":"bge-reranker-base","adapter":null,"score_direction":"higher_is_better","scores":[{"id":"memory:679929eb","score":0.13},{"id":"code:comemory:src/utilities/embed.rs:embed_query","score":0.87}]}
```

Applied order: `code:comemory:src/utilities/embed.rs:embed_query` (rank 1,
score 0.87) first, `memory:679929eb` (rank 0, score 0.13) second.

**Ordering and tie-breaking.** Sort by `score` — descending under
`higher_is_better`, ascending under `lower_is_better` — with ties broken by the
submitted `rank`, ascending. Equal scores therefore preserve the caller's
deterministic order exactly, and the whole ordering is total and reproducible:
scores are already proven finite, so `f64::total_cmp` is used and no comparator
can panic or produce a non-deterministic order.

**Versioning.** `protocol_version` is a single integer with no minor component.
Any additive field, any relaxed rule and any new `score_direction` value
requires incrementing it; `deny_unknown_fields` on both objects makes a silent
divergence impossible. A scorer that receives a `protocol_version` it does not
implement should exit non-zero with a diagnostic on stderr rather than answer.

**What is out of scope for this wire.** Per-domain identity fields, candidate
provenance, judgments, corpus or index revisions, score calibration and dataset
formats. If a downstream issue needs one, it belongs in #208's candidate
observation contract or in a version-2 bump of this protocol, not in an
unversioned extra key.

## Failure modes and edge cases

| Case | Observable behavior |
| --- | --- |
| Program does not exist / is not executable | `Declined(Spawn)`; no thread started, nothing to reap. |
| Serialized request exceeds `max_request_bytes` | `Declined(RequestTooLarge)`; **no process is spawned**. |
| More than `max_candidates` candidates | `Declined(TooManyCandidates)`; no process spawned. |
| Empty candidate list | `Declined(EmptyCandidates)`; no process spawned. |
| Duplicate candidate id in the request | `Declined(DuplicateCandidateId)`; no process spawned. |
| Candidate text over `max_candidate_text_bytes` | `Declined(CandidateTextTooLarge)`; no process spawned. |
| Child never reads stdin | Writer thread gets `EPIPE`; `input_truncated` is set; the run still completes on the child's stdout and exit status. A child that answers from a canned payload is legitimate. |
| Child writes its whole answer before reading stdin | Completes. Both pipes are drained concurrently, so neither fills. |
| Child closes stdout but stays alive | `Declined(TimedOut)` at the deadline; child killed and reaped. |
| Child exits but a descendant holds stdout | `Declined(TimedOut)` at the deadline; the direct child is reaped; the reader thread is left detached and unjoined. An incomplete, never-EOF'd payload is never parsed. |
| Child never exits and never writes | `Declined(TimedOut)` at the deadline; killed and reaped. |
| Child writes more than `max_stdout_bytes` | The reader reports overflow as soon as the cap is passed; the child is killed and reaped without waiting for the deadline; `Declined(OutputTooLarge)`. |
| Child writes unbounded stderr | Drained to EOF so back-pressure can never stall it, but only the first `max_stderr_bytes` are retained. Never an error by itself. |
| Non-zero exit | `Declined(NonZeroExit { code })` even when stdout parsed cleanly. A response is applied only after full validation *and* a zero exit. |
| Killed by a signal | `code` is `None`; still `Declined(NonZeroExit)`. |
| Empty stdout, zero exit | `Declined(Malformed)` from the JSON parser (EOF while parsing a value). |
| Stdout is not UTF-8 | `Declined(Malformed)`; the bytes are parsed as JSON directly, so invalid UTF-8 surfaces as a parse error, never a panic. |
| Unknown JSON key in the response | `Declined(Malformed)` — `deny_unknown_fields`. |
| `score_direction` is an unknown string | `Declined(Malformed)` — unknown enum variant. |
| Response echoes a wrong version / id / model / adapter | `Declined(VersionMismatch / RequestIdMismatch / ModelMismatch / AdapterMismatch)`. |
| Response omits, repeats or invents a candidate id | `Declined(MissingScores / DuplicateScore / UnknownScore)`. |
| Response score is non-finite | `Declined(NonFiniteScore)`. |
| All scores equal | Applied, in exactly the submitted order (rank tie-break). |
| `lower_is_better` | Applied ascending by score, ties still by ascending rank — the rank comparison is never reversed. |
| Any `Declined` | `original_order` carries every submitted id in submitted rank order, so the caller can restore the complete original ranking without holding its own copy. |
| Concurrent runs | `ProcessRunner` and `RerankCommand` are immutable and `Send + Sync`; `run` borrows `&self` and owns all per-run state, so two threads may run the same configuration simultaneously. |

### Preserved embedding behavior

The `embed.rs` refactor keeps: the `sh -c <cmd>` contract, `EMBED_TIMEOUT`
(10 s), the `embed_query` / `embed_query_with_timeout` signatures, tolerance of
a broken stdin pipe, `Error::Config` with the `embed-cmd <phase> failed: {e}`,
`embed-cmd exited with {status}` and `embed-cmd timed out` wordings, and the
`{"embedding":[..]}` payload parse through `embedding_input`. Two things
change, both strictly safer and both documented: the 10-second budget now spans
the whole run instead of the stdout read alone, and stdout is capped at
`ProcessLimits::default().max_stdout_bytes` (8 MiB, the same ceiling
`vector_stdin` already applies to a caller-supplied vector) instead of being
unbounded. stderr stays discarded from the caller's point of view — it is now
drained rather than sent to `/dev/null`, which cannot change an embedder's
output but does stop a chatty one from blocking on a full stderr pipe.

## Acceptance criteria

- **AC-1:** A real `/bin/sh` child that reads the JSON request from stdin,
  extracts `request_id` from it, and answers with a valid response scoring the
  second of two candidates higher is `Applied`, and `order_ids()` returns the
  second candidate's id first. (Issue AC 1.)
- **AC-2:** A request built by `RerankRequest::new` carries
  `protocol_version == 1`, a `request_id` matching `rr-<yyyymmdd>-<8hex>`, the
  caller's model and adapter, and candidates whose `rank` equals their index;
  the serialized JSON has exactly the keys named in "Wire protocol". (Issue
  AC 1.)
- **AC-3:** Each of these responses is `Declined` with the matching typed
  failure, driven through a real child: wrong `protocol_version`, wrong
  `request_id`, wrong `model`, wrong `adapter`, a missing candidate id, a
  duplicated candidate id, an unknown candidate id, malformed JSON, and a
  zero-exit child whose stdout is empty. A non-finite score is `Declined` via
  `validate` called directly on a hand-built response, because JSON has no
  literal for one. (Issue AC 2.)
- **AC-4:** No `Declined` case ever yields a reordered list: in every case in
  AC-3, `order_ids()` equals the submitted order and `is_applied()` is false.
  (Issue AC 2 and AC 6.)
- **AC-5:** `ProcessRunner` with a 300 ms budget returns `TimedOut` in well
  under 3 seconds against each of `sh -c 'sleep 30'` (never exits, never
  writes), `sh -c 'exec >&-; sleep 30'` (stdout closed, process alive) and
  `sh -c 'sleep 30 & exit 0'` (exited, descendant holds the pipe); and after
  all three runs, `ps -e -o ppid= -o stat=` reports **no** process whose parent
  pid is this test process and whose state begins with `Z`. That is an
  observation of the operating system's process table, not of the code's own
  control flow, so it proves the kill-and-reap actually happened. (Issue AC 3
  and AC 4.)
- **AC-6:** A child that writes 512 KiB to stdout *before* draining a 4 MiB
  stdin completes successfully inside its budget — the case that deadlocks
  `embed.rs`'s current sequencing. (Issue AC 4.)
- **AC-7:** A child that closes stdin without reading (`exec <&-`) while the
  parent writes 4 MiB still returns its stdout, with
  `ProcessOutput::input_truncated == true`. (Issue AC 4.)
- **AC-8:** A request whose serialized size exceeds `max_request_bytes`, and
  one with more than `max_candidates` candidates, are both `Declined` without
  spawning anything — proven by pointing the command at a program that would
  create a sentinel file and asserting the file does not exist. (Issue AC 3.)
- **AC-9:** A child writing more than `max_stdout_bytes` fails with
  `StdoutTooLarge` and returns well before the deadline; a child writing far
  more than `max_stderr_bytes` to stderr *and* a valid response to stdout still
  succeeds, with `ProcessOutput::stderr.len() <= max_stderr_bytes` — proving
  the excess is drained rather than left to back-pressure the child. At the
  rerank level the same chatty-stderr child yields `Applied` with
  `stderr_excerpt.len() <= max_stderr_bytes`. (Issue AC 3.)
- **AC-10:** A response whose scores are all equal is `Applied` in exactly the
  submitted order; a `lower_is_better` response orders ascending by score with
  ties still resolved by ascending `rank`. (Issue AC 6.)
- **AC-11:** `rerank` never interpolates text into a shell: a query and a
  candidate text containing `"; touch <tmp>/pwned; #` and
  `$(touch <tmp>/pwned)`, where `<tmp>` is the test's own temporary directory,
  run against `/bin/sh` as the *program* leave no `pwned` file, and the request
  the child recorded from stdin contains both injected substrings verbatim.
  (Issue AC 5.)
- **AC-12:** Every `embed.rs` test that passes on `main` passes unchanged after
  the refactor, and a new test proves the budget is now end-to-end: `sh -c 'cat
  > /dev/null; printf "{\"embedding\":[1.0]}"; sleep 30'` — which writes a
  valid payload and then refuses to exit — fails with `embed-cmd timed out`
  promptly instead of hanging in `wait()`. (Issue AC 8.)
- **AC-13:** The same suite passes on two of the three supported targets
  without a `cfg` branch: locally on `aarch64-apple-darwin` (macOS `sh`) and in
  CI on `x86_64-unknown-linux-gnu` (Debian `dash`). `aarch64-unknown-linux-gnu`
  is covered by the absence of any architecture-dependent construct — the new
  files add no `unsafe` block (so the `no-unsafe-without-safety` guardrail has
  nothing to report), no `cfg(target_os)`, and `Cargo.toml` gains no
  dependency, which `cargo machete` and an empty `git diff Cargo.toml` both
  confirm. (Issue AC 7.)
- **AC-14:** Nothing on the search path changes: `git diff --name-only
  origin/main` touches no file under `src/domains/`, `src/cli/`, `src/serve/`,
  `src/config/`, `src/store/` or `migrations/`, and adds no environment
  variable or CLI flag. (Non-goal 2.)
- **AC-15:** One `RerankRunner` value drives two concurrent requests from two
  threads and both come back `Applied` with their own request ids and orders —
  proving `run` owns all per-run state and that `&self` is genuinely shared.
  (Failure-modes table, "Concurrent runs".)

## Acceptance evidence

| AC | Real input / fixture | Expected observable result | Boundary or failure case | Runnable check |
| --- | --- | --- | --- | --- |
| AC-1 | A POSIX `sh` responder script written to a `tempfile` dir, driven with two real candidate ids | `RerankOutcome::Applied`, `order_ids() == ["memory:bbbbbbbb", "memory:aaaaaaaa"]` | Child reads the whole request before answering | `cargo nextest run --all-features rerank_runner` |
| AC-2 | `RerankRequest::new("bge-reranker-base", None, "bounded subprocess", …, fixed `OffsetDateTime`)` | `is_valid_request_id` true; `serde_json::to_value` has exactly the six documented request keys and three candidate keys | Adapter `Some("lora-v1")` serializes as a string, `None` as `null` | `cargo nextest run --all-features rerank_protocol` |
| AC-3 | Nine responder scripts, one per rejection, plus one hand-built `RerankResponse` with `f64::NAN` | The matching `RerankFailure` variant each time | Empty stdout with exit 0 | `cargo nextest run --all-features rerank_runner rerank_validate` |
| AC-4 | The AC-3 fixtures | `order_ids()` equals the submitted id order; `is_applied()` false | Every declined variant, not just one | same as AC-3 |
| AC-5 | `sh -c 'sleep 30'`, `sh -c 'exec >&-; sleep 30'`, `sh -c 'sleep 30 & exit 0'`, 300 ms budget | `ProcessFailure::TimedOut`; elapsed < 3 s; `ps -e -o ppid= -o stat=` shows no `Z` child of `std::process::id()` | Descendant holding the pipe | `cargo nextest run --all-features process_runner` |
| AC-6 | `sh -c 'cat "$1"; cat > /dev/null'` with a 512 KiB file argument and a 4 MiB stdin | `Ok(ProcessOutput)` with the 512 KiB stdout, exit 0 | Payloads far above both pipe buffers | `cargo nextest run --all-features process_runner` |
| AC-7 | `sh -c 'exec <&-; printf hello'` with a 4 MiB stdin | `stdout == b"hello"`, `input_truncated == true` | `EPIPE` mid-write | `cargo nextest run --all-features process_runner` |
| AC-8 | Limits of `max_request_bytes = 64` and `max_candidates = 1`; program `sh -c 'touch "$1"'` with a sentinel path | `Declined(RequestTooLarge)` / `Declined(TooManyCandidates)`; sentinel absent | Refusal must precede the spawn | `cargo nextest run --all-features rerank_runner` |
| AC-9 | `sh -c 'yes x \| head -c 200000'` with `max_stdout_bytes = 1024`; and a responder that also writes 200 000 bytes to stderr, with `max_stderr_bytes = 64` | `ProcessFailure::StdoutTooLarge`, elapsed far below the budget; the chatty-stderr run succeeds with `stderr.len() == 64`, and `rerank` reports `Applied` | Back-pressure on a full stderr pipe | `cargo nextest run --all-features process_runner rerank_runner` |
| AC-10 | A responder emitting equal scores, and one emitting `"lower_is_better"` | Submitted order preserved; ascending-by-score order with rank tie-break | Exact float equality | `cargo nextest run --all-features rerank_validate` |
| AC-11 | Query `"; touch <tmp>/pwned; #` and candidate text `$(touch <tmp>/pwned)`; the responder copies the bytes it read from stdin into `<tmp>/seen.json` | `<tmp>/pwned` does not exist; `<tmp>/seen.json` contains both substrings verbatim | Shell metacharacters in both query and text | `cargo nextest run --all-features rerank_runner` |
| AC-12 | The six existing `src/utilities/tests/embed.rs` tests, plus the new hang-after-answer case | All pass unmodified; the new one errors with `embed-cmd timed out` in under 3 s | Valid payload followed by a refusal to exit | `cargo nextest run --all-features embed` |
| AC-13 | The whole new suite, on macOS `sh` locally and Debian `dash` in CI | Green on both; guardrails reports no `unsafe`; `git diff Cargo.toml` empty; `cargo machete` clean | Two different POSIX shells | `bash scripts/check-all.sh`, the GitHub `test` workflow, `bash scripts/machete-check.sh` |
| AC-14 | `git diff --name-only origin/main` | No path under `src/domains/`, `src/cli/`, `src/serve/`, `src/config/`, `src/store/`, `migrations/` | — | `git diff --name-only`, recorded in the PR body |
| AC-15 | One `RerankRunner`, two `std::thread::scope` threads, two distinct requests against the same responder script | Both `Applied`; each `request_id` echoes its own request; orders are independent | Shared `&self` across threads | `cargo nextest run --all-features rerank_runner` |

Full gate: `bash scripts/check-all.sh`, `cargo nextest run --all-features`,
`bash scripts/dup-check.sh`, `bash scripts/deny-check.sh`,
`bash scripts/machete-check.sh`.

## Documentation impact

- **New:** this design document, `docs/designs/2026-09-18-reranker-command-protocol.md`,
  whose `## Wire protocol` section is the contract #212, #213 and #214 consume.
- **`docs/README.md`:** add the design under `## Explanation`, beside the
  sync-daemon and domain-first-migration entries.
- **`src/utilities/README.md`:** seven new rows in the Contents table; the
  `embed.rs` and `query_id.rs` rows updated to name what they now run on.
- **`src/utilities.rs`:** seven `mod` declarations, each with its `///` line.
- **`AGENTS.md`:** the `utilities/` module-map row gains the new primitives,
  and the `embed.rs` row records that its budget is now end-to-end.
- **`README.md` (root), `docs/cli-reference.md`, `docs/scenarios/`,
  `docs/configuration.md`:** unchanged — no CLI subcommand, no flag, no
  environment variable, no config key and no HTTP route is added or altered.
  `scripts/cli-docs-check.sh` and `tests/cli_scenario_catalog.rs` therefore
  have nothing to regenerate.
- **`migrations/`, `src/store/schema_*.rs`:** unchanged — nothing is persisted.

## Open Questions

1. **Default timeout of 20 s.** Owner: this issue, resolved. Embedding's 10 s
   is one short forward pass; a cross-encoder over a few hundred candidates on
   CPU is longer. 20 s is a ceiling that still fails an interactive search
   fast. #213 makes it configurable when it wires a search surface. Not
   blocking.
2. **Default `max_candidates` of 256.** Owner: this issue, resolved. The
   retrieval pool is capped by `COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW` (default
   200), so 256 clears today's deepest pool with headroom while still bounding
   a caller that passes an unbounded list. #213 may lower it. Not blocking.
3. **`deny_unknown_fields` on the response.** Owner: this issue, resolved.
   Strict is correct for a versioned protocol whose version is an explicit
   integer: an additive field is a version bump, not a silent extension. Not
   blocking.
4. **Should the runner kill the child's whole process group?** Owner: this
   issue, resolved as Non-Goal 6. It requires `libc` plus an `unsafe` block
   (which D1's SAFETY rule then governs), and `kill(-pgid)` without
   `process_group(0)` can signal processes comemory never started, while
   `process_group(0)` changes terminal signal delivery for `embed`'s existing
   users. The AC asks to "kill/reap **owned** processes", which the direct
   child is. Not blocking; recorded so #213 can revisit with operational
   evidence.
5. **Does `serde_json` yield a non-finite `f64` for an overflowing literal, or
   reject it?** Owner: this issue, **resolved by measurement** on
   `serde_json 1`: it rejects one outright — `from_str::<f64>("1e400")` and
   `("1e309")` both return `Err("number out of range")`. A non-finite score
   therefore cannot arrive over this wire at all, and an overflowing literal
   surfaces as `Malformed`. The finiteness check stays unconditional anyway,
   as defence against a future transport, and is tested by calling `validate`
   on a hand-built response holding `NaN`, `+Inf` and `-Inf` — which no
   subprocess could produce. Not blocking.
