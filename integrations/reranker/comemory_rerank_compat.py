"""Compatibility validation for the pinned base model and a PEFT adapter.

Split out of `comemory_rerank_model.py` along the seam the design document
names: this file decides whether a checkpoint may be scored with at all, and
that file decides how to score with it.

The rule these checks exist to enforce is that a relevance head is never
silently fresh. Transformers attaches a randomly initialized classification
head when a checkpoint carries none, and PEFT's troubleshooting guide records
the matching adapter failure: a LoRA checkpoint that omits the head from
`modules_to_save` reloads a different random head on every load, with no error
anywhere. Both are refused here.

Head provenance is proven from the checkpoint's own tensor names rather than
from a loading report, because the report's shape has changed across
Transformers versions while the tensor names have not.
"""

from __future__ import annotations

import hashlib
import json
import os
import platform
import sys
from typing import Sequence

import comemory_rerank_pins as pins


def import_core() -> tuple:
    """Import torch and transformers, or refuse with the install command."""
    try:
        import torch
        import transformers
    except ImportError as exc:
        raise pins.RerankError(
            "the cross-encoder backend needs torch and transformers: "
            + str(exc) + ". " + pins.INSTALL_HINT,
            pins.EX_UNAVAILABLE,
        ) from exc
    return torch, transformers


def import_peft():
    """Import PEFT's model loader, or refuse with the install command."""
    try:
        from peft import PeftModel
    except ImportError as exc:
        raise pins.RerankError(
            "peft is required for --adapter: " + str(exc) + ". " + pins.INSTALL_HINT,
            pins.EX_UNAVAILABLE,
        ) from exc
    return PeftModel


def call_hub(loader, *args, **kwargs):
    """Run a Hub loader, turning a missing local snapshot into a clear refusal."""
    try:
        return loader(*args, **kwargs)
    except (OSError, ValueError) as exc:
        raise pins.RerankError(
            "could not load the pinned model or adapter: " + str(exc)
            + ". " + pins.DOWNLOAD_HINT + " (or pass --allow-download)",
            pins.EX_UNAVAILABLE,
        ) from exc


def check_architecture(model, config: dict) -> None:
    """Refuse a checkpoint that is not the pinned sequence-classification shape."""
    found = list(getattr(model.config, "architectures", []) or [])
    if found and not set(found) & set(pins.EXPECTED_ARCHITECTURES):
        raise pins.RerankError(
            "model " + config["model_id"] + " is " + ", ".join(found)
            + "; expected one of " + ", ".join(pins.EXPECTED_ARCHITECTURES),
            pins.EX_DATAERR,
        )
    labels = int(model.config.num_labels)
    if labels <= int(config["score_column"]):
        raise pins.RerankError(
            "model has " + str(labels) + " label(s), so --score-column "
            + str(config["score_column"]) + " selects nothing",
            pins.EX_DATAERR,
        )


def head_provenance(model, config: dict) -> list:
    """Prove every relevance-head parameter came from the checkpoint.

    A head parameter absent from the weight file was newly initialized, which
    means scoring with it ranks by random numbers. Refused, not warned about.
    """
    head_modules = config["head_modules"]
    names = sorted(
        name for name, _ in model.named_parameters() if name.split(".")[0] in head_modules
    )
    if not names:
        raise pins.RerankError(
            "no parameter belongs to the declared head module(s) " + ", ".join(head_modules)
            + "; pass --head-module to name this architecture's head",
            pins.EX_DATAERR,
        )
    present = checkpoint_keys(base_weights_path(config))
    missing = [name for name in names if name not in present]
    if missing:
        raise pins.RerankError(
            "the checkpoint for " + config["model_id"] + " does not carry "
            + ", ".join(missing)
            + "; scoring would use randomly initialized relevance weights",
            pins.EX_DATAERR,
        )
    return names


def base_weights_path(config: dict) -> str:
    """Locate the pinned weight file for the configured base model."""
    for name in pins.BASE_WEIGHT_FILES:
        path = hub_file(
            config["model_id"], name, config["model_revision"], config["allow_download"]
        )
        if path:
            return path
    raise pins.RerankError(
        "no weight file (" + ", ".join(pins.BASE_WEIGHT_FILES) + ") for "
        + config["model_id"] + " at the pinned revision. " + pins.DOWNLOAD_HINT,
        pins.EX_UNAVAILABLE,
    )


def validate_adapter(config: dict) -> dict:
    """Check an adapter checkpoint before anything heavy is loaded.

    Returns the facts worth recording in the fingerprint, or an empty dict when
    the process is configured for base-only scoring.
    """
    adapter = config.get("adapter")
    if not adapter:
        return {}
    config_path = os.path.join(adapter, pins.ADAPTER_CONFIG_FILE)
    if not os.path.isfile(config_path):
        raise pins.RerankError(
            "adapter " + adapter + " has no " + pins.ADAPTER_CONFIG_FILE, pins.EX_DATAERR
        )
    with open(config_path, "r", encoding="utf-8") as handle:
        declared = json.load(handle)
    _require_value(declared, "peft_type", pins.ADAPTER_PEFT_TYPE)
    _require_value(declared, "task_type", pins.ADAPTER_TASK_TYPE)
    _check_adapter_base(declared, config)
    weights = _check_adapter_head(adapter, declared, config["head_modules"])
    return {
        "base_model_name_or_path": declared.get("base_model_name_or_path"),
        "task_type": declared.get("task_type"),
        "peft_type": declared.get("peft_type"),
        "modules_to_save": sorted(declared.get("modules_to_save") or []),
        "config_sha256": sha256(config_path),
        "weights_sha256": sha256(weights),
    }


def _check_adapter_base(declared: dict, config: dict) -> None:
    """Refuse an adapter trained against a different base model or revision."""
    problems = []
    base = declared.get("base_model_name_or_path")
    revision = declared.get("revision")
    if base != config["model_id"]:
        problems.append(
            "base_model_name_or_path " + repr(base) + " is not " + repr(config["model_id"])
        )
    if revision and revision != config["model_revision"]:
        problems.append(
            "revision " + repr(revision) + " is not " + repr(config["model_revision"])
        )
    if not problems:
        return
    message = "adapter/base mismatch: " + "; ".join(problems)
    if config["allow_base_mismatch"]:
        diag("warning: " + message + " (allowed by --allow-base-mismatch)")
        return
    raise pins.RerankError(
        message + ". An adapter checkpoint needs the base model it was trained on; pass "
        "--allow-base-mismatch only if you know these are compatible.",
        pins.EX_DATAERR,
    )


def _check_adapter_head(adapter: str, declared: dict, head_modules: Sequence[str]) -> str:
    """Refuse an adapter whose saved state omits the trained relevance head."""
    saved = list(declared.get("modules_to_save") or [])
    missing = [module for module in head_modules if module not in saved]
    if missing:
        raise pins.RerankError(
            "adapter " + adapter + " does not save " + ", ".join(missing)
            + " in modules_to_save, so PEFT would attach a freshly initialized relevance "
            "head and every load would score differently. Retrain with modules_to_save="
            + repr(list(head_modules)) + ".",
            pins.EX_DATAERR,
        )
    weights = _first_existing(adapter, pins.ADAPTER_WEIGHT_FILES)
    if not weights:
        raise pins.RerankError(
            "adapter " + adapter + " has no " + " or ".join(pins.ADAPTER_WEIGHT_FILES),
            pins.EX_DATAERR,
        )
    keys = checkpoint_keys(weights)
    for module in head_modules:
        marker = "." + module + ".modules_to_save."
        if not any(marker in key for key in keys):
            raise pins.RerankError(
                "adapter " + adapter + " declares " + module + " in modules_to_save but its "
                "weight file carries no tensor for it; the trained relevance head is not in "
                "this checkpoint",
                pins.EX_DATAERR,
            )
    return weights


def hub_file(repo_id: str, filename: str, revision: str, allow_download: bool) -> str | None:
    """Resolve one repository file to a local path, or `None` when it is absent."""
    try:
        from huggingface_hub import hf_hub_download
    except ImportError as exc:
        raise pins.RerankError(
            "huggingface_hub is required: " + str(exc) + ". " + pins.INSTALL_HINT,
            pins.EX_UNAVAILABLE,
        ) from exc
    try:
        return hf_hub_download(
            repo_id, filename, revision=revision, local_files_only=not allow_download
        )
    except (OSError, ValueError):
        return None


def checkpoint_keys(path: str) -> set:
    """The tensor names a checkpoint carries.

    A safetensors file is read from its own header with the standard library
    alone — an eight-byte little-endian length followed by that many bytes of
    JSON — so the one check that decides whether a relevance head is real does
    not depend on any library's API staying put. A pickle checkpoint has no such
    header and needs torch.
    """
    if path.endswith(".safetensors"):
        with open(path, "rb") as handle:
            raw = handle.read(8)
            if len(raw) != 8:
                raise pins.RerankError(path + " is not a safetensors file", pins.EX_DATAERR)
            header = json.loads(handle.read(int.from_bytes(raw, "little")).decode("utf-8"))
        return {key for key in header if key != "__metadata__"}
    torch, _ = import_core()
    return set(torch.load(path, map_location="cpu", weights_only=True).keys())


def resolve_dtype(torch, name: str):
    """Map the pinned dtype name onto a torch dtype."""
    dtype = getattr(torch, name, None)
    if dtype is None:
        raise pins.RerankError("unknown dtype " + repr(name), pins.EX_USAGE)
    return dtype


def resolve_device(torch, name: str) -> str:
    """Resolve the requested device, refusing one this machine cannot provide."""
    if name == "auto":
        if torch.cuda.is_available():
            return "cuda"
        return "mps" if torch.backends.mps.is_available() else "cpu"
    if name == "cuda" and not torch.cuda.is_available():
        raise pins.RerankError("--device cuda requested but CUDA is unavailable",
                               pins.EX_UNAVAILABLE)
    if name == "mps" and not torch.backends.mps.is_available():
        raise pins.RerankError("--device mps requested but MPS is unavailable",
                               pins.EX_UNAVAILABLE)
    return name


def library_versions(with_peft: bool) -> dict:
    """Record the versions that actually loaded, not the ones that were pinned."""
    versions = {"python": platform.python_version()}
    names = ["torch", "transformers", "tokenizers", "safetensors", "huggingface_hub"]
    if with_peft:
        names.append("peft")
    for name in names:
        module = sys.modules.get(name)
        versions[name] = getattr(module, "__version__", None) if module else None
    return versions


def sha256(path: str) -> str:
    """Hex SHA-256 of a file, read in bounded chunks."""
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def diag(message: str) -> None:
    """Write one diagnostic line to stderr, which the caller never parses."""
    sys.stderr.write(pins.DIAG_PREFIX + message + "\n")
    sys.stderr.flush()


def _require_value(declared: dict, key: str, expected: str) -> None:
    """Refuse an adapter configured for something other than what is supported."""
    found = declared.get(key)
    if found != expected:
        raise pins.RerankError(
            "adapter " + key + " is " + repr(found) + ", expected " + repr(expected),
            pins.EX_DATAERR,
        )


def _first_existing(directory: str, names: Sequence[str]) -> str | None:
    """The first of `names` that exists in `directory`."""
    for name in names:
        path = os.path.join(directory, name)
        if os.path.isfile(path):
            return path
    return None
