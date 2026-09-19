# Offline LoRA training and qualification

A pinned, reproducible recipe that fits a LoRA adapter to a
[reviewed relevance dataset](../../docs/designs/2026-09-18-reviewed-dataset-export.md), and a
harness that decides — honestly — whether the resulting adapter is worth enabling.

> **Nothing here runs automatically, and nothing here runs inside `comemory`.** The
> `comemory` binary is a standalone Rust CLI with no Python and no model dependency. No hook,
> no watcher and no save, sync or index event starts a training job. Training is a command an
> operator types, against an export an operator produced. This directory is a repository asset
> you opt into, and it is deliberately **not** embedded the way `integrations/agent/` is.

The design and the reasoning behind every pinned choice are in
[docs/designs/2026-09-18-lora-adapter-training.md](../../docs/designs/2026-09-18-lora-adapter-training.md).
The adapter it produces is loaded by [`integrations/reranker/`](../reranker/README.md) and
served through the optional stage documented in
[docs/guides/learned-reranking.md](../../docs/guides/learned-reranking.md).

## Collecting and serving cannot overlap

`[observations] enabled` and `[rerank] enabled` are mutually exclusive at run time, and that is
deliberate. The candidate observation contract fixes `pool_position` as the order retrieval
produced **before any arm reordered anything**, and this recipe trains on it. Recording a pool a
model has already reordered would feed the model's own output back into the set it is trained
from.

So the workflow alternates between two phases rather than running as one loop:

```text
   collect                                     serve
   [observations] enabled = true               [rerank] enabled = true
   [rerank]       enabled = false              [observations] enabled = false
        |                                              ^
        v                                              |
   comemory find / judge / export-dataset  ->  train  -+
```

## What is pinned

| Thing | Value |
| --- | --- |
| Recipe version | `1` |
| Base model / revision | `cross-encoder/ms-marco-MiniLM-L6-v2` @ `233902d25c440f23af6f7d6e94d2946bac0bee0a` |
| Tokenizer / revision | the same repository and revision |
| Architecture | `BertForSequenceClassification`, `num_labels = 1` |
| Seed | `20260918`, driving Python, NumPy, torch and the batch shuffle |
| LoRA rank / alpha / dropout | `16` / `32` / `0.05` |
| Target modules | `query`, `value` — the attention projections, PEFT's own BERT mapping |
| LoRA bias | `none` |
| `task_type` / `modules_to_save` | `SEQ_CLS` / `["classifier"]` |
| Objective | `BCEWithLogitsLoss` on `relevance / 3` |
| Epochs / learning rate / schedule | `3` / `2e-4` / linear with a `0.1` warmup ratio |
| Optimizer | AdamW, betas `(0.9, 0.999)`, eps `1e-8`, weight decay `0.01`, clip `1.0` |
| Batch size (train / eval) | `16` / `32` |
| Precision | `float32` throughout, no autocast |
| Max length / truncation / padding | `512` / `longest_first` / `longest` |
| Checkpoint selection | validation nDCG@10, ties broken by lower validation loss |
| Reload tolerance | `1e-4` absolute on the logit |
| Trainable fraction ceiling | `0.05` |

Every one of these is a constant in `comemory_train_pins.py` and appears in the manifest each
adapter is packaged with. The base identity, the tokenization policy and the score column are
**imported** from `integrations/reranker/comemory_rerank_pins.py` rather than restated, because
a drift between the trainer's idea of the base model and the scorer's is undetectable from
either side: the adapter would load and produce confidently wrong numbers.

The dependency set in `requirements.txt` is identical, version for version, to the reference
backend's, for the same reason — training and serving must run the same libraries or the
save/reload parity check compares two builds rather than one.

## The workflow, in full

Every command below is copy-pasteable. Only steps 4 and 5 need the pinned libraries and the
pinned weights.

### 1. Collect reviewed judgments

```bash
# Capture is off by default, because it stores passage snapshots.
export COMEMORY_OBSERVATIONS_ENABLED=1

comemory find "paged search non-determinism" --k 20
# -> observation_id: o-20260918-9f8e7d6c

comemory judge o-20260918-9f8e7d6c \
  --ref "memory:5a9f19bc:5a9f19bc40…=3" \
  --ref "code:comemory:src/domains/retrieval/rerank.rs:rerank:9d1c1f0b=2" \
  --ref "document:ef3d2b5a:guides/schema-migrations.md:df2e4e83…:0=0"
```

A grade of `0` is a reviewed hard negative and is worth recording: it is different from a
candidate nobody judged, and the dataset keeps the distinction.

### 2. Export the dataset

```bash
comemory export-dataset --out ./lora-datasets/v1
```

The holdout is **withheld by default**. That is what makes it a holdout: a `holdout.jsonl` on
disk during training is a file a mining or selection step could read. The manifest publishes its
digest either way, so the adapter can still record which held-out set it must be qualified
against.

### 3. Resolve the recipe, before installing anything

```bash
python3 integrations/training/comemory_train.py plan --dataset ./lora-datasets/v1
python3 integrations/training/comemory_train.py plan --dataset ./lora-datasets/v1 --json
```

`plan` is the standard library alone. It applies every gate the trainer applies — the contract
versions, the per-file digests, the split of each row, the group boundary between train and
validation — and prints what would be fitted, to what, under which pins. It never opens
`holdout.jsonl`.

### 4. Install the training environment

```bash
python3 -m venv .venv-training          # Python 3.12 or newer: numpy 2.5.3 requires it
. .venv-training/bin/activate
pip install -r integrations/training/requirements.txt

# Downloading the weights is always an explicit step.
huggingface-cli download cross-encoder/ms-marco-MiniLM-L6-v2 \
  --revision 233902d25c440f23af6f7d6e94d2946bac0bee0a
```

### 5. Train

```bash
python3 integrations/training/comemory_train.py train \
  --dataset ./lora-datasets/v1 \
  --out ./lora-adapters/lora-v1 \
  --adapter-label lora-v1
```

It writes exactly three files and refuses a non-empty `--out` without `--force`:

| File | Holds |
| --- | --- |
| `adapter_config.json` | the LoRA configuration, including `modules_to_save: ["classifier"]`, the base model id and the pinned revision |
| `adapter_model.safetensors` | the LoRA matrices and the trained relevance head |
| `comemory_adapter.json` | the immutable manifest: dataset provenance, the parameter audit, the frozen-base digest, per-epoch validation numbers, the reload check, hardware, library versions and the limitations |

On another machine, re-establish the guarantees before serving:

```bash
python3 integrations/training/comemory_train.py verify --adapter ./lora-adapters/lora-v1
```

### 6. Qualify it

Qualification is two `comemory benchmark` passes with a scoring step between them. The first
pass captures one candidate pool per task; every arm then reorders **that one snapshot**, which
is what makes the comparison paired and what makes injecting a positive retrieval never returned
impossible.

```bash
Q=./lora-qualification/v1
MODEL=cross-encoder/ms-marco-MiniLM-L6-v2@233902d25c440f23af6f7d6e94d2946bac0bee0a

# Re-export WITH the holdout, now that training is over.
comemory export-dataset --out ./lora-datasets/v1-qualify --include-holdout

# 6a. The reviewed set, generated from the withheld holdout.
python3 integrations/training/comemory_qualify.py build-set \
  --dataset ./lora-datasets/v1-qualify \
  --out "$Q/set.yaml" --provenance "$Q/set.provenance.json"

# 6b. Capture the pool once. This arm is comemory's own deterministic ranking.
comemory benchmark --set "$Q/set.yaml" --report "$Q/baseline.json"

# 6c. The unadapted cross-encoder.
python3 integrations/training/comemory_qualify.py score \
  --report "$Q/baseline.json" --arm base \
  --out "$Q/base.scores.json" --sidecar "$Q/base.scoring.json" \
  --model "$MODEL" \
  -- python3 integrations/reranker/comemory_rerank.py score --scoring cross-encoder

# 6d. The same cross-encoder with the adapter.
python3 integrations/training/comemory_qualify.py score \
  --report "$Q/baseline.json" --arm lora \
  --out "$Q/lora.scores.json" --sidecar "$Q/lora.scoring.json" \
  --model "$MODEL" --adapter lora-v1 \
  -- python3 integrations/reranker/comemory_rerank.py score --scoring cross-encoder \
       --adapter ./lora-adapters/lora-v1 --adapter-label lora-v1

# 6e. Score all three arms over the one captured pool.
comemory benchmark --set "$Q/set.yaml" \
  --scores "$Q/base.scores.json" --scores "$Q/lora.scores.json" \
  --report "$Q/qualification.json"

# 6f. Record the outcome.
python3 integrations/training/comemory_qualify.py decide \
  --report "$Q/qualification.json" --provenance "$Q/set.provenance.json" \
  --scoring "base=$Q/base.scoring.json" --scoring "lora=$Q/lora.scoring.json" \
  --adapter-manifest ./lora-adapters/lora-v1/comemory_adapter.json \
  --out "$Q/decision.json" --markdown "$Q/decision.md"
```

### 7. Activate it, only if the decision says so

```toml
# config.toml — explicit, and off until you write this.
[rerank]
enabled = true
command = [
  "python3", "/abs/path/to/comemory/integrations/reranker/comemory_rerank.py",
  "score", "--scoring", "cross-encoder",
  "--adapter", "/abs/path/to/lora-adapters/lora-v1", "--adapter-label", "lora-v1",
]
model = "cross-encoder/ms-marco-MiniLM-L6-v2@233902d25c440f23af6f7d6e94d2946bac0bee0a"
adapter = "lora-v1"
prefix = 50
timeout_ms = 20000
```

Then confirm it is actually running, on each surface that can report it:

```bash
comemory find "activation decay" --json | jq '.learned | {applied, model, adapter, elapsed_ms}'
comemory search "activation decay" --json | jq '.learned'
```

**Rollback is a configuration change, not a runtime fallback.** Wire protocol v1 requires a
response to echo the `adapter` it was asked for byte for byte, so answering a `lora-v1` request
with base-model scores while echoing `lora-v1` is something the format cannot express. The two
levers are:

1. **Back to the base model** — drop `adapter` from `[rerank]` and `--adapter` from `command`.
2. **Back to deterministic ranking** — `enabled = false`. The next command launches nothing.

And any failure already falls back on its own: a scorer that cannot start, exits non-zero, times
out or answers with anything invalid leaves comemory's entire deterministic order in place and
reports itself in `learned.fallback`.

## The decision

`decide` records one of exactly three words.

| Outcome | When |
| --- | --- |
| `go` | The paired nDCG interval clears the declared `min_ndcg_gain`, every operational budget is met, and every integrity check passed |
| `no-go` | Every check ran and the evidence does not support enabling: a regression, a neutral result, or an improvement that breached a budget |
| `insufficient-evidence` | The benchmark verdict is inconclusive, an arm is missing, or an integrity check failed or could not run |

**A negative result is a valid result.** Nothing in the harness prefers `go`, and an adapter is
not enabled because training completed.

**A check that could not run is never recorded as a pass.** Every check appears in the record as
`pass`, `fail` or `skipped` together with the values it compared, and a `skipped` check caps the
outcome at `insufficient-evidence`. Omitting `--adapter-manifest`, for instance, means the
adapter's own record of which holdout it withheld could not be compared with the one being
scored — so the provenance chain is unverified and the answer says so.

Two operational budgets belong to this harness rather than to the benchmark set, because they
are properties of the scorer child that `comemory` cannot see:

| Budget | Value | Why |
| --- | --- | --- |
| `MAX_FALLBACK_RATE` | `0.0` | Production tolerates a fallback by design — comemory's own ranking stands. A qualification run that could not score every held-out task has not measured the arm it is about to claim a number for |
| `MAX_SCORING_P95_MS` | `20000` | `DEFAULT_RERANK_TIMEOUT`. A scorer whose p95 exceeds the production deadline would fall back in production, so its measured quality would not be the quality served |

The deterministic `lexical-overlap` scorer can never earn a `go`. It measures no relevance
quality — the reference backend says so in its own fingerprint — and the decision refuses it by
model label.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | The command did what it was asked |
| `64` | Bad command line |
| `65` | The dataset, the manifest, an artifact or an adapter package is not what the contract says |
| `69` | A pinned library or the pinned snapshot is absent, or the holdout the qualification needs was not exported |
| `70` | An audit failed: nothing trains, the base moved, or a reloaded adapter disagrees beyond the tolerance |

Every non-zero exit writes one `comemory-train: <message>` line to stderr.

## Tests

Two suites, deliberately separated.

**Offline — always runs, no model needed.**

```bash
cargo nextest run --all-features -E 'binary(lora_qualification) + binary(lora_qualification_2)'
python3 -m unittest discover --start-directory integrations/training/tests \
  --pattern 'test_offline_*.py' --verbose
```

The Rust suites drive the real binary over a real SQLite store: real memories, a real git
repository, a real indexed document tree, real `comemory judge` verdicts, a real
`comemory export-dataset`, a real `comemory benchmark`, and real `python3` children speaking the
real wire protocol. They require `python3` on `PATH` and nothing else, and they **fail** rather
than skip when it is missing.

The Python suite proves the contracts the standard library can reach: that the recipe's pins are
the backend's pins, that the two requirements files agree line for line, that the held-out file
is outside the trainer's readable allowlist, and — the important one — that a synthetic adapter
package with exactly the tensor names this recipe saves is **accepted** by the reference
backend's own `validate_adapter`, while every way of breaking it is refused with exit `65`.

**Model integration — opt-in, and it refuses to pretend.**

```bash
pip install -r integrations/training/requirements.txt
huggingface-cli download cross-encoder/ms-marco-MiniLM-L6-v2 \
  --revision 233902d25c440f23af6f7d6e94d2946bac0bee0a
bash integrations/training/run-training-tests.sh
```

It checks every pinned library version and the pinned snapshot first. A missing library, a
*mismatched* version, or an absent snapshot each print the exact fix and exit `69` with the words
"the training suite did NOT run". It never exits 0 without having run.

## Measured results

**Not measured.** The recipe, the harness and both suites are complete and reviewed; the numbers
are not, because this change deliberately stops before the model download and before any training
job. Fill each row from a real run and record the hardware beside it: a latency number without a
machine next to it is not a measurement.

| What | Command | Value |
| --- | --- | --- |
| Which parameters train, and the frozen base | `bash run-training-tests.sh` | implemented, **not measured** |
| Save/reload agreement on the parity probe | `bash run-training-tests.sh` | implemented, **not measured** |
| The reference backend scoring through a real adapter | `bash run-training-tests.sh` | implemented, **not measured** |
| Training wall clock and peak RSS | `comemory_train.py train` → `comemory_adapter.json` `hardware` | not measured |
| Per-epoch validation nDCG@10 | the same file's `selection.per_epoch` | not measured |
| Three-arm quality, per domain, with the paired interval | steps 6b–6e above → `qualification.json` | not measured |
| Candidate pool recall | the same artifact, `arms[].overall.pool_recall` | not measured |
| Scoring latency p50 / p95 and peak child RSS | steps 6c–6d → `*.scoring.json` | not measured |
| Fallback rate | the same files, `fallback_rate` | not measured |
| The recorded outcome | step 6f → `decision.json` | **`insufficient-evidence`** — no adapter has been trained, so there is no LoRA arm |
| End-to-end through `[rerank]` on all four surfaces | step 7 | documented, **not measured** |

## Files

| File | Holds |
| --- | --- |
| `comemory_train_pins.py` | Every pinned constant, the re-export of the backend's base identity, and `RecipeError` |
| `comemory_train_data.py` | Reading an export: version gates, digest verification, the readable-file allowlist, the group boundary |
| `comemory_train_model.py` | Building the LoRA model, the parameter audit, the frozen-base digest, save, reload and the parity probe |
| `comemory_train_loop.py` | The seeded loop, the validation pass, and validation-only checkpoint selection |
| `comemory_train_manifest.py` | The resolved recipe, the immutable adapter manifest and its `adapter_id` |
| `comemory_train.py` | The entry point: `plan`, `train`, `verify` |
| `comemory_qualify_set.py` | The withheld holdout split as a reviewed benchmark set |
| `comemory_qualify_yaml.py` | The standard-library YAML emitter that set is written with |
| `comemory_qualify_score.py` | One captured artifact plus one scorer as one arm's scores and its operational cost |
| `comemory_qualify_wire.py` | Response validation, and every way a scorer's answer is refused |
| `comemory_qualify_checks.py` | The integrity checks and the operational budgets |
| `comemory_qualify_decide.py` | The three-word outcome and the record behind it |
| `comemory_qualify_render.py` | That record as markdown |
| `comemory_qualify.py` | The entry point: `build-set`, `score`, `decide` |
| `requirements.txt` | The pinned dependency set for the training mode |
| `run-training-tests.sh` | Preflight, then the opt-in model suite |
| `tests/test_offline_recipe.py` | What the standard library alone can prove |
| `tests/test_model_recipe.py` | What only the pinned weights can prove |
