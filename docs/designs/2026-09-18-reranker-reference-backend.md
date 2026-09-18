# Optional Python reference reranker backend — Design

**Date:** 2026-09-18   **Status:** Approved   **Author:** Auto
**Topic:** A pinned, optional Python cross-encoder backend that speaks the
#211 reranker command protocol, loads a PEFT adapter when one is configured,
and can be proven conformant without downloading a model.

Issue: [#212](https://github.com/Falconiere/comemory/issues/212).
Tracking issue: [#207](https://github.com/Falconiere/comemory/issues/207).
Dependency: [#211](https://github.com/Falconiere/comemory/issues/211), merged —
`docs/designs/2026-09-18-reranker-command-protocol.md` § Wire protocol is the
binding contract this backend implements verbatim.

## Problem

#211 delivered the wire contract and the bounded runner, and deliberately
shipped no scorer. Without one reference implementation there is nothing to
compare an unadapted model against a LoRA adapter with, nothing for
[#213](https://github.com/Falconiere/comemory/issues/213) to point a retrieval
surface at, and nothing for
[#214](https://github.com/Falconiere/comemory/issues/214) to train against.

Three properties make that reference implementation hard to deliver honestly:

- **It must not leak into the Rust binary.** comemory is a standalone Rust CLI
  with no in-process model. A Python or model dependency in `Cargo.toml`, in
  the binary, or on the normal `cargo nextest` path would undo that.
- **Protocol conformance must be provable in the normal test path.** A test
  that needs a multi-hundred-megabyte download to run is a test that does not
  run, and a suite that reports green because the download was absent is worse
  than no suite at all — the repository's recurring failure mode.
- **A sequence-classification adapter can silently produce garbage.** PEFT's
  own troubleshooting guide records it: Transformers attaches a randomly
  initialized classification head, and a LoRA checkpoint that omits that head
  from `modules_to_save` reloads a *different* random head every time, giving
  confidently wrong scores with no error anywhere.

## Non-Goals

1. **No retrieval or CLI integration.** No search surface calls this backend,
   no `[rerank]` configuration section, no environment variable, no CLI flag
   and no `docs/cli-reference.md` entry. That is #213.
2. **No dataset capture, export or judgment work.** #209 and #210 own those.
3. **No training recipe and no training run.** #214 owns LoRA training; this
   change ships the loader and the compatibility validation it will need, and
   never trains anything.
4. **No change to the #211 wire protocol.** Not one field, not one rule. An
   observation about a protocol limitation is recorded under Open Questions
   rather than acted on.
5. **No model download and no benchmark numbers in this change.** Every model
   command is documented and copy-pasteable; none is executed here. The
   performance table ships with explicitly unmeasured rows rather than
   invented ones.
6. **No Rust production code.** The only Rust this change adds is one colocated
   test file and the one-line `mod` include that names it.
7. **No Windows support**, matching #211: the three cargo-dist targets are all
   Unix, and the warm transport is a Unix domain socket.
8. **No packaging of the backend into the released binary.** It is a repository
   asset under `integrations/`, not an embedded bundle like
   `integrations/agent/`, and `src/domains/integrations/install/bundle.rs` is
   untouched.

## Architecture

### Where the code lives

`integrations/` already holds the one shipped non-Rust component that is a
*component* rather than a repository gate — `integrations/agent/`, the embedded
plugin. `scripts/` holds repository gates plus one user-facing wrapper,
`scripts/comemory-embed.sh`, which is 40 lines of shell with no files of its
own. A reference inference backend is the first shape, not the second, so it
becomes a sibling of `agent/`:

```
integrations/reranker/
  comemory_rerank.py            entry point: argparse, dispatch, exit codes
  comemory_rerank_pins.py       every pinned constant, in one file
  comemory_rerank_protocol.py   request decode / response encode / identity gate
  comemory_rerank_lexical.py    the deterministic non-neural scorer
  comemory_rerank_model.py      the Transformers + PEFT cross-encoder scorer
  comemory_rerank_compat.py     whether a checkpoint may be scored with at all
  comemory_rerank_warm.py       the warm Unix-socket server and its thin client
  comemory_rerank_preflight.py  the opt-in suite's prerequisite check
  requirements.txt              the pinned dependency set for the model path
  run-model-tests.sh            preflight + runner for the opt-in model suite
  README.md                     install, invocation, lifecycle, timeouts, fallback
  tests/test_model_backend.py   the opt-in model suite
```

Flat modules with a shared `comemory_rerank_` prefix, mirroring the Rust side's
"no barrels, one responsibility per file" rule. There is no `__init__.py` and no
package: a package `__init__.py` re-exporting siblings is the Python spelling of
the barrel this repository bans. The entry point puts its own resolved directory
on `sys.path` before importing its siblings, so the backend runs correctly from
any working directory and through a symlink.

No file exceeds the repository's 300-code-line ceiling, so the guardrails
`file-size` check gives the same verdict on these paths that it gives on Rust.
The scorer was written as one file first and measured at 448 code lines — the
counter does not treat a Python docstring as a comment — so it took exactly the
split this section anticipated: checkpoint and adapter validation moved to
`comemory_rerank_compat.py`, leaving tokenization and the forward pass behind.
`comemory_rerank_preflight.py` is the second file the first draft did not have:
the opt-in suite's prerequisite check is Python rather than shell so the pinned
versions it enforces are read from `comemory_rerank_pins.REQUIREMENTS` instead
of being retyped.

### The two scoring modes

`--scoring` selects one of two implementations behind one protocol:

| Mode | What it is | Dependencies | Purpose |
| --- | --- | --- | --- |
| `cross-encoder` (default) | The pinned Transformers sequence-classification reranker, with optional PEFT adapter loading | torch, transformers, peft, safetensors, huggingface-hub | The real scorer |
| `lexical-overlap` | A pinned, deterministic, non-neural token-overlap function | Python standard library only | Protocol conformance, provable with no download |

`lexical-overlap` exists so that the real subprocess, the real JSON on real
pipes, the real framing, the real identity gate and the real Rust
`RerankRunner` are all exercised on every `cargo nextest run`. It is **not** a
quality baseline and must never be read as one: it answers to the model label
`lexical-overlap@1`, which no request expecting a real model can carry, and its
fingerprint declares `"scoring_is_neural": false` with a `note` saying in words
that it measures no relevance quality.

Its definition is pinned, so a test can compute an expected order by hand:

```python
TOKEN_RE = re.compile(r"[a-z0-9]+")
tokens(s) = set(TOKEN_RE.findall(s.lower()))
score(query, text) = |tokens(query) & tokens(text)| / |tokens(query) | tokens(text)|
```

with `0.0` for an empty union. That is Jaccard similarity over lowercased
alphanumeric token sets. `score_direction` is `higher_is_better`.

### The pinned base model

`cross-encoder/ms-marco-MiniLM-L6-v2`, revision
`233902d25c440f23af6f7d6e94d2946bac0bee0a`.

Selected against the issue's constraint — small, a genuine reranker, not a
large generative model — on criteria that can be checked from the Hub metadata
without downloading weights, all verified on 2026-09-18:

| Criterion | Value | Why it decides |
| --- | --- | --- |
| Architecture | `BertForSequenceClassification` | The `AutoModelForSequenceClassification` path the issue names, with no `trust_remote_code` |
| Size | 6 layers, hidden 384, ~22.7M parameters, ~90 MiB of fp32 weights | Loads on a laptop CPU in seconds; a reranker, not an LLM |
| Head | `num_labels = 1`, `id2label = {0: "LABEL_0"}` | One relevance logit per pair, which is exactly what the protocol's `score` is |
| Training | MS MARCO passage ranking | Already a trained reranker, so base-only scoring is meaningful on day one |
| Context | `max_position_embeddings = 512` | Matches the protocol's 8 KiB candidate-text cap comfortably |
| Licence | Apache-2.0 | Redistributable, and clean for `scripts/deny-check.sh`'s sibling concerns |
| LoRA surface | Standard BERT `query` / `key` / `value` / `dense` module names | PEFT's `target_modules` inference works without a custom map |

The considered alternative, `BAAI/bge-reranker-base`, is
`XLMRobertaForSequenceClassification` at 278M parameters — twelve times larger
for a multilingual capability comemory does not need, under MIT rather than
Apache-2.0. It is recorded here as the documented upgrade path, not the
default.

**Compatibility is validated, not assumed.** The selection above is made on
published metadata. The *measured* confirmation — that these pinned library
versions load these pinned weights and that a PEFT `SEQ_CLS` adapter round
trips through them — is exactly what the opt-in model suite performs, and the
maintainer's scope decision for this issue is to ship that suite without
running it. § Acceptance evidence marks every such row explicitly.

### Pinned inference configuration

Every knob that can change a score is a constant in `comemory_rerank_pins.py`
and appears in the fingerprint:

| Knob | Pinned default | Flag |
| --- | --- | --- |
| Model id | `cross-encoder/ms-marco-MiniLM-L6-v2` | `--model-id` |
| Model revision | `233902d25c440f23af6f7d6e94d2946bac0bee0a` | `--model-revision` |
| Tokenizer id / revision | the same repository and revision | `--tokenizer-id`, `--tokenizer-revision` |
| dtype | `float32` | `--dtype {float32,float16,bfloat16}` |
| Device | `cpu` | `--device {cpu,cuda,mps,auto}` |
| Maximum length | `512` | `--max-length` |
| Truncation | `longest_first` | `--truncation {longest_first,only_second}` |
| Padding | `longest` | `--padding {longest,max_length}` |
| Batch size | `16` | `--batch-size` |
| Score column | `0`, with `num_labels == 1` required | `--score-column` |
| Score direction | `higher_is_better` | not configurable |
| Head modules | `["classifier"]` | `--head-module` (repeatable) |
| Network | off — `local_files_only=True` | `--allow-download` |

`padding=longest` pads to the longest sequence in a batch; padded positions are
masked, so batch composition cannot change a score beyond floating-point
reassociation. `--padding max_length` is the bit-reproducibility lever and is
documented as roughly an order of magnitude slower.

Inference always runs under `model.eval()` and `torch.inference_mode()`, and
base-only and adapted inference share one code path, so preprocessing,
batching, truncation and pooling are identical by construction rather than by
review.

**Network access is off by default.** A model download inside a request would
turn a 20-second rerank budget into a multi-minute stall, so
`local_files_only=True` is the default and a missing snapshot fails fast with
`EX_UNAVAILABLE` and the exact `huggingface-cli download` line to run.

### The identity gate

The protocol's `model` and `adapter` are identity labels the caller expects and
the response must echo byte-for-byte. A backend that echoed any label it was
handed would let a caller asking for `adapter=lora-v3` receive `lora-v1` scores
under the `lora-v3` name, and nothing on the wire could detect it.

So the backend answers only to its own configured identity:

- `--model-label`, defaulting to `"{model_id}@{model_revision}"` for
  `cross-encoder` and to `"lexical-overlap@1"` for `lexical-overlap`.
- `--adapter-label`, defaulting to the adapter directory's basename, and
  `None` when no `--adapter` is configured.

A request whose `model` or `adapter` differs from those labels is refused with
`EX_DATAERR` and a stderr diagnostic naming both sides. The Rust runner sees
`RerankFailure::NonZeroExit`, declines, and the caller's deterministic ranking
stands unchanged. Fail-closed, and observable.

### Never a fresh relevance head

Both directions of the "do not silently initialize a fresh relevance head"
requirement are enforced by reading the checkpoint's own tensor names, which is
independent of any Transformers version's loading-report shape:

**Base checkpoint.** After the model is instantiated, every parameter whose
name's first segment is one of `--head-module` must be present in the resolved
weight file's tensor keys. A base repository with no trained head therefore
fails with `EX_DATAERR` rather than scoring with random weights.

**Adapter checkpoint.** `adapter_config.json` is read before anything is
loaded and must satisfy all of:

1. `peft_type == "LORA"`.
2. `task_type == "SEQ_CLS"`.
3. `base_model_name_or_path` equals the configured `--model-id`, and
   `revision`, when non-null, equals `--model-revision`. `--allow-base-mismatch`
   downgrades this to a loud stderr warning for a deliberately retargeted
   adapter.
4. `modules_to_save` is non-empty and contains every `--head-module`. This is
   the check that catches the PEFT failure mode by name, and its diagnostic
   says so.
5. `adapter_model.safetensors` (or `adapter_model.bin`) contains, for each head
   module `M`, at least one tensor key containing `.{M}.modules_to_save.`.

Only then is `PeftModel.from_pretrained` called. After loading, a head
parameter that is bit-identical to the base model's produces a stderr warning
— a frozen head is a legitimate recipe, so it is not an error — and a
non-finite head parameter is a hard `EX_SOFTWARE` failure.

### Warm local inference

A cold `cross-encoder` start pays interpreter startup, torch import and weight
loading on every request. The protocol is deliberately one child process per
request, and this design does not change that. Instead the heavy work moves
behind a local socket:

```
comemory_rerank.py serve  --socket /run/user/1000/comemory-rerank.sock [--ready-file F] [--idle-timeout S]
comemory_rerank.py client --socket /run/user/1000/comemory-rerank.sock
```

`serve` loads and validates the model once, writes its fingerprint and a
`ready` line to stderr, optionally touches `--ready-file`, and then serves
connections serially — serially because the model is the bottleneck and
concurrent requests would multiply resident memory without shortening the
queue.

`client` imports only `socket`, `struct`, `sys` and `argparse`, so its startup
is interpreter startup alone. It is what #213 would point `RerankRunner` at:
still a real child process, still one JSON object on stdin and one on stdout,
still exit 0 — the #211 contract, unchanged.

The socket carries a small framed transport that is **an implementation detail
of the warm option and explicitly not the #211 wire protocol**:

```
frame := kind:u8 | code:u8 | length:u32be | payload[length]
kind 1  request / response   code 0, payload is the protocol JSON object
kind 2  error                code is the sysexits code, payload is a UTF-8 diagnostic
```

Framing is needed because a socket has no EOF-per-message, and the error kind
is needed because the #211 response object has no error representation — a
refusal has to reach the client as an exit code, and this is how the server
tells the client which one to exit with.

**Lifecycle and cost are documented separately from query latency**, as the
issue requires: startup is one model load, measured and reported by `serve`
itself as `load_ms` on its ready line; steady-state per-query cost is what
`benchmark` reports. `--idle-timeout` (default `0`, never) lets an operator
reclaim the memory. The socket is created under a `0o077` umask, refuses to
replace a socket another live server is listening on, and is removed on exit.

### Fallback to base scoring

The backend **never** substitutes one identity for another in-process. Falling
back from adapter to base inside one process would mean answering a request
that declared `adapter: "lora-v1"` with base-model scores while echoing
`"lora-v1"`, which the protocol cannot express and which no caller could
detect.

The documented fallback ladder is therefore, in order:

1. The adapter fails validation or loading → the backend exits non-zero with a
   diagnostic. `RerankRunner` returns `Declined`, and `RerankOutcome::order_ids`
   yields the submitted order, so comemory's own deterministic ranking is used
   unchanged. This is the guaranteed no-regression path and it needs no
   configuration.
2. To actually serve base-model scores, the operator points the runner at a
   second invocation with no `--adapter`, and the caller sends `adapter: null`.
   Choosing between the two at runtime is #213's configuration work.

## Interfaces / Schema

### Command line

```
comemory_rerank.py score      [--scoring M] [model/adapter/inference flags]
comemory_rerank.py serve      --socket PATH [--ready-file PATH] [--idle-timeout S] [...]
comemory_rerank.py client     --socket PATH [--connect-timeout S] [--request-timeout S]
comemory_rerank.py fingerprint [--scoring M] [...]
comemory_rerank.py benchmark  [--scoring M] [--candidates N] [--repeat R] [...]
```

- `score` reads one JSON request from stdin to EOF and writes one JSON response
  to stdout, then exits 0. This is the #211 contract.
- `client` does the same, with the scoring delegated over the socket.
- `fingerprint` writes one JSON object to stdout and exits 0. It reads nothing.
- `benchmark` writes one JSON report to stdout: `load_ms`, per-query latency
  percentiles over `--repeat` runs of a `--candidates`-sized request, and peak
  resident memory from `resource.getrusage(RUSAGE_SELF).ru_maxrss`. It reads
  nothing from stdin: the request is generated in-process from a fixed query
  and a fixed sentence template numbered by candidate index, so two runs on one
  machine measure the same work and the report is reproducible.

`client` takes **only** `--socket` and the two timeouts. It carries no
`--scoring`, `--model-id`, `--model-revision`, `--adapter` or inference flag,
because every one of those is the server's configuration: duplicating them on
the client would create a second place for the identity to drift. The identity
gate therefore runs on the server, and its refusal reaches the client as the
error frame described above.

**Diagnostics.** `score` and `client` are silent on success — stdout carries the
response and nothing is written to stderr — so a happy-path `stderr_excerpt` is
empty. `--verbose` adds one `comemory-rerank: fingerprint <json>` line to
stderr before scoring. `serve` always writes two kinds of line to stderr, and
they are the only stderr output it produces in normal operation:

```
comemory-rerank: fingerprint {"backend":"comemory-rerank",...}
comemory-rerank: ready socket=<path> load_ms=<int>
comemory-rerank: served request_id=<id> candidates=<int> score_ms=<int>
```

one `fingerprint` line and one `ready` line at startup, then exactly one
`served` line per completed request. That is what makes "the model was loaded
once and used twice" an observation of the process rather than of the code's
own control flow.

### Exit codes (`sysexits.h`, as everywhere else in this repository)

| Code | Name | When |
| --- | --- | --- |
| `0` | — | One valid response written to stdout |
| `64` | `EX_USAGE` | Bad command line |
| `65` | `EX_DATAERR` | Unparsable request, unsupported `protocol_version`, model or adapter identity mismatch, adapter or base compatibility failure |
| `69` | `EX_UNAVAILABLE` | A dependency is not importable, the pinned snapshot is absent under `local_files_only`, or the warm socket cannot be reached |
| `70` | `EX_SOFTWARE` | Scoring raised, or produced a non-finite score |
| `73` | `EX_CANTCREAT` | The warm socket path cannot be bound, or another server already owns it |
| anything else | — | An unhandled internal error, with a traceback on stderr. Every refusal the backend knows about is one of the codes above, so this is a bug in the backend rather than a contract |

Every non-zero exit writes exactly one `comemory-rerank: <message>` line to
stderr. stderr is diagnostic only and is never parsed by the Rust side; it
surfaces as `RerankApplied::stderr_excerpt` / `RerankDeclined::stderr_excerpt`.

### Request handling, in order

1. Read stdin to EOF, before writing a single byte to stdout. That ordering is
   what keeps `ProcessOutput::input_truncated` false on every path here: the
   backend never fills the stdout pipe while the parent is still writing, so
   the parent's write always completes and no `EPIPE` is raised.
   Empty input → `EX_DATAERR`.
2. `json.loads`. Not an object, or a parse error → `EX_DATAERR`.
3. Exactly the six documented keys, no more and no fewer → `EX_DATAERR`.
   The backend mirrors `deny_unknown_fields` on the request, so a caller that
   invented a field learns immediately rather than being silently ignored.
4. `protocol_version == 1` → otherwise `EX_DATAERR`, as § Wire protocol's
   versioning rule requires ("should exit non-zero with a diagnostic on stderr
   rather than answer").
5. `request_id` matches `rr-<yyyymmdd>-<8 lowercase hex>` → otherwise
   `EX_DATAERR`.
6. `model` equals the configured model label and `adapter` equals the
   configured adapter label → otherwise `EX_DATAERR`.
7. `candidates` is a non-empty list of `{id, rank, text}` objects with unique
   ids → otherwise `EX_DATAERR`.
8. Score every candidate, in submitted order.
9. Write exactly the six documented response keys, one score per candidate, in
   submitted order, with `score_direction: "higher_is_better"`. Exit 0.

The response never carries an extra key: § Wire protocol's `deny_unknown_fields`
makes any addition a protocol version bump, and the fingerprint is emitted on
stderr and through `fingerprint` instead.

### Fingerprint object

```json
{
  "backend": "comemory-rerank",
  "backend_version": 1,
  "protocol_version": 1,
  "scoring": "cross-encoder",
  "scoring_is_neural": true,
  "note": "...",
  "model_label": "cross-encoder/ms-marco-MiniLM-L6-v2@233902d2...",
  "model_id": "cross-encoder/ms-marco-MiniLM-L6-v2",
  "model_revision": "233902d2...",
  "tokenizer_id": "...", "tokenizer_revision": "...",
  "architectures": ["BertForSequenceClassification"],
  "num_labels": 1, "score_column": 0,
  "score_direction": "higher_is_better",
  "dtype": "float32", "device": "cpu",
  "max_length": 512, "truncation": "longest_first",
  "padding": "longest", "batch_size": 16,
  "weights_sha256": "...",
  "head_parameters": ["classifier.weight", "classifier.bias"],
  "adapter_label": null, "adapter_path": null,
  "adapter_base_model": null, "adapter_task_type": null,
  "adapter_peft_type": null, "adapter_modules_to_save": null,
  "adapter_config_sha256": null, "adapter_weights_sha256": null,
  "libraries": {"python": "3.12.4", "torch": "...", "transformers": "...",
                "peft": "...", "tokenizers": "...", "safetensors": "..."}
}
```

In `lexical-overlap` mode the model keys carry the lexical identity,
`scoring_is_neural` is `false`, `note` states that the mode measures no
relevance quality, and `libraries` holds `python` alone.

### Pinned dependency set — `integrations/reranker/requirements.txt`

```
torch==2.14.0
transformers==5.17.0
peft==0.21.0
tokenizers==0.23.2
safetensors==0.8.0
huggingface-hub==1.32.0
numpy==2.5.3
```

Verified against PyPI on 2026-09-18: `peft 0.21.0` (2026-09-15) postdates
`transformers 5.17.0` (2026-09-09), so it is the first PEFT release built
against that Transformers major; `transformers 5.17.0` requires
`huggingface-hub <2.0,>=1.5.0`, `tokenizers <0.24.0,>=0.23.1` and
`safetensors >=0.8.0`, all satisfied above. `numpy==2.5.3` requires Python
`>=3.12`, which is therefore the floor for the model path; the
`lexical-overlap` path is standard library only and runs on Python 3.9 and
later, which is what keeps the normal test path portable.

`integrations/reranker/run-model-tests.sh` treats these pins the way
`scripts/dup-check.sh` treats `similarity-rs`: an **absent** library or an
absent snapshot exits `EX_UNAVAILABLE` naming the fix, and a **version
mismatch** is an equally hard failure, because a result produced by another
build of torch or Transformers is not the result the pinned configuration
would produce. Neither case can exit 0.

These are exact `==` pins of the direct dependency set. Transitive resolution
is left to pip rather than enumerated, because enumerating a transitive set
without resolving it in the target environment would record guesses as pins.

### Rust-side test surface

`src/utilities/tests/rerank_runner_2.rs`, included from
`src/utilities/rerank_runner.rs` with the repository's standard split-suite
form:

```rust
#[cfg(test)]
#[path = "tests/rerank_runner_2.rs"]
mod tests_2;
```

No production Rust file is added, so
`docs/designs/2026-09-17-domain-first-migration-inventory.md` needs no new
**row**: `scripts/lib/architecture-inventory.sh` builds its file list with
`find src -type f -name '*.rs' ! -path '*/tests/*'`. Its existing
`src/utilities/rerank_runner.rs` row does need one cell changed, because the
same gate cross-checks every production file's `#[path]` bridges against the
row's `bridge` column, and that file now declares two. The edit is exactly:

```
| src/utilities/rerank_runner.rs | … | src/utilities/tests/rerank_runner.rs; src/utilities/tests/rerank_runner_2.rs | none | … |
```

## Failure modes and edge cases

| Case | Observable behavior |
| --- | --- |
| Request body over 8 MiB | `EX_DATAERR`. The backend enforces its own ceiling rather than trusting the runner's, because it is also driven by hand and by #214. |
| `python3` absent | The Rust conformance suite fails with a message naming `python3` as a prerequisite. It never skips: a skipped protocol test that reads as a pass is the failure mode this design exists to avoid. |
| Empty stdin | `EX_DATAERR`; the Rust side reports `NonZeroExit { code: Some(65) }`, and `order_ids()` is the submitted order. |
| Stdin is not JSON, or is a JSON array | `EX_DATAERR`. |
| Request carries an unknown key, or omits one | `EX_DATAERR`, naming the key. |
| `protocol_version` is not `1` | `EX_DATAERR`, naming both versions — the behavior § Wire protocol § Versioning prescribes. |
| `request_id` is not `rr-<yyyymmdd>-<8hex>` | `EX_DATAERR`. |
| `model` or `adapter` does not match the configured label | `EX_DATAERR`, naming expected and actual. |
| `candidates` is empty, or two candidates share an `id` | `EX_DATAERR`. The Rust runner already refuses both before spawning; the backend checks anyway, because it is also driven by hand and by #214. |
| Candidate text is empty | Scored normally. `lexical-overlap` gives `0.0` unless the query is also empty; the cross-encoder scores the pair as the tokenizer presents it. |
| Query and candidate text contain shell metacharacters | Scored as literal text. `ProcessRunner` passes program and arguments separately and the payload only ever crosses stdin, so there is no shell to inject into. |
| Candidate text longer than `--max-length` tokens | Truncated by the pinned `longest_first` policy. Byte-level bounding is the caller's, enforced by `RerankLimits::max_candidate_text_bytes`. |
| A dependency is missing (`torch`, `transformers`, `peft`) | `EX_UNAVAILABLE` at startup, naming the missing module and the `pip install -r` line. Never a traceback. |
| The pinned snapshot is absent and `--allow-download` was not passed | `EX_UNAVAILABLE`, naming the exact download command. |
| The base checkpoint has no trained head | `EX_DATAERR`, stating that scoring would use randomly initialized weights. |
| `adapter_config.json` omits the head from `modules_to_save` | `EX_DATAERR`, stating that PEFT would attach a fresh random relevance head. |
| The adapter's `base_model_name_or_path` disagrees with `--model-id` | `EX_DATAERR`, unless `--allow-base-mismatch`, which downgrades it to a stderr warning. |
| A head parameter survives loading bit-identical to the base | stderr warning, not an error — a frozen head is a legitimate recipe. |
| A score is `NaN` or infinite | `EX_SOFTWARE` before anything is written to stdout. The response is never partially emitted. |
| The warm socket does not exist, or nothing is listening | `client` exits `EX_UNAVAILABLE`; the Rust side declines; the original ranking stands. |
| `serve` is asked to bind a path a live server already owns | `EX_CANTCREAT`; the incumbent is never disturbed. |
| `serve` is asked to bind a path holding a stale socket | The stale socket is removed and the bind proceeds. |
| `client` request exceeds `--request-timeout` | `EX_UNAVAILABLE`. The default of 15 s sits below #211's 20 s `DEFAULT_RERANK_TIMEOUT` so the client produces a diagnostic rather than being killed mid-write. |
| The runner's deadline expires first | `ProcessRunner` kills and reaps the child; `Declined(TimedOut)`. A warm `serve` process is a separate process the runner does not own and is unaffected. |
| The server dies while a client is connected | The client's read returns a short frame; `EX_UNAVAILABLE`. |
| Two clients hit one warm server | Served serially, each with its own request id. |
| All scores equal | Applied in exactly the submitted order — #211's rank tie-break, which this backend exercises deliberately. |

## Acceptance criteria

- **AC-1:** `python3 integrations/reranker/comemory_rerank.py score --scoring
  lexical-overlap`, driven as a real child by a real `RerankRunner` over three
  real domain-qualified candidates, returns `Applied` with `order_ids()` equal
  to the order the pinned Jaccard definition predicts by hand, and echoes the
  request id, model label and `adapter: null`. (Issue AC 1, AC 7.)
- **AC-2:** The same invocation over candidates with identical text returns
  `Applied` in exactly the submitted order, proving the backend's per-candidate
  scores feed #211's rank tie-break rather than an order of its own. (Issue
  AC 1.)
- **AC-3:** A request whose `model` is not the backend's configured label is
  refused with exit `65`; `RerankOutcome` is `Declined(NonZeroExit { code:
  Some(65) })`, `order_ids()` equals the submitted order, and
  `stderr_excerpt` names both the expected and the received label. The same
  holds for a request carrying `adapter: Some(..)` against a base-only
  invocation. (Issue AC 2, AC 8.)
- **AC-4:** A hand-written request body with `protocol_version: 2`, an empty
  body, a body with an unknown key, and a body with a malformed `request_id`
  are each refused with exit `65` and a one-line stderr diagnostic, driven
  through the real `ProcessRunner`. (Issue AC 1.)
- **AC-5:** `fingerprint --scoring lexical-overlap` emits a JSON object whose
  `protocol_version` is `1`, `scoring` is `lexical-overlap`, `model_label` is
  `lexical-overlap@1`, and `scoring_is_neural` is `false` — so the deterministic
  mode is machine-identifiable as not-a-model. (Issue AC 2, AC 7.)
- **AC-6:** A real `serve --scoring lexical-overlap --socket <tmp>` process,
  started once, answers **two** consecutive `RerankRunner` requests through
  `client --socket <tmp>`, both `Applied` with their own request ids, and the
  server's captured stderr contains exactly one `ready` line and two `served`
  lines — the observable proof that repeated requests did not reload. (Issue
  AC 4.)
- **AC-7:** `client --socket <a path with no listener>` yields
  `Declined(NonZeroExit { code: Some(69) })` with the submitted order intact.
  (Issue AC 4, AC 8.)
- **AC-8:** A query and a candidate text containing `"; touch <tmp>/pwned; #`
  and `$(touch <tmp>/pwned)` are scored as literal text: the outcome is
  `Applied`, `<tmp>/pwned` does not exist afterwards, and the candidate
  carrying the metacharacters scores strictly higher than one sharing no
  tokens with the query. (Issue AC 1.)
- **AC-9:** `Cargo.toml` is byte-identical to `origin/main`, `cargo machete`
  is clean, and `git diff --name-only origin/main` touches no file under
  `src/` other than `src/utilities/rerank_runner.rs` and
  `src/utilities/tests/`. No Python or model dependency is reachable from the
  Rust binary. (Issue AC 6.)
- **AC-10:** `bash integrations/reranker/run-model-tests.sh` exits non-zero
  with a message naming each missing prerequisite — interpreter, pinned
  library versions, pinned snapshot — and never exits 0 when the model is
  absent. A library present at a version other than the pin is refused just as
  hard as an absent one. A skipped model test cannot read as a pass. (Issue
  AC 7.)
- **AC-11:** With the pinned model present, the opt-in suite loads it, asserts
  `architectures == ["BertForSequenceClassification"]` and `num_labels == 1`,
  asserts every head parameter came from the checkpoint, and scores a
  four-candidate request whose expected top-1 is the passage that answers the
  query. **Implemented, not measured in this change** — no download is
  performed here. (Issue AC 1, AC 2, AC 7.)
- **AC-12:** With the pinned model present, the opt-in suite builds a PEFT
  `SEQ_CLS` LoRA adapter with `modules_to_save=["classifier"]`, saves it
  without training, and asserts the backend loads it, echoes the adapter label
  and scores; then rewrites the saved `adapter_config.json` to drop
  `classifier` from `modules_to_save` and asserts the backend refuses with the
  fresh-head diagnostic. **Implemented, not measured in this change.** (Issue
  AC 3.)
- **AC-13:** `benchmark` reports `load_ms`, per-query latency percentiles and
  peak resident memory as JSON, and runs in `lexical-overlap` mode in the
  conformance suite so the harness itself is exercised. The pinned model's
  cold/warm latency, memory and accelerator rows in
  `integrations/reranker/README.md` are shipped **explicitly unmeasured**, each
  naming the command that fills it in. (Issue AC 5, honestly scoped.)
- **AC-14:** `bash scripts/check-all.sh` exits 0, `cargo nextest run
  --all-features` is green, and `dup-check` / `deny-check` / `machete-check`
  report no regression.

## Acceptance evidence

| AC | Real input / fixture | Expected observable result | Boundary or failure case | Runnable check |
| --- | --- | --- | --- | --- |
| AC-1 | Query `bounded subprocess deadline`; candidates `memory:aaaaaaaa` ("markdown is the source of truth", Jaccard `0.0`), `code:comemory:src/utilities/process_runner.rs:run` ("the bounded subprocess deadline", `3/4`), `document:notes.md#3` ("deadline", `1/3`) | `Applied`; `order_ids() == [code…, document…, memory:aaaaaaaa]` | Submitted order is deliberately not the scored order | `cargo nextest run --all-features rerank_runner_2` |
| AC-2 | Two candidates with byte-identical text plus one sharing no tokens | `Applied`; the two tied ids appear in submitted rank order | Exact float equality | same |
| AC-3 | The lexical backend asked for `model: "cross-encoder/ms-marco-MiniLM-L6-v2@…"`; and a base-only backend asked for `adapter: Some("lora-v1")` | `Declined(NonZeroExit { code: Some(65) })` twice; `order_ids()` unchanged; `stderr_excerpt` contains both labels | The label that would silently misattribute a score | same |
| AC-4 | Four hand-written bodies driven through `ProcessRunner::run` | exit `65` each, one stderr line each | Empty body; unknown key; bad id shape; future version | same |
| AC-5 | `fingerprint --scoring lexical-overlap` through `ProcessRunner` | The four asserted fields | A mode that must never be mistaken for a model | same |
| AC-6 | A real `serve` child with stderr captured to a file, a `--ready-file` poll, then two `RerankRunner` runs through `client` | Both `Applied`; one `ready` line; two `served` lines | The second request must not reload | same |
| AC-7 | `client --socket <tmpdir>/absent.sock` | `Declined(NonZeroExit { code: Some(69) })` | No listener at all | same |
| AC-8 | Query `"; touch <tmp>/pwned; #`, candidate text `$(touch <tmp>/pwned) bounded` | `Applied`; no `pwned` file; the metacharacter candidate outranks a disjoint one | Shell metacharacters in both fields | same |
| AC-9 | `git diff --name-only origin/main`, `git diff origin/main -- Cargo.toml`, `cargo machete` | No `Cargo.toml` change; no `src/` path outside the two named | — | `bash scripts/machete-check.sh`, recorded in the PR body |
| AC-10 | `bash integrations/reranker/run-model-tests.sh` on a machine with no torch and no snapshot | Non-zero exit; one line per missing prerequisite; the fix command for each | The absent-model case, which must never be green | run it on this machine |
| AC-11 | The pinned snapshot plus `tests/test_model_backend.py` | Architecture, `num_labels`, head-provenance and top-1 assertions | **Implemented, not measured here** | `bash integrations/reranker/run-model-tests.sh` |
| AC-12 | A PEFT `SEQ_CLS` adapter saved without training into a `tmp_path`, then a mutated copy | Load and score; then refusal with the fresh-head diagnostic | The exact PEFT failure mode the troubleshooting guide records | same |
| AC-13 | `benchmark --scoring lexical-overlap --candidates 32 --repeat 5` | A JSON report with `load_ms`, `p50_ms`, `p95_ms`, `peak_rss_bytes` | Model rows ship unmeasured, by the maintainer's scope decision | `cargo nextest run --all-features rerank_runner_2` |
| AC-14 | The whole tree | Every gate green | — | `bash scripts/check-all.sh`, `cargo nextest run --all-features`, `bash scripts/dup-check.sh`, `bash scripts/deny-check.sh`, `bash scripts/machete-check.sh` |

## Documentation impact

- **New:** this design, `docs/designs/2026-09-18-reranker-reference-backend.md`.
- **`docs/designs/2026-09-17-domain-first-migration-inventory.md`:** one cell,
  the `bridge` column of the `src/utilities/rerank_runner.rs` row, which now
  names both colocated suites. No row is added or removed.
- **New:** `integrations/reranker/README.md` — the operator document the issue's
  final acceptance criterion asks for: install, base versus adapter invocation,
  the warm lifecycle and its startup cost, timeout behavior, the fallback
  ladder, the pinned identities, and the performance table with its unmeasured
  rows and the commands that fill them.
- **`integrations/README.md`:** gains a `reranker/` section beside `agent/`,
  stating that it is a repository asset and is not embedded in the binary.
- **`docs/README.md`:** this design under `## Explanation`, beside the #211
  protocol entry, and `integrations/reranker/README.md` under
  `## How-to guides`, because operating the backend is task-oriented material
  and the design is not.
- **`AGENTS.md`:** the `utilities/` module-map row's "#212 supplies a backend"
  sentence is replaced by the shipped path; `## Testing` records `python3` as a
  prerequisite of the colocated reranker conformance suite.
- **`README.md` (root):** one short paragraph under the existing optional-tooling
  material, stating plainly that nothing on the search path calls it yet.
- **`src/utilities/README.md`: deliberately unchanged.** Its Contents table is
  one row per *production* file, and this change adds none — the only Rust it
  adds is a colocated test, which that README's closing sentence already covers
  generically ("Colocated unit tests live in `tests/` beside this file"). The
  `rerank_runner.rs` row still describes exactly what that file does.
- **Unchanged:** `docs/cli-reference.md`, `docs/scenarios/`,
  `docs/configuration.md`, `Cargo.toml`, `migrations/`, `src/store/`. No CLI
  subcommand, flag, environment variable, config key, HTTP route, dependency or
  schema object is added, so `scripts/cli-docs-check.sh`,
  `tests/cli_scenario_catalog.rs` and `scripts/migration-check.sh` have nothing
  to regenerate.

## Open Questions

1. **The protocol cannot express "which identity actually scored".** Owner:
   recorded here for #213/#214; **not acted on, by explicit non-goal 4.** The
   response echoes the request's `model` and `adapter`, so a backend can never
   truthfully say "you asked for `lora-v1`, I scored with the base model". That
   is why in-process fallback is refused rather than implemented. A protocol
   version 2 could add a `scored_with: {model, adapter}` pair beside the echoed
   identities and make a declared fallback honest. Not blocking: the
   `Declined` → deterministic-ranking path already gives comemory a correct,
   no-regression fallback today.
2. **Measured base/adapter compatibility is deferred.** Owner: this issue,
   resolved as scope. The maintainer's decision is to ship the loader, the
   validation and the suite, and to stop before the download. AC-11, AC-12 and
   the model half of AC-13 are therefore implemented-but-unmeasured, and every
   one of them says so in place. Not blocking; the conformance suite covers the
   protocol without them.
3. **`--padding longest` versus `max_length`.** Owner: this issue, resolved.
   `longest` is the default because padded positions are attention-masked, so
   batch composition cannot change a score beyond floating-point reassociation;
   `max_length` is documented as the bit-reproducibility lever for #214's
   qualification runs, at roughly an order of magnitude more compute. Not
   blocking.
4. **Serial warm serving.** Owner: this issue, resolved. One model instance
   behind one socket, requests served in arrival order. Concurrency would
   multiply resident memory without shortening the queue, and comemory issues
   one rerank per search. #213 may revisit with operational evidence. Not
   blocking.
5. **Transitive dependency pins.** Owner: this issue, resolved. Direct
   dependencies are pinned exactly; the transitive closure is left to pip
   rather than enumerated, because an unresolved enumeration would record
   guesses as pins. A `pip freeze` lockfile can be added by #214 from the first
   environment that is actually built and measured. Not blocking.
