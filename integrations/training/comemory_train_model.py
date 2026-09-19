"""Build the LoRA model, prove what trains, and prove a save survives a reload.

Everything that decides whether a checkpoint may be used at all is delegated to
`integrations/reranker/comemory_rerank_compat`, the module the serving path
already uses. The architecture check, the head-provenance proof, the dtype and
device resolution and the adapter validation are therefore literally the same
code on both sides: an adapter this recipe saves is checked by the function that
will refuse it in production, not by a second implementation that agrees today.

Three guarantees are established here, and each is a refusal rather than a log
line. The trainable set is exactly the LoRA matrices plus the relevance head.
The frozen base is byte-identical before and after training, proven by digest
and not by a `requires_grad` flag. And a saved adapter reloads to the same
predictions within the pinned tolerance, measured on a fixed probe.
"""

from __future__ import annotations

import hashlib
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))

import comemory_train_manifest as manifest  # noqa: E402
import comemory_train_pins as pins  # noqa: E402

_BACKEND = os.path.join(os.path.dirname(os.path.dirname(os.path.realpath(__file__))), "reranker")
if _BACKEND not in sys.path:
    sys.path.insert(0, _BACKEND)

import comemory_rerank_compat as compat  # noqa: E402


def backend_config(device: str, allow_download: bool, adapter: str | None = None) -> dict:
    """The configuration object the shared compatibility checks read."""
    return {
        "model_id": pins.MODEL_ID,
        "model_revision": pins.MODEL_REVISION,
        "tokenizer_id": pins.TOKENIZER_ID,
        "tokenizer_revision": pins.TOKENIZER_REVISION,
        "dtype": pins.TRAIN_DTYPE,
        "device": device,
        "allow_download": allow_download,
        "head_modules": list(pins.HEAD_MODULES),
        "score_column": pins.SCORE_COLUMN,
        "digest_base_weights": True,
        "adapter": adapter,
        "allow_base_mismatch": False,
    }


def torch_module():
    """The torch module, imported through the shared refusal path."""
    torch, _transformers = compat.import_core()
    return torch


def load_base(config: dict):
    """Load and validate the pinned base model and its tokenizer.

    Returns `(tokenizer, model, head_parameters)` and NOT the torch module. A
    caller that needs torch asks `torch_module` for it, so no call site
    destructures a local named after the library, and the return order carries
    no import in it to go stale.
    """
    torch, transformers = compat.import_core()
    tokenizer = compat.call_hub(
        transformers.AutoTokenizer.from_pretrained,
        config["tokenizer_id"],
        revision=config["tokenizer_revision"],
        local_files_only=not config["allow_download"],
    )
    model = compat.call_hub(
        transformers.AutoModelForSequenceClassification.from_pretrained,
        config["model_id"],
        revision=config["model_revision"],
        dtype=compat.resolve_dtype(torch, config["dtype"]),
        local_files_only=not config["allow_download"],
    )
    compat.check_architecture(model, config)
    head = compat.head_provenance(model, config)
    return tokenizer, model, head


def attach_adapter(model):
    """Wrap the frozen base in the pinned LoRA configuration.

    `base_model_name_or_path` and `revision` are set explicitly rather than left
    for PEFT to infer, because the reference backend compares both against its
    own pins before it will score with an adapter. Leaving `revision` unset
    would make that half of the check vacuously pass.
    """
    try:
        from peft import LoraConfig, get_peft_model
    except ImportError as exc:
        raise pins.RecipeError(
            "peft is required to train an adapter: " + str(exc) + ". " + pins.INSTALL_HINT,
            pins.EX_UNAVAILABLE,
        ) from exc
    config = LoraConfig(
        task_type=pins.ADAPTER_TASK_TYPE,
        r=pins.LORA_R,
        lora_alpha=pins.LORA_ALPHA,
        lora_dropout=pins.LORA_DROPOUT,
        target_modules=list(pins.LORA_TARGET_MODULES),
        bias=pins.LORA_BIAS,
        modules_to_save=list(pins.MODULES_TO_SAVE),
    )
    config.base_model_name_or_path = pins.MODEL_ID
    config.revision = pins.MODEL_REVISION
    return get_peft_model(model, config)


def audit(model) -> dict:
    """Prove the trainable set is exactly the adapter plus the relevance head."""
    trainable, total, names = [], 0, []
    for name, parameter in model.named_parameters():
        total += parameter.numel()
        if parameter.requires_grad:
            trainable.append(parameter.numel())
            names.append(name)
    trained = sum(trainable)
    lora = [n for n in names if ".lora_" in n]
    head = [n for n in names if _is_head(n)]
    other = [n for n in names if n not in lora and n not in head]
    if not lora:
        raise pins.RecipeError(
            "no LoRA parameter is trainable, so this run would fit nothing",
            pins.EX_SOFTWARE,
        )
    if not head:
        raise pins.RecipeError(
            "no relevance-head parameter is trainable; the reference backend refuses an "
            "adapter whose saved head is absent, so a frozen head cannot be saved either",
            pins.EX_SOFTWARE,
        )
    if other:
        raise pins.RecipeError(
            "parameter " + other[0] + " is trainable but is neither a LoRA matrix nor part "
            "of " + ", ".join(pins.HEAD_MODULES) + "; the base must stay frozen",
            pins.EX_SOFTWARE,
        )
    fraction = trained / total if total else 0.0
    if fraction > pins.TRAINABLE_FRACTION_MAX:
        raise pins.RecipeError(
            "trainable fraction " + format(fraction, ".4f") + " exceeds the ceiling "
            + str(pins.TRAINABLE_FRACTION_MAX) + "; this recipe adapts attention "
            "projections and the head, nothing else",
            pins.EX_SOFTWARE,
        )
    return {
        "parameters": trained,
        "total": total,
        "fraction": fraction,
        "lora_parameters": len(lora),
        "head_parameters": len(head),
        "names": sorted(names),
    }


def frozen_digest(torch, model) -> dict:
    """SHA-256 over every parameter the adapter is not allowed to move.

    A `requires_grad` assertion proves only what was intended. This proves what
    happened: the same digest taken before the first optimizer step and after
    the last one must be equal, or the base moved.
    """
    digest = hashlib.sha256()
    counted = 0
    for name, parameter in sorted(model.named_parameters(), key=lambda item: item[0]):
        if ".lora_" in name or _is_head(name):
            continue
        counted += parameter.numel()
        digest.update(name.encode("utf-8"))
        digest.update(parameter.detach().to(torch.float32).cpu().numpy().tobytes())
    return {"sha256": digest.hexdigest(), "parameters": counted}


def encode(tokenizer, queries: list, texts: list, device):
    """Tokenize one batch exactly as the serving path does.

    The four values below are imported from the backend's pins, so train-time
    and serve-time preprocessing cannot drift: a difference between the two arms
    must come from the adapter, which is the whole point of the comparison.

    `queries` is a list rather than one string because a training batch mixes
    observations while a scoring batch does not. The serving path's call is the
    special case where every entry is the same query, and it tokenizes to the
    same tensors.
    """
    encoded = tokenizer(
        list(queries),
        list(texts),
        padding=pins.PADDING,
        truncation=pins.TRUNCATION,
        max_length=pins.MAX_LENGTH,
        return_tensors="pt",
    )
    return {key: value.to(device) for key, value in encoded.items()}


def score_pairs(model, tokenizer, torch, pairs: list) -> list:
    """The relevance logit for every `(query, text)` pair, in submitted order."""
    scores: list = []
    device = next(model.parameters()).device
    for start in range(0, len(pairs), pins.EVAL_BATCH_SIZE):
        window = pairs[start : start + pins.EVAL_BATCH_SIZE]
        encoded = encode(
            tokenizer, [q for q, _ in window], [text for _, text in window], device
        )
        with torch.inference_mode():
            logits = model(**encoded).logits
        column = logits[:, pins.SCORE_COLUMN].to(torch.float32).cpu().tolist()
        scores.extend(float(value) for value in column)
    return scores


def parity_pairs(count: int) -> list:
    """The fixed probe both sides of a save/reload comparison are scored on."""
    return [
        (pins.PARITY_QUERY, pins.PARITY_TEXT.format(index=index)) for index in range(count)
    ]


def save(model, directory: str) -> list:
    """Write the adapter, then record the digest of every file it wrote."""
    model.save_pretrained(directory)
    written = []
    for name in sorted(os.listdir(directory)):
        path = os.path.join(directory, name)
        if os.path.isfile(path) and name != pins.ADAPTER_MANIFEST_FILE:
            written.append({"path": name, "sha256": compat.sha256(path)})
    return written


def reload_and_compare(directory: str, before: list, device: str, allow_download: bool) -> dict:
    """Reload the saved adapter and compare its predictions with the pre-save ones."""
    config = backend_config(device, allow_download, adapter=directory)
    compat.validate_adapter(config)
    torch = torch_module()
    tokenizer, base, _head = load_base(config)
    peft_model = compat.import_peft()
    adapted = compat.call_hub(
        peft_model.from_pretrained, base, directory, local_files_only=not allow_download
    )
    adapted.eval()
    adapted.to(compat.resolve_device(torch, device))
    after = score_pairs(adapted, tokenizer, torch, parity_pairs(len(before)))
    delta = max((abs(a - b) for a, b in zip(after, before)), default=0.0)
    passed = delta <= pins.RELOAD_MAX_ABS_DELTA
    result = {
        "probe_pairs": len(before),
        "max_abs_delta": delta,
        "tolerance": pins.RELOAD_MAX_ABS_DELTA,
        "passed": passed,
    }
    if not passed:
        raise pins.RecipeError(
            "a reloaded adapter scores the parity probe " + format(delta, ".3e")
            + " away from the model that saved it, above the tolerance "
            + str(pins.RELOAD_MAX_ABS_DELTA) + "; the package is left in place for "
            "inspection and no manifest was written",
            pins.EX_SOFTWARE,
        )
    return result


def verify(directory: str, candidates: int, allow_download: bool) -> int:
    """Re-establish a saved adapter's guarantees on the machine that will serve it."""
    path = os.path.join(directory, pins.ADAPTER_MANIFEST_FILE)
    if not os.path.isfile(path):
        raise pins.RecipeError(
            directory + " has no " + pins.ADAPTER_MANIFEST_FILE
            + "; a package without its manifest is incomplete", pins.EX_DATAERR
        )
    with open(path, "r", encoding="utf-8") as handle:
        recorded = json.load(handle)
    recomputed = manifest.adapter_id(recorded)
    if recomputed != recorded.get("adapter_id"):
        raise pins.RecipeError(
            "adapter_id " + str(recorded.get("adapter_id")) + " does not match the "
            "manifest it describes, which recomputes to " + recomputed, pins.EX_DATAERR
        )
    config = backend_config("cpu", allow_download, adapter=directory)
    compat.validate_adapter(config)
    torch = torch_module()
    tokenizer, base, _head = load_base(config)
    peft_model = compat.import_peft()
    adapted = compat.call_hub(
        peft_model.from_pretrained, base, directory, local_files_only=not allow_download
    )
    adapted.eval()
    scores = score_pairs(adapted, tokenizer, torch, parity_pairs(candidates))
    sys.stdout.write(
        json.dumps(
            {
                "adapter_id": recomputed,
                "adapter_label": recorded.get("adapter_label"),
                "probe_pairs": len(scores),
                "recorded_reload_check": recorded.get("reload_check"),
                "scores": scores,
            },
            indent=2,
            sort_keys=True,
        )
        + "\n"
    )
    return pins.EX_OK


def _is_head(name: str) -> bool:
    """Whether a parameter name belongs to a declared relevance-head module.

    PEFT renames a `modules_to_save` parameter, so `classifier.weight` becomes
    `base_model.model.classifier.modules_to_save.default.weight` alongside an
    `original_module` copy of the untouched base value. Matching on the dotted
    segment covers both spellings and the unwrapped base name.
    """
    segments = name.split(".")
    return any(module in segments for module in pins.HEAD_MODULES)
