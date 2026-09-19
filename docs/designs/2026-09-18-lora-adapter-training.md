# Reproducible LoRA adapter training and qualification — Design

**Date:** 2026-09-18   **Status:** Approved   **Author:** Falconiere R. Barbosa
**Topic:** An external, offline LoRA training recipe over #210 dataset exports, and a
qualification harness that compares deterministic ranking, unadapted reranking and LoRA
reranking on identical held-out candidates through #208's benchmark and #212's backend.

Issue: [#214](https://github.com/Falconiere/comemory/issues/214).
Roadmap: [#207](https://github.com/Falconiere/comemory/issues/207).
Dependencies, all merged: #208 (benchmark and candidate observation contract), #210
(reviewed dataset export), #212 (reference inference backend), #213 (retrieval
integration).

## Problem

Adding adapter support is not evidence of better retrieval. #212 can load a PEFT adapter
and #213 can serve one, but nothing in this repository produces an adapter, and nothing
decides whether an adapter that exists is worth enabling. Two gaps follow from that.

**There is no recipe.** A reranker adapter is only reproducible if the base model
revision, the tokenizer revision, the dependency versions, the seed, the LoRA rank, alpha,
dropout and target modules, the learning schedule, the numeric precision and the
scoring-head behavior are all pinned somewhere a reader can find in one place. #212 proved
how much that matters from the other side: it refuses a checkpoint whose relevance head is
freshly initialized, because PEFT reloads a *different* random head on every load when the
head is omitted from `modules_to_save`. A recipe that does not save what that refusal
expects to find produces an adapter the backend will not score with.

**There is no verdict.** #208 defined pool recall, recall@k, MRR, nDCG@k, a paired
bootstrap and a budget-driven verdict, and `comemory benchmark` already scores any number
of arms over one candidate snapshot. What is missing is the step that turns a trained
adapter into two of those arms and records, honestly, whether the evidence supports
enabling it — including the outcome where it does not, and the outcome where there is not
enough evidence to say.

This issue closes the #207 roadmap, so the deliverable is the recipe, the harness and a
recorded outcome — not an enabled adapter.

## Non-Goals

1. **No training run and no model weights.** This change ships the recipe and the harness.
   It does not run a training job, does not download a checkpoint, and adds no measured
   quality number. Everything that genuinely needs real weights or a GPU is an explicitly
   opt-in suite that refuses to exit 0 without having run. The expected recorded outcome
   of the qualification is therefore `insufficient-evidence`, and saying so plainly is the
   deliverable.
2. **No pairwise or listwise losses.** The objective is the single-score classification
   objective the pinned cross-encoder was trained with. Alternatives are later experiments
   justified by measurements, exactly as the issue scopes them.
3. **No quantization and no embedding adaptation.** Same reason.
4. **No Python, no model dependency and no new crate dependency in the Rust binary.**
   Nothing here enters `Cargo.toml` or the released binary. The one Rust artifact is a
   crate-root integration test that drives the real binary and the real scripts.
5. **No change to any merged contract.** Not #208's observation contract or its benchmark
   set schema, not #209's tables, not #210's record or manifest format, not #211's wire
   protocol, and not #213's integration. The harness is a consumer of all five.
6. **No automatic training.** Nothing runs on save, sync, watch or any hook. The workflow
   is a sequence of commands an operator types.
7. **No cursor-stable pagination.** #206 stays out of scope.
8. **No new CLI subcommand, flag, config key, HTTP route, table or migration.** The
   `comemory` binary is unchanged by this work.

## Architecture

### The shape

```text
    ── collection ──────────────────────────────────────────────────────────
    [observations] enabled = true,  [rerank] enabled = false
    comemory find …                       -> candidate_query_observations
    comemory judge <observation-id> …     -> candidate_judgments
    comemory export-dataset --out DATA    -> train/validation(/holdout).jsonl + manifest.json

    ── training (external, offline, opt-in) ────────────────────────────────
    comemory_train.py plan  --dataset DATA          (standard library only)
    comemory_train.py train --dataset DATA --out ADAPTER
                                            -> adapter_config.json
                                               adapter_model.safetensors
                                               comemory_adapter.json

    ── qualification ───────────────────────────────────────────────────────
    comemory_qualify.py build-set --dataset DATA --out set.yaml
    comemory benchmark --set set.yaml --report baseline.json
    comemory_qualify.py score --report baseline.json --arm base -- <backend …>
    comemory_qualify.py score --report baseline.json --arm lora -- <backend … --adapter>
    comemory benchmark --set set.yaml --scores base.json --scores lora.json \
                       --report qualification.json
    comemory_qualify.py decide --report qualification.json …
                                            -> decision.json + decision.md
                                               go | no-go | insufficient-evidence
```

Collection and serving cannot overlap. #213 made `[observations]` and `[rerank]` mutually
exclusive at run time because `pool_position` is defined as the order retrieval produced
*before any arm reordered it*, so recording a reranked pool would feed the model's own
output back into the set it is trained from. The workflow above is therefore two phases an
operator alternates between, never one loop, and the documented procedure says so.

### Decisive trade-off: reuse `comemory benchmark`, do not restate it

The obvious implementation of "compare three arms" is a Python scorer that computes nDCG
itself. It is rejected.

`comemory benchmark` already captures one candidate pool per task and scores every arm
over that one snapshot (`benchmark_arm.rs`: *"An arm never re-runs retrieval. It reorders
one already-captured candidate snapshot, which is what makes the comparison paired"*). It
already computes pool recall apart from recall@k, MRR and nDCG@k, already breaks every
figure down per domain, already bootstraps the paired per-task delta with a seeded stream,
and already reads a verdict off the interval rather than the point estimate. A second
implementation in Python would be a second definition of the repository's own quality
metrics, free to drift from the one CI tests.

So the harness supplies **arms**, not metrics. It generates the reviewed set from the
held-out split, turns an already-captured artifact into a scores file by calling #212's
backend over #211's wire protocol, and reads the artifact `comemory benchmark` writes.
Three consequences follow, and all three are wanted:

- **Identical held-out candidates are structural, not asserted.** Every arm reorders the
  same pool because the binary captured it once.
- **A positive retrieval did not return can never be injected.** `benchmark_metrics`
  counts an unmatched judgment in the pool-recall denominator and never adds it to the
  pool. The harness has no code path that could insert one.
- **Per-domain quality, candidate recall and paired uncertainty come for free**, in the
  exact definitions #208 documented and tested.

What the harness must measure itself is what the binary cannot see: **scoring latency,
peak scorer memory and fallback rate**, which belong to the child process, not to
retrieval. `ArmReport::latency_ms` is retrieval's own wall clock and is documented as
excluding the arm's inference; conflating the two would report a 7 ms retrieval as the cost
of a cross-encoder. The scoring step therefore writes an operational sidecar beside each
scores file, and the decision reads both.

### Decisive trade-off: an explicit training loop, not `Trainer`

`transformers.Trainer` brings `accelerate`, its own argument surface, its own checkpoint
selection and its own RNG plumbing. The recipe needs eleven pinned numbers, a seeded
shuffle, a linear warmup schedule, gradient clipping and validation-only selection — about
a hundred lines of explicit torch. Writing them explicitly keeps the dependency set to the
seven packages #212 already pins, keeps every knob visible in `comemory_train_pins.py`,
and removes a layer between the pinned value and the optimizer step. `Trainer` is the
right tool when the schedule is standard and the audit is not the point; here the audit is
the point.

### Decisive trade-off: one source of truth for the base identity

`comemory_train_pins.py` imports `comemory_rerank_pins` from `integrations/reranker/` by
path and re-exports `MODEL_ID`, `MODEL_REVISION`, `TOKENIZER_ID`, `TOKENIZER_REVISION`,
`HEAD_MODULES`, `MAX_LENGTH`, `TRUNCATION`, `PADDING`, `SCORE_COLUMN` and the exit-code
vocabulary. Restating them would create a second place for the identity to drift, and a
drift between the trainer and the scorer is undetectable from either side: the adapter
would load and produce confidently wrong numbers. The offline test suite asserts the
agreement, so the import cannot silently become a copy.

### Where the code lives

`integrations/training/`, a sibling of `integrations/reranker/`, following exactly the
precedent #212 set: external, offline, opt-in, not embedded in the binary. Nothing in
`src/`, `Cargo.toml` or `migrations/` changes.

| File | Holds |
| --- | --- |
| `comemory_train_pins.py` | Every pinned constant of the recipe, and the re-export of #212's base identity |
| `comemory_train_data.py` | Reading a #210 export: version gates, digest verification, the split boundary, example construction |
| `comemory_train_model.py` | Building the LoRA model, the trainable-parameter audit, the frozen-base proof, save, reload and the parity check |
| `comemory_train_loop.py` | The seeded training loop, the validation pass and checkpoint selection |
| `comemory_train_manifest.py` | The resolved recipe, the immutable adapter manifest and its `adapter_id` |
| `comemory_train.py` | Entry point: `plan`, `train`, `verify` |
| `comemory_qualify_set.py` | Held-out split -> #208 benchmark set plus its provenance sidecar |
| `comemory_qualify_yaml.py` | The standard-library block-YAML emitter that set is written with |
| `comemory_qualify_score.py` | A benchmark artifact -> one arm's scores file plus its operational sidecar |
| `comemory_qualify_checks.py` | The integrity checks and the operational budgets |
| `comemory_qualify_decide.py` | The artifact plus the sidecars -> the recorded decision |
| `comemory_qualify_render.py` | That decision as markdown |
| `comemory_qualify.py` | Entry point: `build-set`, `score`, `decide` |
| `requirements.txt` | The pinned dependency set for the training mode |
| `run-training-tests.sh` | Preflight, then the opt-in model suite |
| `tests/test_offline_recipe.py` | What the standard library alone can prove; runs on every `cargo nextest` |
| `tests/test_model_recipe.py` | What only real weights can prove; opt-in |
| `README.md` | The copy-pasteable workflow, the pins, and the not-measured table |

Every one of those files is sized against this repository's 300-code-line ceiling, which
counts Python docstrings: `scripts/guardrails/checks/file-size.sh` skips blank lines, `//`
lines and `#`-prefixed lines, and a docstring is none of the three. #212 measured a
documented cross-encoder module at 448 lines and had it refused outright. Roughly 40 percent
of each file below is prose, so each one is planned at or under about 180 statement lines.

`plan`, `build-set`, `score`, `decide` and the whole offline suite are **standard library
only**. Only `train` and `verify` import torch. That split is what lets the normal
`cargo nextest run` exercise the dataset gates, the split refusal, the set generator, the
wire protocol, the arms and the decision on real data with no download.

## Interfaces / Schema

### `comemory_train.py`

```text
comemory_train.py plan   --dataset DIR [--json]
comemory_train.py train  --dataset DIR --out DIR [--adapter-label NAME] [--force]
                         [--device cpu|cuda|mps|auto] [--allow-nondeterministic-algorithms]
                         [--allow-download] [--epochs N] [--seed N]
comemory_train.py verify --adapter DIR [--dataset DIR] [--candidates N]
```

`plan` resolves the recipe against a dataset and prints it, reading no weights and
importing nothing outside the standard library. It is the dry run, and it is what the
offline suite drives.

`train` runs the recipe. `--epochs` and `--seed` exist so a deviation from the pins is an
explicit, recorded act rather than an edit; every override is written into the adapter
manifest, and a manifest whose values differ from `comemory_train_pins` is visibly not the
pinned recipe.

`verify` reloads a saved adapter, re-scores the fixed parity probe and re-checks the
tolerance, so the save/reload guarantee can be re-established on the machine that will
serve the adapter rather than only on the machine that trained it.

### The pinned recipe

Every value below is a constant in `comemory_train_pins.py`, appears in the adapter
manifest, and has a flag or an explicit override only where the table says so.

| Thing | Value | Why |
| --- | --- | --- |
| Recipe version | `1` | Bumped whenever any row below changes |
| Base model / revision | `cross-encoder/ms-marco-MiniLM-L6-v2` @ `233902d25c440f23af6f7d6e94d2946bac0bee0a` | #212's pin, imported not restated |
| Tokenizer / revision | the same repository and revision | A tokenizer from another revision is a different preprocessing |
| Architecture | `BertForSequenceClassification`, `num_labels = 1` | The single relevance logit the protocol scores by |
| Head modules | `("classifier",)` | What #212 requires in `modules_to_save` |
| Seed | `20260918` | One seed for Python, NumPy, torch and the shuffle |
| LoRA rank `r` | `16` | |
| LoRA `alpha` | `32` | `alpha / r = 2`, the common ratio for a 22.7M-parameter encoder |
| LoRA dropout | `0.05` | |
| Target modules | `("query", "value")` | PEFT's own BERT mapping; attention projections only |
| LoRA bias | `"none"` | A bias term outside the adapter would change the frozen base |
| `task_type` | `"SEQ_CLS"` | What #212's compatibility check requires |
| `modules_to_save` | `["classifier"]` | The trained relevance head travels with the adapter |
| Objective | `BCEWithLogitsLoss` on `relevance / 3` | The single-score objective this model family was trained with |
| Epochs | `3` | |
| Learning rate | `2e-4` | LoRA's usual range; the base is frozen |
| Schedule | linear decay, warmup ratio `0.1` | |
| Optimizer | AdamW, betas `(0.9, 0.999)`, eps `1e-8`, weight decay `0.01` | |
| Gradient clipping | `1.0` max norm | |
| Train / eval batch | `16` / `32` | Train batch matches the backend's inference batch |
| Precision | `float32` throughout, no autocast | The serving path is float32; a bf16-trained head scored in fp32 is a second, unmeasured difference |
| Max length / truncation / padding | `512` / `longest_first` / `longest` | Imported from #212, so train and serve tokenize identically |
| Selection metric | validation nDCG@10, ties broken by lower validation loss | The deployment metric is ranking quality, not loss |
| Reload tolerance | `1e-4` absolute on the logit | float32 carries ~1e-7 relative precision; a reduction-order difference across a reload accumulates a few multiples of that at logit magnitudes under 20. Two orders above that floor, and far below any gap that could reorder a pool. The observed delta is recorded, so the pin can be tightened from evidence |
| Trainable fraction ceiling | `0.05` | A recipe that trains more than 5% of the parameters targeted something it did not mean to |
| Parity probe | 32 fixed `(query, passage)` pairs generated from a pinned template | Same shape as #212's benchmark request; deterministic, needs no dataset |

The dependency set is `integrations/training/requirements.txt`, pinned to exactly the
versions `integrations/reranker/requirements.txt` already pins — torch 2.14.0,
transformers 5.17.0, peft 0.21.0, tokenizers 0.23.2, safetensors 0.8.0,
huggingface-hub 1.32.0, numpy 2.5.3 — because training and serving must run the same
libraries or the parity check compares two builds. `run-training-tests.sh` refuses a
mismatched version exactly as #212's does.

### What the trainer may read

```python
READABLE_FILES = ("manifest.json", "train.jsonl", "validation.jsonl")
```

That tuple is the whole allowlist, and it is the structural expression of "final test data
stays out of training, mining and selection". `holdout.jsonl`, `holdout.implicit.jsonl`
and the two other `.implicit.jsonl` files are never opened. The manifest's `withheld`
entry for the holdout is *recorded* in the adapter manifest — its digest names which
holdout this adapter must be qualified against — and never read.

Before it builds a single example, `comemory_train_data` refuses:

| Situation | Exit | Message names |
| --- | --- | --- |
| `manifest.json` absent, or `manifest_version` / `record_version` / `observation_version` unknown | 65 | the file, the version found and the version supported |
| A file's SHA-256 differs from its `files[]` entry | 65 | the file, both digests |
| A row's `record_version` is not 1 | 65 | the file, the line number |
| A row's `split` is not the file's own split | 65 | the file, the line, both splits |
| Any `group_id` appears in both `train.jsonl` and `validation.jsonl` | 65 | the group and both files |
| A row's `label` is null (an unjudged row, from `--include-unjudged`) | dropped, counted | `dropped_unjudged` |
| A row's `label.provenance` is not `manual` | dropped, counted | `dropped_provenance` |
| Zero train rows, or zero validation rows | 65 | which split is empty, and that selection needs validation |
| Zero observations carrying a positive label in train | 65 | that a set with no positive carries no signal |

The group-overlap check is deliberately redundant with #210's splitter. It costs one pass
and it proves the property on the data actually in hand rather than trusting that the
producer held it.

### The adapter package

`train --out DIR` writes exactly three owned names, and refuses a non-empty directory
without `--force`:

| Name | Written by | Holds |
| --- | --- | --- |
| `adapter_config.json` | PEFT `save_pretrained` | `peft_type: "LORA"`, `task_type: "SEQ_CLS"`, `base_model_name_or_path`, `revision`, `r`, `lora_alpha`, `lora_dropout`, `target_modules`, `modules_to_save: ["classifier"]` |
| `adapter_model.safetensors` | PEFT `save_pretrained` | The LoRA matrices and the trained `classifier` tensors under their `…classifier.modules_to_save.default.…` names |
| `comemory_adapter.json` | this recipe | The immutable manifest below |

`base_model_name_or_path` and `revision` are set on the `LoraConfig` before
`get_peft_model`, so #212's `_check_adapter_base` has both sides to compare and the
revision check is armed rather than vacuously skipped.

```jsonc
{
  "manifest_version": 1,
  "recipe_version": 1,
  "adapter_id": "a-3c1f5a90b7e24d68",     // sha256 of this object with adapter_id removed
  "adapter_label": "lora-v1",             // the label #211 requests and #212 answers to
  "dataset": {
    "dataset_id": "d-…", "snapshot_digest": "…", "manifest_sha256": "…",
    "files_read": [{ "path": "train.jsonl", "sha256": "…", "rows": 210 }, …],
    "holdout": { "path": "holdout.jsonl", "sha256": "…", "read": false },
    "split": { "policy": "grouped-hash", "seed": "…", "ratios": {…} },
    "rows_by_split": {…}, "rows_by_domain": {…}, "rows_by_relevance": {…},
    "dropped": { "unjudged": 0, "provenance": 0 },
    "retrieval_revisions": [{ "knobs_hash": "…", "corpus_digest": "…" }]
  },
  "base": { "model_id": "…", "model_revision": "…", "tokenizer_id": "…",
            "tokenizer_revision": "…", "architectures": ["BertForSequenceClassification"],
            "num_labels": 1, "head_modules": ["classifier"],
            "frozen_parameters_sha256": "…", "frozen_parameters": 22713217 },
  "lora": { "r": 16, "alpha": 32, "dropout": 0.05,
            "target_modules": ["query", "value"], "bias": "none",
            "task_type": "SEQ_CLS", "modules_to_save": ["classifier"] },
  "schedule": {…}, "precision": {…}, "objective": {…},
  "seed": 20260918, "deterministic_algorithms": true,
  "trainable": { "parameters": 1181697, "total": 23894914, "fraction": 0.0494,
                 "names": ["base_model.model.bert.encoder.layer.0.attention.self.query.lora_A.default.weight", …] },
  "selection": { "metric": "ndcg_at_10", "k": 10, "selected_epoch": 2, "holdout_used": false,
                 "per_epoch": [{ "epoch": 1, "train_loss": …, "validation_loss": …,
                                 "validation_ndcg_at_10": …, "validation_observations": … }, …] },
  "reload_check": { "probe_pairs": 32, "max_abs_delta": 3.2e-06,
                    "tolerance": 0.0001, "passed": true },
  "hardware": { "platform": "…", "machine": "…", "python": "…", "device": "cpu",
                "cpu_count": 12, "torch_threads": 8,
                "peak_rss_bytes": …, "train_seconds": … },
  "libraries": { "torch": "2.14.0", … },
  "limitations": ["…", "…"],
  "files": [{ "path": "adapter_config.json", "sha256": "…" }, …]
}
```

`adapter_id` is `a-` plus the first sixteen hex characters of SHA-256 over the canonical
JSON of the object with `adapter_id` removed — the same construction #210 uses for
`dataset_id`. "Immutable" means the manifest digests its own inputs and outputs: change
any of them and the id no longer matches, which `verify` checks.

`limitations` is a written list, not a placeholder: the corpus the adapter was trained on,
the domains represented, the fact that a 512-token bound truncates longer passages, the
fact that the adapter is valid only for the pinned base revision, and the fact that
quality is whatever the qualification recorded — including "not measured".

### The frozen-base proof

Before the first optimizer step and again after the last, the trainer digests every
parameter that is neither a LoRA matrix nor a `modules_to_save` tensor — in sorted name
order, hashing each tensor's little-endian float32 bytes into one running SHA-256 — and
requires the two digests to be equal. A `requires_grad` assertion alone proves only what
was *intended*; the digest proves what happened. The digest, and the count of parameters
it covered, go into the manifest.

The audit that accompanies it records, and refuses on:

| Check | Refusal |
| --- | --- |
| At least one LoRA parameter is trainable | exit 70: the adapter would train nothing |
| Every `classifier` parameter is trainable | exit 70: naming the head parameter that is frozen |
| No other parameter is trainable | exit 70: naming the first offender |
| `trainable / total <= 0.05` | exit 70: naming the fraction |
| The base digest after training equals the digest before | exit 70: the base was modified |

### `comemory_qualify.py build-set`

```text
comemory_qualify.py build-set --dataset DIR --out set.yaml --provenance set.provenance.json
                              [--name NAME] [--k N] [--min-tasks N]
                              [--min-ndcg-gain F] [--max-ndcg-regression F]
                              [--max-p95-task-ms N] [--no-pin-content-version]
```

Reads `holdout.jsonl` and `manifest.json` and emits a #208 benchmark set. It is the only
component that reads the holdout, and it reads it to *evaluate*, never to train.

- One task per holdout observation that carries at least one `manual` label, id
  `<observation_id>`, query verbatim.
- `domain` from the observation's `filters.domains`: the three together are `all`, a single
  one is that one.
- `filters` mapped field by field onto `TaskFilters` — `repo`, `kind`, `lang`, `path`,
  `since`, `until`, `as_of` — and only the ones the task's domain scope permits, because
  #208 refuses a filter narrowing a leg the task excludes.
- `judgments` from the labels: `{relevance, target}` with the target built from `identity`
  — memory `{id, content_hash}`, code `{repo, path, symbol, blob_oid}`, document
  `{path, revision_hash, chunk_ordinal}`. The version pin is on by default, so a judgment
  describing a content version the qualification run no longer sees is counted stale rather
  than silently matched against changed content.
- `ranking` is pinned with `decay: 0.0`, which is what actually freezes ACT-R activation,
  and the rest of the knobs come from the manifest's `retrieval_revisions[0].knobs` when
  the export carries exactly one revision. Two or more distinct `knobs_hash` values is a
  refusal (exit 65) naming them: one set cannot pin two configurations, and silently
  picking one would qualify against a configuration half the data never saw.
- `budgets` from the flags, defaulting to #208's documented example: `min_tasks: 8`,
  `min_ndcg_gain: 0.02`, `max_ndcg_regression: 0.01`, `max_p95_task_ms: 1500`.

Observations it cannot express are dropped and counted, never approximated:

| Dropped | Counter |
| --- | --- |
| `filters.vector.kind == "supplied"` — a BYO-vector run whose vector the export does not carry | `dropped_vector_scenario` |
| Two labels on one task resolving to the same target with different grades | `dropped_contradictory_target` |
| No `manual` label | `dropped_unjudged` |

An export whose holdout carries **no** judged observation is refused with exit 65, naming the
count and the `--split` ratios and `--include-holdout` flag that produce one. `BenchmarkSet`
would refuse the empty `tasks` list too, but its message is about the YAML file rather than
about the export the operator actually has to fix.

Observations whose captured pool was truncated (`pool_truncated: true`) are admitted — the
qualification re-runs retrieval, so its pool is not the truncated one — and counted in
`truncated_source_pools`, because their reviewed labels may be less complete than an untruncated
observation's.

The sidecar `set.provenance.json` carries `dataset_id`, `snapshot_digest`, the holdout
file's SHA-256, the drop counters, `truncated_source_pools`, the resolved budgets, the
`knobs_hash` the set was pinned to and the corpus digest the export was captured against. `decide` reads it; #208's
set file gains no field, because `BenchmarkSet` denies unknown ones.

### `comemory_qualify.py score`

```text
comemory_qualify.py score --report baseline.json --arm NAME --out scores.json
                          --sidecar scoring.json --model LABEL [--adapter LABEL]
                          [--timeout-ms N] -- <program> [args…]
```

`--timeout-ms` defaults to `20000`, which is `utilities::rerank_runner::DEFAULT_RERANK_TIMEOUT`
— the budget production would hold the same child under.

There is no candidate-text bound here. The artifact's text was already bounded by the set's
`defaults.max_text_bytes` when the benchmark captured it, and a second ceiling in the scoring
step would be a second place for that number to drift. The text is submitted as the artifact
carries it.

For each task in the artifact it builds one #211 request — `protocol_version: 1`, a
`rr-<yyyymmdd>-<8hex>` request id, the `model` and `adapter` labels verbatim, the query,
and the task's candidates as `{id, rank, text}` in `pool_position` order — spawns the
backend as a child with the request on stdin under an end-to-end deadline, validates the
response — `{protocol_version, request_id, model, adapter, score_direction, scores: [{id,
score}]}`, exactly those six keys, with the request id and both identity labels echoed byte
for byte and every score finite — and records the scores keyed by `candidate_ref`.

The wire `id` is the candidate's 1-based pool position rendered as a string, not the
candidate reference. #213 established that `candidate_ref` is **not unique within a pool**
— its code form is `(repo, path, symbol, blob_oid)` while `code_symbols` is unique on
`(repo, path, symbol, line_start)` — and #211 refuses a repeated candidate id with
`DuplicateCandidateId`, which would decline the whole task. Pool position is unique by
construction. The response's scores are then mapped back through the position.

Two candidates in one pool that share a `candidate_ref` are a real consequence of that
same wrinkle, and the scores file #208 reads is keyed by reference. They are counted as
`ambiguous_refs`, the first occurrence's score wins, and `decide` refuses a `go` when the
count is non-zero — an honest treatment rather than a silent collapse.

The sidecar:

```jsonc
{
  "arm": "lora", "model": "cross-encoder/…@233902d2…", "adapter": "lora-v1",
  "backend": ["python3", "…/comemory_rerank.py", "score", "--adapter", "…"],
  "report_sha256": "…",                    // the artifact these scores were produced from
  "corpus_digest": "…", "knobs_hash": "…", // copied from the artifact
  "tasks": 12, "invocations": 12, "failures": 1, "fallback_rate": 0.0833,
  "candidates_submitted": 337, "candidates_scored": 306, "ambiguous_refs": 0,
  "failure_reasons": [{ "task_id": "…", "kind": "non_zero_exit", "code": 65,
                        "stderr_excerpt": "…" }],
  "latency_ms": { "p50": …, "p90": …, "p95": …, "max": …, "mean": … },
  "peak_child_rss": { "value": …, "unit": "bytes|kilobytes", "platform": "darwin" },
  "timeout_ms": 20000
}
```

`fallback_rate` is `failures / invocations`: the fraction of held-out tasks for which
production would have fallen back to the deterministic order. `latency_ms` is the child's
own wall clock — the number a `[rerank] timeout_ms` budget is set against — and is kept
strictly apart from the artifact's `latency_ms`, which is retrieval's. `peak_child_rss`
records the raw `getrusage(RUSAGE_CHILDREN)` value with its unit and platform beside it,
because that field is bytes on macOS and kilobytes on Linux and a number without its unit
is not a measurement.

A failure never aborts the run: the task simply has no scores, the arm's `scored_fraction`
drops, and both facts reach the decision.

### `comemory_qualify.py decide`

```text
comemory_qualify.py decide --report qualification.json --provenance set.provenance.json
                           --scoring base=base.scoring.json --scoring lora=lora.scoring.json
                           [--adapter-manifest comemory_adapter.json]
                           --out decision.json [--markdown decision.md]
                           [--lora-arm NAME] [--base-arm NAME]
```

The outcome vocabulary is exactly three words, and the mapping is a table rather than a
judgment call:

| Outcome | When |
| --- | --- |
| `insufficient-evidence` | The LoRA arm's #208 verdict is `inconclusive`; or an arm is missing; or a harness integrity check failed, so the numbers cannot be trusted |
| `no-go` | Every check ran and the evidence does not support enabling: verdict `regressed` or `neutral`, or `improved` with an operational budget breached |
| `go` | Verdict `improved`, every operational budget met and every integrity check passed |

The integrity checks, each of which forces `insufficient-evidence` when it fails and each
of which is named in the record:

1. The artifact's `artifact_version` and `observation_version` are the supported ones.
2. Every scoring sidecar's `corpus_digest` and `knobs_hash` equal the artifact's. This is
   what catches a corpus that moved between the two benchmark passes: a sidecar copies the
   digests of the artifact it actually scored, so comparing it against the artifact the
   verdict came from needs no second file and cannot be omitted.
3. The artifact's six pinned ranking knobs equal the ones the generated set declared, so the
   run really applied the configuration the set pins. The export's own `knobs_hash` is
   **reported** beside it rather than gated on: `build-set` freezes ACT-R decay to `0.0`, which
   is what makes a benchmark time-independent and which necessarily changes the hash, and every
   arm reorders one pool captured in one run either way. What a drift qualifies is how far the
   result generalizes to the configuration the labels came from, and the record says so.
4. `judgments_stale` summed over tasks is zero.
5. Every arm's `scored_fraction` is within `1e-9` of `1.0`.
6. `ambiguous_refs` is zero on every arm.
7. The adapter manifest's recomputed `adapter_id` matches its recorded one, its
   `dataset.holdout.sha256` matches the provenance sidecar's holdout digest — the adapter
   was qualified against the holdout that was withheld from its own training — and
   `selection.holdout_used` is `false`.
8. Neither scored arm's model label is a known non-neural label. `lexical-overlap@1` can
   never produce a `go`, because it measures no relevance quality and #212 says so in its
   own fingerprint.

**A check that could not run is never recorded as a pass.** `--adapter-manifest` is optional
because there may be no adapter yet; when it is absent, check 7 is recorded as `skipped` and
the outcome is capped at `insufficient-evidence`, since an unverified provenance chain is
exactly the thing that could hide training on the holdout. Every check appears in the record
as `pass`, `fail` or `skipped` together with the values it compared, so reading the record
tells you what was checked and not only what was concluded.

The operational budgets, declared in `comemory_train_pins.py` beside everything else:

| Budget | Value | Why |
| --- | --- | --- |
| `MAX_FALLBACK_RATE` | `0.0` | Production tolerates fallbacks by design; a qualification run that could not score every held-out task has not measured the arm it is about to claim a number for |
| `MAX_SCORING_P95_MS` | `20000` | `DEFAULT_RERANK_TIMEOUT`. A scorer whose p95 exceeds the production deadline would fall back in production, so its measured quality would not be the quality served |

`decision.json` carries the outcome, every check with its pass/fail and the values
compared, every arm's overall and per-domain summary, the paired delta and its interval,
both latency distributions, memory, fallback rate, and the full provenance chain
(`dataset_id`, `snapshot_digest`, `adapter_id`, `corpus_digest`, `knobs_hash`).
`decision.md` renders the same content as a table, including a `not measured` row for
anything that was not.

## Failure modes and edge cases

| Situation | Behavior |
| --- | --- |
| `--dataset` has no `manifest.json` | exit 65 naming the directory and the missing file |
| The manifest declares a version this recipe does not know | exit 65 naming both versions. A consumer of an unknown version must refuse, not guess |
| `train.jsonl` was truncated mid-write | its SHA-256 differs from the manifest's entry; exit 65 naming both digests |
| `holdout.jsonl` is present and corrupt | `plan` and `train` succeed unchanged. They never open it, which is exactly what the offline suite proves by corrupting it |
| `holdout.jsonl` is absent when `build-set` runs | exit 69 naming `--include-holdout`, because the withheld holdout is the point and re-exporting is the fix |
| The holdout carries no judged observation | exit 65 naming the count, the `--split` ratios and `--include-holdout`. A set with no task cannot qualify anything |
| `decide` is run with no `--adapter-manifest` | the provenance checks are recorded `skipped` and the outcome is capped at `insufficient-evidence`. A skipped check never reads as a pass |
| torch or peft is absent | `train` exits 69 with the install command; `plan`, `build-set`, `score` and `decide` are unaffected because they never import them |
| The pinned snapshot is not in the local cache | exit 69 with the `huggingface-cli download` command. The trainer never downloads inside a run unless `--allow-download` |
| Validation split is empty | exit 65. Selecting a checkpoint without validation data would mean selecting on training data or on nothing |
| Every validation observation has a single candidate | nDCG is degenerate; the run reports `validation_ndcg_at_10` as `null` for those observations, selects on validation loss, and records `selection.metric` as `validation_loss` so the record says what actually decided |
| The frozen-base digest changed | exit 70. Nothing is saved: an adapter whose base moved is not an adapter |
| The reload parity check exceeds the tolerance | exit 70 naming the observed delta and the tolerance. The adapter directory is left in place for inspection and the manifest is **not** written, so a package without a manifest is visibly incomplete |
| `--out` is a non-empty directory | exit 65 naming it and `--force`. An adapter silently mixed with an older one is unqualifiable |
| The backend exits non-zero for one task | counted as a failure with its code and stderr excerpt; the run continues; `fallback_rate` rises; `decide` refuses a `go` |
| The backend exceeds the deadline | the child is killed and reaped, counted as `timed_out`, same consequence |
| The backend's response echoes a different model or adapter | counted as `identity_mismatch`. This is #212's identity gate working, and it is a failure of the invocation, not of the harness |
| A response omits, repeats or returns a non-finite score | counted as `invalid_response` naming the divergence; no partial scores are recorded for that task |
| A task's pool is empty | no request is sent; the task is counted in `tasks` and contributes no scores |
| The artifact holds zero tasks | `score` exits 65: a scores file over no tasks would silently compare nothing |
| Two candidates in one task share a `candidate_ref` | counted in `ambiguous_refs`; the first score wins; `go` is refused |
| A scores file names a task the set does not declare | #208's `ArmScores::load` already refuses it with `EX_CONFIG`; the harness does not restate that check |
| The corpus changed between the two benchmark passes | integrity check 2 fails; outcome `insufficient-evidence` naming both digests |
| `decide` is given only a base arm, or only a LoRA arm | `insufficient-evidence` naming the missing arm. A three-arm comparison with two arms is not the comparison that was asked for |
| Nothing was trained yet | there is no LoRA arm, so the outcome is `insufficient-evidence`. This is the expected recorded outcome of this change |

## Acceptance criteria

- **AC-1:** Given a real `comemory export-dataset` output produced by the real binary over
  a real mixed corpus, `comemory_train.py plan --dataset DIR --json` prints the resolved
  recipe with every pinned value, the dataset's `dataset_id` and `snapshot_digest`, the
  per-split row counts and the holdout digest it recorded but did not read.
- **AC-2:** With `holdout.jsonl` present in that directory and overwritten with bytes that
  are not valid JSON, `plan` succeeds with byte-identical output, proving the trainer's
  readable-file allowlist is structural.
- **AC-3:** `plan` exits 65 naming the file and both digests when one byte of
  `train.jsonl` is changed after export, and exits 65 naming both versions when the
  manifest's `manifest_version` is raised to an unsupported number.
- **AC-4:** `plan` exits 65 naming the group and both files when a `group_id` is made to
  appear in both `train.jsonl` and `validation.jsonl`.
- **AC-5:** `comemory_qualify.py build-set` over a real export's holdout produces a YAML
  file that the real `comemory benchmark --set` loads and runs without error, with one task
  per judged holdout observation and one judgment per reviewed label, each carrying its
  content-version pin.
- **AC-6:** The generated set's `ranking` block pins `decay: 0.0` and the remaining knobs
  from the export's single retrieval revision; an export carrying two distinct
  `knobs_hash` values is refused with exit 65 naming both.
- **AC-7:** `comemory_qualify.py score` over a real `--report` artifact, driving the real
  `integrations/reranker/comemory_rerank.py` in its deterministic `lexical-overlap` mode as
  a real child process over real pipes, produces a scores file that `comemory benchmark
  --scores` accepts, and the resulting artifact carries two arms with
  `scored_fraction == 1.0`.
- **AC-8:** The scoring sidecar from that run reports `invocations` equal to the task
  count, `failures: 0`, `fallback_rate: 0.0`, a latency distribution with `p50 <= p95 <=
  max`, and a `peak_child_rss` object carrying a value, a unit and the platform.
- **AC-9:** When the backend command is replaced by one that exits non-zero, the same run
  completes, records that task under `failure_reasons` with `kind: "non_zero_exit"` and its
  exit code, and reports a `fallback_rate` above zero.
- **AC-10:** `comemory_qualify.py decide` over an artifact whose judged task count is below
  `budgets.min_tasks` records `insufficient-evidence` and names the #208 verdict
  `inconclusive` as the reason.
- **AC-11:** `decide` over an artifact with a single scored arm whose model label is
  `lexical-overlap@1` never records `go`: with the minimum-task budget satisfied it records
  `insufficient-evidence` for the missing arm, and with both arms supplied by that same
  non-neural label it records `insufficient-evidence` naming the non-neural check.
- **AC-12:** `decide` records `insufficient-evidence`, naming the check and both values, when
  a scoring sidecar's `corpus_digest` differs from the artifact's, and when no adapter manifest
  is supplied — and the record spells that last one `skipped`, never `pass`.
- **AC-13:** A synthetic adapter directory built by the offline suite with the exact
  `adapter_config.json` and safetensors tensor names this recipe saves is **accepted** by
  #212's own `comemory_rerank_compat.validate_adapter`; removing `classifier` from
  `modules_to_save`, or removing the head tensors from the weight file, is refused by it
  with exit code 65.
- **AC-14:** `comemory_train_pins` and `comemory_rerank_pins` agree on the model id, the
  model revision, the tokenizer identity, the head modules, the maximum length, the
  truncation and padding policies and the score column, asserted by the offline suite so a
  copy cannot silently replace the import.
- **AC-15:** `integrations/training/requirements.txt` pins exactly the versions
  `integrations/reranker/requirements.txt` pins, asserted by the offline suite; and
  `run-training-tests.sh` refuses to run the model suite when a library is absent or at a
  different version, printing the exact fix and exiting 69 with the words "the training
  suite did NOT run".
- **AC-16:** `cargo nextest run --all-features` runs the whole offline suite through a real
  `python3` child and fails, rather than skipping, when `python3` is absent.
- **AC-17:** No file under `src/`, `Cargo.toml`, `Cargo.lock`, `migrations/` or
  `docs/cli-reference.md` changes, and `.gitignore` excludes every path this workflow
  writes — adapter directories, dataset exports, benchmark artifacts, scores files and
  decision records — so model weights and user-derived training exports cannot enter a
  normal source commit.
- **AC-18:** `README.md`, `AGENTS.md`, `integrations/README.md` and
  `docs/guides/learned-reranking.md` describe the offline workflow, state that training is
  never triggered by save, sync or watch, and state that collection and reranking cannot
  run at the same time.
- **AC-19:** `integrations/training/README.md` carries a measured-results table whose rows
  are `not measured` with the exact command that fills each one, following the precedent
  `integrations/reranker/README.md` set, and the design document records the qualification
  outcome for this change as `insufficient-evidence` with the reason stated.
- **AC-20:** `build-set` run twice over one export writes byte-identical `set.yaml` and
  `set.provenance.json`, because a qualification whose own inputs are not reproducible cannot
  support a reproducible verdict.
- **AC-21:** The opt-in model suite, when its prerequisites are present, asserts on real
  weights that: the LoRA model's trainable parameters are exactly the LoRA matrices plus every
  `classifier` parameter and nothing else; the frozen-base digest is unchanged by a training
  step; a saved adapter reloads to predictions within the pinned tolerance on the fixed parity
  probe; the saved `adapter_config.json` and `adapter_model.safetensors` pass #212's
  `validate_adapter`; and the reloaded adapter scores a request through
  `comemory_rerank.py score --adapter` without the identity gate refusing it. **This suite is
  implemented and not measured in this change**, because running it needs the pinned weights
  and this change deliberately stops before the download.
- **AC-22:** The end-to-end qualification through #213 — `[rerank] enabled = true` with
  `command`, `model` and `adapter` pointing at the trained adapter, one run of each of
  `search`, `search-code`, `find` and `context`, reading `learned.applied` and
  `learned.model` / `learned.adapter`, then the two rollback levers (drop `adapter` and
  `--adapter` for base-model scoring, or `enabled = false` for none) — is documented as a
  copy-pasteable procedure in `integrations/training/README.md` and recorded as **not
  measured**, because it needs the same real weights.

## Acceptance evidence

| AC | Real input | Expected observable | Boundary / failure case | Runnable check |
| --- | --- | --- | --- | --- |
| AC-1 | `tests/common/export_dataset_corpus.rs::prepared()` + `comemory export-dataset --split 0.34,0.33,0.33 --include-holdout` | `plan --json` object with `recipe`, `dataset.dataset_id`, `rows_by_split`, `holdout.read == false` | an export with no validation rows exits 65 | `cargo nextest run --all-features lora_qualification` |
| AC-2 | the same export, `holdout.jsonl` overwritten with `not json` | identical stdout to AC-1 | — | same |
| AC-3 | the same export, one byte flipped in `train.jsonl`; separately `manifest_version: 2` | exit 65, message carries both digests / both versions | — | same |
| AC-4 | the same export, a validation row's `group_id` rewritten to a train group | exit 65 naming the group | — | same |
| AC-5 | the same export's `holdout.jsonl` | `comemory benchmark --set` exits 0; task count equals judged holdout observations | an observation with no manual label is dropped and counted | same |
| AC-6 | the same export | `ranking.decay == 0.0`, knobs equal to `retrieval_revisions[0].knobs` | a hand-edited manifest with two revisions exits 65 | same |
| AC-7 | `comemory benchmark --set --report`, then `comemory_rerank.py score --scoring lexical-overlap` | artifact with arms `deterministic` and the scored arm, both `scored_fraction >= 1.0 - 1e-9` | — | same |
| AC-8 | the same run's sidecar | counters and percentiles as stated | — | same |
| AC-9 | backend replaced with `python3 -c "raise SystemExit(65)"` | `failure_reasons[0].kind == "non_zero_exit"`, `code == 65`, `fallback_rate > 0` | — | same |
| AC-10 | the artifact from AC-7 with the default `min_tasks: 8` | `outcome == "insufficient-evidence"`, reason names `inconclusive` | — | same |
| AC-11 | the same artifact with `--min-tasks 1` | `outcome != "go"` in both configurations, reason names the check | flipping the non-neural label list to empty changes the outcome — the flip-and-revert proof | same |
| AC-12 | a copy of the artifact with one hex digit of `corpus.digest` changed | `outcome == "insufficient-evidence"`, both digests in the record | — | same |
| AC-13 | a safetensors file written by the offline suite with the standard library alone | `validate_adapter` returns the fingerprint facts; both mutations raise `RerankError` with `code == 65` | an adapter config naming a different base model is refused | `python3 -m unittest` through the Rust test |
| AC-14 | both pins modules imported in one process | every compared value equal | — | same |
| AC-15 | both `requirements.txt` files | identical version map | a stubbed module at the wrong version makes `run-training-tests.sh` exit 69 | `bash integrations/training/run-training-tests.sh` |
| AC-16 | `PATH` without `python3` | the Rust test fails naming the prerequisite | — | `cargo nextest run` |
| AC-17 | `git diff --name-only origin/main...HEAD` | no path under `src/`, `Cargo.*`, `migrations/` | — | `bash scripts/check-all.sh`, `cargo nextest run --all-features` |
| AC-18 | the four documents | the stated sentences present | — | review + `bash scripts/check-all.sh` |
| AC-19 | `integrations/training/README.md` | a table of `not measured` rows with commands | — | review |
| AC-20 | one real export, two `build-set` runs into different directories | both files byte-identical | — | `cargo nextest run --all-features lora_qualification` |
| AC-21 | the pinned weights and the pinned libraries | every assertion listed passes | a missing library or snapshot makes the script exit 69 without running | `bash integrations/training/run-training-tests.sh` — **not run in this change** |
| AC-22 | a trained adapter and a real `config.toml` | `learned.applied == true` with the adapter label echoed; `enabled = false` restores byte-identical output | — | the documented procedure — **not run in this change** |

Every check above runs on real data: a real SQLite store, the real `comemory` binary, real
`comemory export-dataset` output, real `python3` child processes over real pipes, and
#212's real compatibility validator. No test asserts that a mock returned what it was told.

## Documentation impact

| Surface | Change |
| --- | --- |
| `integrations/training/README.md` | New. The workflow, the pins, the exit codes, the opt-in suite and the not-measured table |
| `integrations/README.md` | A third section for `training/`, stating that it is opt-in and not embedded |
| `README.md` | One paragraph in the learned-reranking area pointing at the offline workflow and stating that training never runs automatically |
| `AGENTS.md` | The `integrations/` note gains the training directory; the testing section records the second `python3`-dependent suite and the second opt-in script |
| `docs/guides/learned-reranking.md` | A section on where an adapter comes from, the two-phase collection/serving alternation, and that rollback is a configuration change |
| `docs/designs/2026-09-18-lora-adapter-training.md` | This document |
| `.gitignore` | Adapter directories, dataset exports, benchmark artifacts, scores files and decision records |
| `docs/cli-reference.md`, `docs/scenarios/` | Unchanged — no CLI subcommand or flag is added |

## Open Questions

1. **Should the qualification also run through `[rerank]` end to end, not only through
   `comemory benchmark`?** Resolved, non-blocking: yes, and it is AC-22. It cannot be
   measured here because it needs real weights, so it ships as a documented procedure listed
   as not measured rather than as a check that could not run and would report nothing.
2. **Should `MAX_FALLBACK_RATE` be zero?** Resolved, non-blocking: yes for a qualification
   run, and the README states the distinction from production, where a fallback is the
   designed no-regression path. An operator who wants a different bar edits one pinned
   constant and the decision record shows the value used.
3. **Is validation nDCG@10 the right selection metric when a validation observation has
   very few judged candidates?** Resolved, non-blocking: the run records the per-epoch
   validation loss beside it and falls back to loss, naming which metric decided, when no
   validation observation has two or more judged candidates.
