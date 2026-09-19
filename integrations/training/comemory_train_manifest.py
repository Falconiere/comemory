"""The resolved recipe, and the immutable manifest an adapter is packaged with.

Two things live here because they are one thing seen twice. `recipe` is every
pinned value as a plain object, which `comemory_train.py plan` prints and which
`train` writes verbatim into the package. `build` wraps that around the dataset
provenance, the parameter audit, the selection record, the reload check and the
hardware, and closes it with `adapter_id` — the digest of everything above.

"Immutable" is not a filesystem permission. It means the manifest digests its
own inputs and its own outputs, so changing any of them makes the recorded
`adapter_id` stop matching the recomputed one, and both `comemory_train.py
verify` and `comemory_qualify.py decide` recompute it.

The whole module is standard library only, so `plan` — and the offline test
suite — can resolve and print a complete recipe on a machine with no torch.
"""

from __future__ import annotations

import hashlib
import json
import platform
import sys

import comemory_train_pins as pins

MANIFEST_VERSION = pins.ADAPTER_MANIFEST_VERSION

# Written into every package. These are the things a reader must know before
# trusting a number the adapter produced, and none of them is discoverable from
# the weights.
LIMITATIONS = (
    "Trained on one comemory corpus's reviewed judgments. Relevance learned from "
    "one operator's memories, code index and documents does not transfer to another "
    "corpus without re-measurement.",
    "Valid only for the base model and revision recorded under `base`. PEFT will "
    "happily load these weights onto a different checkpoint; the reference backend "
    "refuses to, and that refusal is the protection.",
    "Passages longer than the pinned maximum length are truncated before scoring, "
    "so a long document is judged on its head. Training and serving truncate "
    "identically, which makes the bias consistent rather than absent.",
    "Graded relevance is rescaled into [0, 1] and fitted with binary cross entropy. "
    "The model learns a calibrated-looking score, not a ranking loss; its ordering "
    "quality is whatever the qualification measured and nothing more.",
    "Quality is only what the qualification report recorded against predeclared "
    "budgets. An adapter whose report says `insufficient-evidence` has not been "
    "shown to help, and enabling it on that basis is not supported.",
)


def recipe(overrides: dict | None = None) -> dict:
    """Every pinned value of the recipe, with any explicit override applied."""
    applied = dict(overrides or {})
    return {
        "recipe_version": pins.RECIPE_VERSION,
        "seed": applied.get("seed", pins.SEED),
        "deterministic_algorithms": applied.get(
            "deterministic_algorithms", pins.DETERMINISTIC_ALGORITHMS
        ),
        "base": {
            "model_id": pins.MODEL_ID,
            "model_revision": pins.MODEL_REVISION,
            "tokenizer_id": pins.TOKENIZER_ID,
            "tokenizer_revision": pins.TOKENIZER_REVISION,
            "expected_architectures": list(pins.EXPECTED_ARCHITECTURES),
            "expected_num_labels": pins.EXPECTED_NUM_LABELS,
            "head_modules": list(pins.HEAD_MODULES),
        },
        "lora": {
            "r": pins.LORA_R,
            "alpha": pins.LORA_ALPHA,
            "dropout": pins.LORA_DROPOUT,
            "target_modules": list(pins.LORA_TARGET_MODULES),
            "bias": pins.LORA_BIAS,
            "task_type": pins.ADAPTER_TASK_TYPE,
            "peft_type": pins.ADAPTER_PEFT_TYPE,
            "modules_to_save": list(pins.MODULES_TO_SAVE),
        },
        "objective": {
            "loss": pins.OBJECTIVE,
            "max_relevance": pins.MAX_RELEVANCE,
            "target": "relevance / max_relevance",
            "pos_weight": pins.POS_WEIGHT,
            "score_column": pins.SCORE_COLUMN,
            "score_direction": pins.SCORE_DIRECTION,
        },
        "schedule": {
            "epochs": applied.get("epochs", pins.EPOCHS),
            "learning_rate": pins.LEARNING_RATE,
            "weight_decay": pins.WEIGHT_DECAY,
            "warmup_ratio": pins.WARMUP_RATIO,
            "lr_schedule": pins.LR_SCHEDULE,
            "max_grad_norm": pins.MAX_GRAD_NORM,
            "train_batch_size": pins.TRAIN_BATCH_SIZE,
            "eval_batch_size": pins.EVAL_BATCH_SIZE,
            "optimizer": pins.OPTIMIZER,
            "adam_betas": list(pins.ADAM_BETAS),
            "adam_eps": pins.ADAM_EPS,
        },
        "precision": {
            "dtype": pins.TRAIN_DTYPE,
            "autocast": pins.AUTOCAST,
            "device": applied.get("device", "cpu"),
        },
        "tokenization": {
            "max_length": pins.MAX_LENGTH,
            "truncation": pins.TRUNCATION,
            "padding": pins.PADDING,
        },
        "selection": {
            "metric": pins.SELECTION_METRIC,
            "k": pins.SELECTION_K,
            "fallback_metric": pins.SELECTION_FALLBACK_METRIC,
            "splits_consulted": [pins.VALIDATION_SPLIT],
        },
        "audit": {
            "trainable_fraction_max": pins.TRAINABLE_FRACTION_MAX,
            "reload_max_abs_delta": pins.RELOAD_MAX_ABS_DELTA,
            "parity_pairs": pins.PARITY_PAIRS,
        },
    }


def dataset_block(dataset) -> dict:
    """The provenance of the data an adapter was fitted to."""
    return {
        "directory": dataset.directory,
        "dataset_id": dataset.dataset_id,
        "snapshot_digest": dataset.snapshot_digest,
        "manifest_sha256": dataset.manifest_sha256,
        "readable_files": list(pins.READABLE_FILES),
        "files_read": dataset.files_read,
        "holdout": dataset.holdout,
        "split": dataset.split_policy,
        "rows_by_split": dataset.rows_by_split(),
        "rows_by_domain": dataset.rows_by_domain(),
        "rows_by_relevance": dataset.rows_by_relevance(),
        "train_observations_with_a_positive": dataset.positive_observations(),
        "dropped": dataset.counters,
        "retrieval_revisions": dataset.retrieval_revisions,
    }


def build(**parts) -> dict:
    """Assemble the adapter manifest and close it with its own digest."""
    manifest = {
        "manifest_version": MANIFEST_VERSION,
        "adapter_label": parts["adapter_label"],
        "recipe": parts["recipe"],
        "dataset": parts["dataset"],
        "base_checkpoint": parts["base_checkpoint"],
        "trainable": parts["trainable"],
        "selection": parts["selection"],
        "reload_check": parts["reload_check"],
        "hardware": parts["hardware"],
        "libraries": parts["libraries"],
        "limitations": list(LIMITATIONS),
        "files": parts["files"],
    }
    manifest["adapter_id"] = adapter_id(manifest)
    return manifest


def adapter_id(manifest: dict) -> str:
    """`a-` plus the first sixteen hex characters of the manifest's own digest.

    The same construction the dataset export uses for `dataset_id`, and computed
    over the manifest with any recorded id removed, so recomputing it is a real
    check rather than a digest of a digest.
    """
    without = {key: value for key, value in manifest.items() if key != "adapter_id"}
    body = json.dumps(without, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return "a-" + hashlib.sha256(body.encode("utf-8")).hexdigest()[:16]


def hardware(device: str, peak_rss_bytes: int | None, seconds: float | None) -> dict:
    """Where the run happened and what it cost.

    `peak_rss_bytes` is recorded with its unit named on the platform that
    produced it, because `getrusage` reports bytes on macOS and kilobytes on
    Linux and a number without its unit is not a measurement.
    """
    return {
        "platform": sys.platform,
        "system": platform.system(),
        "machine": platform.machine(),
        "python": platform.python_version(),
        "device": device,
        "peak_rss": {
            "value": peak_rss_bytes,
            "unit": "bytes" if sys.platform == "darwin" else "kilobytes",
            "platform": sys.platform,
        },
        "train_seconds": seconds,
    }


def write(path: str, manifest: dict) -> None:
    """Write a manifest as stable, pretty-printed, key-sorted JSON."""
    body = json.dumps(manifest, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(body)
