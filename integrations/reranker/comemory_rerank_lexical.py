"""The deterministic, non-neural scoring mode.

This is not a model and must never be read as a quality baseline. It exists so
that the real subprocess, the real JSON on real pipes, the real identity gate
and the real Rust `RerankRunner` can all be exercised on every
`cargo nextest run`, with no download, no torch and no network — which is the
only way protocol conformance stays continuously proven rather than
occasionally checked by hand.

Two properties make it safe to ship beside a real scorer. Its model label is
`lexical-overlap@1`, which no request expecting a real model can carry, so a
process started in this mode physically cannot answer for the cross-encoder.
And its fingerprint says `scoring_is_neural: false` with a note that spells the
limitation out, so the distinction survives into any log or report that records
one.

The function itself is pinned so an expected order can be computed by hand:
Jaccard similarity over the sets of lowercased alphanumeric tokens in the query
and the candidate text, with an empty union scoring `0.0`.
"""

from __future__ import annotations

import platform
import re
from typing import Sequence

import comemory_rerank_pins as pins

# Deliberately ASCII-only and deliberately simple. A cleverer tokenizer would
# make the expected scores in the conformance suite something a reader has to
# trust rather than something a reader can recompute.
_TOKEN_RE = re.compile(r"[a-z0-9]+")


class LexicalScorer:
    """Score query/candidate pairs by token-set overlap, deterministically."""

    def __init__(self, model_label: str | None = None) -> None:
        self._model_label = model_label or pins.LEXICAL_MODEL_LABEL

    @property
    def model_label(self) -> str:
        """The identity this process answers to."""
        return self._model_label

    @property
    def adapter_label(self) -> str | None:
        """Always `None`: there is no model here, so there is no adapter."""
        return None

    def load(self) -> int:
        """Prepare for scoring, and report the cost in milliseconds.

        There is nothing to load, so the cost is zero. The method exists because
        the entry point, the warm server and the benchmark all drive a scorer
        through one interface, and a mode-specific branch at each of those three
        call sites is how the two modes would drift apart.
        """
        return 0

    def score(self, query: str, texts: Sequence[str]) -> list:
        """Score every candidate text against `query`, in submitted order."""
        query_tokens = _tokens(query)
        return [_jaccard(query_tokens, _tokens(text)) for text in texts]

    def score_pair(self, query: str, text: str) -> float:
        """Score one pair, for a caller checking the pinned definition itself."""
        return _jaccard(_tokens(query), _tokens(text))

    def fingerprint(self) -> dict:
        """The complete identity of this scoring configuration.

        Shaped like the cross-encoder's so a consumer reads one schema, with
        every model-specific field explicitly `null` rather than absent: an
        absent key reads as "unknown", and this mode's model configuration is
        not unknown, it does not exist.
        """
        return {
            "backend": pins.BACKEND_NAME,
            "backend_version": pins.BACKEND_VERSION,
            "protocol_version": pins.PROTOCOL_VERSION,
            "scoring": pins.SCORING_LEXICAL,
            "scoring_is_neural": False,
            "note": pins.LEXICAL_NOTE,
            "model_label": self._model_label,
            "model_id": None,
            "model_revision": None,
            "tokenizer_id": None,
            "tokenizer_revision": None,
            "architectures": None,
            "num_labels": None,
            "score_column": None,
            "score_direction": pins.SCORE_DIRECTION,
            "dtype": None,
            "device": None,
            "max_length": None,
            "truncation": None,
            "padding": None,
            "batch_size": None,
            "weights_sha256": None,
            "head_parameters": None,
            "adapter_label": None,
            "adapter_path": None,
            "adapter_base_model": None,
            "adapter_task_type": None,
            "adapter_peft_type": None,
            "adapter_modules_to_save": None,
            "adapter_config_sha256": None,
            "adapter_weights_sha256": None,
            "lexical_version": pins.LEXICAL_VERSION,
            "libraries": {"python": platform.python_version()},
        }


def _tokens(text: str) -> set:
    """The lowercased alphanumeric token set of one string."""
    return set(_TOKEN_RE.findall(text.lower()))


def _jaccard(left: set, right: set) -> float:
    """Jaccard similarity, with an empty union scoring `0.0` rather than dividing."""
    union = left | right
    if not union:
        return 0.0
    return len(left & right) / len(union)
