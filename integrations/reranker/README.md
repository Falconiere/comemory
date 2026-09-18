# Optional reference reranker backend

A pinned Python cross-encoder that speaks comemory's
[reranker command protocol](../../docs/designs/2026-09-18-reranker-command-protocol.md),
scores one relevance value per query/passage pair, and loads a PEFT LoRA
adapter when one is configured.

> **Nothing in comemory calls this yet.** The `comemory` binary is a standalone
> Rust CLI with no Python and no model dependency, and no search surface
> invokes a reranker. Wiring one into retrieval is
> [#213](https://github.com/Falconiere/comemory/issues/213); training an
> adapter is [#214](https://github.com/Falconiere/comemory/issues/214). This
> directory is a repository asset you opt into, not part of the released
> binary, and it is deliberately **not** embedded the way
> `integrations/agent/` is.

The design and the reasoning behind every pinned choice are in
[docs/designs/2026-09-18-reranker-reference-backend.md](../../docs/designs/2026-09-18-reranker-reference-backend.md).

## What is pinned

| Thing | Value |
| --- | --- |
| Protocol version | `1` |
| Base model | `cross-encoder/ms-marco-MiniLM-L6-v2` |
| Revision | `233902d25c440f23af6f7d6e94d2946bac0bee0a` |
| Architecture | `BertForSequenceClassification`, `num_labels = 1`, ~22.7M parameters |
| Licence | Apache-2.0 |
| dtype / device | `float32` / `cpu` |
| Maximum length | `512` tokens |
| Truncation / padding | `longest_first` / `longest` |
| Batch size / score column | `16` / `0` |
| Score direction | `higher_is_better` |
| Network | off — `local_files_only`, unless `--allow-download` |
| Maximum request | 8 MiB on stdin, matching `RerankLimits::max_request_bytes` |

Every one of these is a constant in `comemory_rerank_pins.py`, appears in the
fingerprint the backend emits, and has a flag that overrides it. The dependency
versions are pinned in `requirements.txt` and mirrored in that same file, which
is what `run-model-tests.sh` enforces.

## Install

The model path needs Python 3.12 or newer, because `numpy 2.5.3` does.

```bash
python3 -m venv .venv-reranker
. .venv-reranker/bin/activate
pip install -r integrations/reranker/requirements.txt

# Downloading the weights is always an explicit step. The backend never fetches
# a model inside a request: a download in the middle of a 20-second rerank
# budget is a stall, not a feature.
huggingface-cli download cross-encoder/ms-marco-MiniLM-L6-v2 \
  --revision 233902d25c440f23af6f7d6e94d2946bac0bee0a
```

The deterministic `lexical-overlap` mode, the warm `client` and `--help` need
**none** of that. They are Python standard library alone and run on Python 3.9
and newer.

## Invocation

### Base model, one process per request

```bash
python3 integrations/reranker/comemory_rerank.py score
```

Reads one JSON request from stdin, writes one JSON response to stdout, exits 0.
Silent on stderr when it succeeds. The request must carry
`"model": "cross-encoder/ms-marco-MiniLM-L6-v2@233902d25c440f23af6f7d6e94d2946bac0bee0a"`
and `"adapter": null`.

### With a LoRA adapter

```bash
python3 integrations/reranker/comemory_rerank.py score \
  --adapter /path/to/lora-v1 --adapter-label lora-v1
```

The request must then carry `"adapter": "lora-v1"`. `--adapter-label` defaults
to the directory's own name.

### Print the complete fingerprint

```bash
python3 integrations/reranker/comemory_rerank.py fingerprint
python3 integrations/reranker/comemory_rerank.py fingerprint --scoring lexical-overlap
```

### Measure it

```bash
python3 integrations/reranker/comemory_rerank.py benchmark --candidates 32 --repeat 20
```

Reports `load_ms`, `p50_ms`, `p95_ms`, `max_ms` and `peak_rss_bytes` as JSON
over a request it generates itself.

## The identity gate

A process answers **only** to the identity it was configured with. Ask it for a
different model or a different adapter and it exits `65` with a diagnostic
naming both sides, rather than echoing the label you asked for over scores
something else produced.

That matters because the protocol requires the response to echo `model` and
`adapter` byte for byte. A backend that echoed whatever it was handed would let
a caller asking for `lora-v3` receive `lora-v1` scores under the `lora-v3` name,
and nothing on the wire could detect it. The gate makes that impossible.

`--model-label` and `--adapter-label` set the labels a process answers to when
your naming differs from the defaults.

## The deterministic mode

```bash
python3 integrations/reranker/comemory_rerank.py score --scoring lexical-overlap
```

**This is not a model and it is not a quality baseline.** It scores Jaccard
overlap of lowercased alphanumeric token sets, it answers to the model label
`lexical-overlap@1` — which no request expecting a real model can carry — and
its fingerprint reports `"scoring_is_neural": false`.

It exists so that the real subprocess, the real JSON on real pipes, the real
identity gate and the real Rust `RerankRunner` are exercised on every
`cargo nextest run`, with no download and no torch. That suite is
`src/utilities/tests/rerank_runner_2.rs`.

## Warm local inference

A cold cross-encoder start pays interpreter startup, a torch import and weight
loading every time. The protocol is one child process per request and that does
not change; the model moves behind a local socket instead.

```bash
# Start once. Loads and validates the model, then serves.
python3 integrations/reranker/comemory_rerank.py serve \
  --socket "$XDG_RUNTIME_DIR/comemory-rerank.sock" \
  --ready-file "$XDG_RUNTIME_DIR/comemory-rerank.ready"

# Point the caller at this instead of `score`.
python3 integrations/reranker/comemory_rerank.py client \
  --socket "$XDG_RUNTIME_DIR/comemory-rerank.sock"
```

`client` imports nothing heavier than `socket`, so it is still a real child
process speaking the real protocol — one JSON object in on stdin, one out on
stdout, exit 0.

**Lifecycle, which is separate from query latency.** The server logs three
kinds of line to stderr and nothing else:

```
comemory-rerank: fingerprint {"backend":"comemory-rerank",...}
comemory-rerank: ready socket=<path> load_ms=<int>
comemory-rerank: served request_id=<id> candidates=<int> score_ms=<int>
```

`load_ms` on the `ready` line is the **startup** cost, paid once. `score_ms` on
each `served` line is the **per-query** cost. Capture that stderr to a file and
the two numbers never get confused with each other.

Requests are served one at a time: the model is the bottleneck, so concurrency
would multiply resident memory without shortening the queue, and comemory
issues one rerank per search. `--idle-timeout SECONDS` stops the server after
that much inactivity so the memory comes back; the default, `0`, never stops.

The socket is created under a `0o077` umask, a stale socket file is reclaimed,
and a socket another live server is listening on is never displaced — that
exits `73`.

`client` takes only `--socket`, `--connect-timeout` and `--request-timeout`. It
carries no model, adapter or scoring flags, because every one of those is the
server's configuration and duplicating them would create a second place for the
identity to drift.

## Timeout behavior

| Budget | Default | Owner |
| --- | --- | --- |
| End-to-end child budget | 20 s (`DEFAULT_RERANK_TIMEOUT`) | the Rust `RerankRunner` |
| `client --connect-timeout` | 2 s | this backend |
| `client --request-timeout` | 15 s | this backend |
| `serve --idle-timeout` | 0 (never) | this backend |

The Rust runner's budget is authoritative and covers startup, stdin, stdout and
exit; when it expires the child is killed and reaped and the outcome is
`Declined(TimedOut)`. The client's own budget deliberately sits **below** it, so
a stalled server produces a diagnostic instead of the client being killed
mid-write with nothing to show for it.

A warm `serve` process is not the runner's child, so the runner's deadline never
touches it.

## Never a fresh relevance head

Transformers attaches a randomly initialized classification head to a checkpoint
that has none, and PEFT's troubleshooting guide records the matching adapter
failure: a LoRA checkpoint that omits the head from `modules_to_save` reloads a
*different* random head every time, giving confidently wrong scores with no
error anywhere.

Both are refused here, proven from the checkpoint's own tensor names:

- **Base.** Every parameter of the declared head modules must be present in the
  weight file. If one is not, the backend exits `65` saying that scoring would
  use randomly initialized relevance weights.
- **Adapter.** `adapter_config.json` must declare `peft_type: "LORA"`,
  `task_type: "SEQ_CLS"`, a `base_model_name_or_path` equal to the configured
  model id, a matching `revision` when it declares one, and a
  `modules_to_save` containing every head module — and the adapter weight file
  must actually carry a tensor for each of them. Anything else exits `65`.

`--head-module` names the head for an architecture whose head is not called
`classifier`. `--allow-base-mismatch` downgrades an adapter/base identity
disagreement to a warning, for a deliberately retargeted adapter.

## Falling back to base scoring

The backend **never** substitutes one identity for another inside one process.
Answering a request that declared `adapter: "lora-v1"` with base-model scores
while echoing `"lora-v1"` is something the protocol cannot express and no caller
could detect, so it is not offered.

The ladder is:

1. **The adapter fails validation or loading** → the backend exits non-zero,
   `RerankRunner` returns `Declined`, and `RerankOutcome::order_ids()` hands
   back the submitted order. comemory's own deterministic ranking is used
   unchanged. This is the guaranteed no-regression path and it needs no
   configuration.
2. **To actually serve base-model scores**, run a second invocation with no
   `--adapter` and have the caller send `adapter: null`. Choosing between the
   two at runtime is #213's configuration work.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | One valid response on stdout |
| `64` | Bad command line |
| `65` | Unparsable request, unsupported `protocol_version`, identity mismatch, or an adapter/base compatibility failure |
| `69` | A dependency is not importable, the pinned snapshot is absent, or the warm socket is unreachable |
| `70` | Scoring raised, or produced a non-finite score |
| `73` | The warm socket cannot be bound, or another server owns it |
| anything else | An unhandled internal error. The traceback is on stderr, and it is a bug in this backend — every refusal it knows about is one of the codes above |

Every non-zero exit writes one `comemory-rerank: <message>` line to stderr.
stderr is diagnostic only and is never parsed; it reaches a Rust caller as
`RerankApplied::stderr_excerpt` / `RerankDeclined::stderr_excerpt`.

## Tests

Two suites, deliberately separated.

**Protocol conformance — always runs, no model needed.**

```bash
cargo nextest run --all-features rerank_runner_2
```

Real `python3` children, real pipes, real `RerankRunner`. Requires `python3` on
`PATH` and nothing else. It does **not** skip when the interpreter is missing:
it fails, naming the prerequisite.

**Model integration — opt-in, and it refuses to pretend.**

```bash
pip install -r integrations/reranker/requirements.txt
huggingface-cli download cross-encoder/ms-marco-MiniLM-L6-v2 \
  --revision 233902d25c440f23af6f7d6e94d2946bac0bee0a
bash integrations/reranker/run-model-tests.sh
```

It checks every pinned library version and the pinned snapshot first. A missing
library, a *mismatched* library version, or an absent snapshot each print the
exact fix and exit `69` with the words "the model suite did NOT run". It never
exits 0 without having run, because a skipped model test that reads as a pass
turns "we have not checked this" into "we checked this and it was fine".

## Measured performance

**Not yet measured.** The scaffolding is complete and reviewed; the numbers are
not, because this change deliberately stops before the model download. Fill the
table in from a real run and record the hardware.

| Metric | Command | Value |
| --- | --- | --- |
| Cold load (`load_ms`) | `comemory_rerank.py benchmark --repeat 1` | not measured |
| Warm p50 / p95, 32 candidates | `comemory_rerank.py benchmark --candidates 32 --repeat 20` | not measured |
| Warm p50 / p95, 200 candidates | `comemory_rerank.py benchmark --candidates 200 --repeat 20` | not measured |
| Peak resident memory | same run, `peak_rss_bytes` | not measured |
| CPU functionality | `bash run-model-tests.sh` | implemented, not measured |
| CUDA (`--device cuda`) | `bash run-model-tests.sh` on a CUDA host | implemented, **unverified** |
| Apple MPS (`--device mps`) | `bash run-model-tests.sh` on Apple silicon | implemented, **unverified** |

`--device` resolves and refuses an unavailable accelerator rather than silently
falling back to CPU, but only the CPU path is claimed. Record hardware with each
row: a latency number without a machine beside it is not a measurement.

## Files

| File | Holds |
| --- | --- |
| `comemory_rerank.py` | The entry point: subcommands, exit codes, stdin and stdout |
| `comemory_rerank_pins.py` | Every pinned constant, the exit-code vocabulary, and `RerankError` |
| `comemory_rerank_protocol.py` | Request decoding, the identity gate, response encoding |
| `comemory_rerank_lexical.py` | The deterministic, non-neural scorer |
| `comemory_rerank_model.py` | The cross-encoder: load, tokenize, forward pass, fingerprint |
| `comemory_rerank_compat.py` | Whether a checkpoint or adapter may be scored with at all |
| `comemory_rerank_warm.py` | The framed Unix-socket server and its thin client |
| `comemory_rerank_preflight.py` | The opt-in suite's prerequisite check |
| `requirements.txt` | The pinned dependency set for the cross-encoder mode |
| `run-model-tests.sh` | Preflight, then the opt-in model suite |
| `tests/test_model_backend.py` | What only the pinned weights can prove |
