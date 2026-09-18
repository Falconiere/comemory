"""The pinned Transformers cross-encoder, with PEFT adapter loading.

Base-only and adapted inference run through one code path, so tokenization,
truncation, padding, batching and the logit column are identical by
construction rather than by review. That is the whole point of the comparison
#208 and #214 will make: a difference between the two must come from the
adapter, not from the preprocessing.

This module is imported only when `--scoring cross-encoder` is selected, so a
machine with no torch can still run the deterministic mode, the warm client and
`--help`. Whether a checkpoint may be scored with at all is
`comemory_rerank_compat`'s question; this file answers how to score with it.
"""

from __future__ import annotations

import os
import time
from typing import Sequence

import comemory_rerank_compat as compat
import comemory_rerank_pins as pins


class CrossEncoderScorer:
    """Score query/passage pairs with the pinned sequence-classification model."""

    def __init__(self, **config) -> None:
        self._config = config
        self._model_label = config.get("model_label") or pins.model_label(
            config["model_id"], config["model_revision"]
        )
        adapter = config.get("adapter")
        self._adapter_label = config.get("adapter_label") or (
            os.path.basename(os.path.normpath(adapter)) if adapter else None
        )
        self._model = None
        self._tokenizer = None
        self._torch = None
        self._details: dict = {}

    @property
    def model_label(self) -> str:
        """The model identity this process answers to."""
        return self._model_label

    @property
    def adapter_label(self) -> str | None:
        """The adapter identity this process answers to, `None` for base-only."""
        return self._adapter_label

    def load(self) -> int:
        """Load, validate and freeze the model. Returns the cost in milliseconds.

        The adapter configuration is checked first, before a tokenizer or a set
        of weights is touched, so an unusable adapter costs milliseconds rather
        than a full model load.
        """
        if self._model is not None:
            return 0
        started = time.perf_counter()
        torch, transformers = compat.import_core()
        self._torch = torch
        adapter_facts = compat.validate_adapter(self._config)
        self._tokenizer = compat.call_hub(
            transformers.AutoTokenizer.from_pretrained,
            self._config["tokenizer_id"],
            revision=self._config["tokenizer_revision"],
            local_files_only=not self._config["allow_download"],
        )
        model = compat.call_hub(
            transformers.AutoModelForSequenceClassification.from_pretrained,
            self._config["model_id"],
            revision=self._config["model_revision"],
            dtype=compat.resolve_dtype(torch, self._config["dtype"]),
            local_files_only=not self._config["allow_download"],
        )
        compat.check_architecture(model, self._config)
        head_parameters = compat.head_provenance(model, self._config)
        self._record(model, head_parameters, adapter_facts)
        if adapter_facts:
            model = self._attach_adapter(model, head_parameters)
        model.eval()
        model.to(compat.resolve_device(torch, self._config["device"]))
        self._details["device"] = str(model.device)
        self._model = model
        return int((time.perf_counter() - started) * 1000)

    def score(self, query: str, texts: Sequence[str]) -> list:
        """Score every candidate text against `query`, in submitted order."""
        if self._model is None:
            self.load()
        scores: list = []
        batch = int(self._config["batch_size"])
        for start in range(0, len(texts), batch):
            scores.extend(self._score_batch(query, list(texts[start : start + batch])))
        return scores

    def fingerprint(self) -> dict:
        """The complete identity of this scoring configuration."""
        if self._model is None:
            self.load()
        adapter = self._details["adapter"] or {}
        return {
            "backend": pins.BACKEND_NAME,
            "backend_version": pins.BACKEND_VERSION,
            "protocol_version": pins.PROTOCOL_VERSION,
            "scoring": pins.SCORING_CROSS_ENCODER,
            "scoring_is_neural": True,
            "note": pins.CROSS_ENCODER_NOTE,
            "model_label": self._model_label,
            "model_id": self._config["model_id"],
            "model_revision": self._config["model_revision"],
            "tokenizer_id": self._config["tokenizer_id"],
            "tokenizer_revision": self._config["tokenizer_revision"],
            "architectures": self._details["architectures"],
            "num_labels": self._details["num_labels"],
            "score_column": int(self._config["score_column"]),
            "score_direction": pins.SCORE_DIRECTION,
            "dtype": self._config["dtype"],
            "device": self._details["device"],
            "max_length": int(self._config["max_length"]),
            "truncation": self._config["truncation"],
            "padding": self._config["padding"],
            "batch_size": int(self._config["batch_size"]),
            "weights_sha256": self._details.get("weights_sha256"),
            "head_parameters": self._details["head_parameters"],
            "adapter_label": self._adapter_label,
            "adapter_path": self._config.get("adapter"),
            "adapter_base_model": adapter.get("base_model_name_or_path"),
            "adapter_task_type": adapter.get("task_type"),
            "adapter_peft_type": adapter.get("peft_type"),
            "adapter_modules_to_save": adapter.get("modules_to_save"),
            "adapter_config_sha256": adapter.get("config_sha256"),
            "adapter_weights_sha256": adapter.get("weights_sha256"),
            "lexical_version": None,
            "libraries": self._details["libraries"],
        }

    def _score_batch(self, query: str, window: list) -> list:
        """One forward pass over one batch, under `inference_mode`."""
        torch = self._torch
        encoded = self._tokenizer(
            [query] * len(window),
            window,
            padding=self._config["padding"],
            truncation=self._config["truncation"],
            max_length=int(self._config["max_length"]),
            return_tensors="pt",
        )
        encoded = {key: value.to(self._model.device) for key, value in encoded.items()}
        with torch.inference_mode():
            logits = self._model(**encoded).logits
        column = int(self._config["score_column"])
        if column >= logits.shape[-1]:
            raise pins.RerankError(
                "--score-column " + str(column) + " is out of range for a head with "
                + str(logits.shape[-1]) + " label(s)",
                pins.EX_DATAERR,
            )
        return [float(v) for v in logits[:, column].to(torch.float32).cpu().tolist()]

    def _record(self, model, head_parameters: list, adapter_facts: dict) -> None:
        """Capture everything the fingerprint reports about the loaded model."""
        self._details = {
            "device": str(model.device),
            "head_parameters": head_parameters,
            "num_labels": int(model.config.num_labels),
            "architectures": list(getattr(model.config, "architectures", []) or []),
            "adapter": adapter_facts,
            "libraries": compat.library_versions(bool(adapter_facts)),
        }
        if self._config["digest_base_weights"]:
            self._details["weights_sha256"] = compat.sha256(
                compat.base_weights_path(self._config)
            )

    def _attach_adapter(self, model, head_parameters: list):
        """Apply the validated adapter, reporting a head that survived unchanged."""
        peft_model = compat.import_peft()
        wanted = set(head_parameters)
        before = {
            name: value.detach().clone().cpu()
            for name, value in model.named_parameters()
            if name in wanted
        }
        adapted = compat.call_hub(
            peft_model.from_pretrained,
            model,
            self._config["adapter"],
            local_files_only=not self._config["allow_download"],
        )
        self._warn_if_head_unchanged(adapted, before)
        return adapted

    def _warn_if_head_unchanged(self, adapted, before: dict) -> None:
        """Warn when the adapter's saved head is bit-identical to the base one.

        A frozen head is a legitimate recipe, so this is a warning and not a
        refusal — but it is worth saying out loud, because the other way to
        reach this state is an adapter that saved a head and failed to apply it.

        PEFT renames a `modules_to_save` parameter, so `classifier.weight`
        becomes `base_model.model.classifier.modules_to_save.default.weight`
        alongside an `...original_module.weight` copy of the untouched base
        value. Matching on the new name's own shape is therefore load-bearing:
        comparing base names directly would find nothing and warn on every
        adapter, and warnings nobody can act on are warnings everybody ignores.
        """
        if not before:
            return
        torch = self._torch
        current = dict(adapted.named_parameters())
        compared = 0
        for name, original in before.items():
            module, _, rest = name.partition(".")
            marker = "." + module + ".modules_to_save."
            for candidate, value in current.items():
                if marker not in candidate or not candidate.endswith("." + rest):
                    continue
                compared += 1
                if not torch.equal(value.detach().cpu(), original):
                    return
        if compared == 0:
            compat.diag(
                "warning: the adapted model exposes no saved relevance-head parameter, "
                "so the head could not be compared against the base model"
            )
            return
        compat.diag("warning: every relevance-head parameter is identical to the base model")
