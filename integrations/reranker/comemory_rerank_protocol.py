"""Decode one reranker request, gate its identity, and encode its response.

Pure data handling with no process I/O and no model, so every rejection rule
can be exercised from a hand-built payload. The rules mirror the Rust side's
`utilities::rerank_protocol` exactly, including `deny_unknown_fields` in both
directions: a caller that invented a field learns immediately rather than
being silently ignored, and the response carries exactly the six documented
keys because an additive one would be a protocol version bump.

The identity gate is the reason this module exists as a separate concern. The
response must echo `model` and `adapter` byte for byte, so a backend that
echoed whatever it was handed would let a caller asking for one adapter
receive another adapter's scores under the requested name, with nothing on the
wire able to detect it. Instead a process answers only to the identity it was
configured with, and refuses everything else.
"""

from __future__ import annotations

import json
import math
import re
from dataclasses import dataclass
from typing import Iterable, Sequence

import comemory_rerank_pins as pins

_REQUEST_ID_RE = re.compile(pins.REQUEST_ID_PATTERN)


@dataclass(frozen=True)
class Candidate:
    """One candidate offered for scoring, in the caller's deterministic order."""

    id: str
    rank: int
    text: str


@dataclass(frozen=True)
class Request:
    """One decoded, fully validated scoring request."""

    protocol_version: int
    request_id: str
    model: str
    adapter: str | None
    query: str
    candidates: tuple


def decode(raw: bytes) -> Request:
    """Parse and validate one request body, or raise `RerankError`.

    Checks run in the order the design document lists them, so the diagnostic
    names the first real divergence rather than a downstream symptom of it.
    """
    if not raw.strip():
        raise pins.RerankError("empty stdin; expected exactly one JSON request object")
    try:
        payload = json.loads(raw.decode("utf-8"))
    except UnicodeDecodeError as exc:
        raise pins.RerankError("request is not valid UTF-8: " + str(exc)) from exc
    except ValueError as exc:
        raise pins.RerankError("request is not valid JSON: " + str(exc)) from exc
    if not isinstance(payload, dict):
        raise pins.RerankError(
            "request must be a JSON object, found " + type(payload).__name__
        )
    _exact_keys(payload, pins.REQUEST_KEYS, "request")
    _check_version(payload["protocol_version"])
    request_id = _require_str(payload, "request_id")
    if not _REQUEST_ID_RE.match(request_id):
        raise pins.RerankError(
            "request_id " + repr(request_id) + " is not rr-<yyyymmdd>-<8 lowercase hex>"
        )
    model = _require_str(payload, "model")
    if not model:
        raise pins.RerankError("model must be a non-empty string")
    adapter = payload["adapter"]
    if adapter is not None and not isinstance(adapter, str):
        raise pins.RerankError("adapter must be a string or null")
    return Request(
        protocol_version=pins.PROTOCOL_VERSION,
        request_id=request_id,
        model=model,
        adapter=adapter,
        query=_require_str(payload, "query"),
        candidates=_decode_candidates(payload["candidates"]),
    )


def check_identity(request: Request, model: str, adapter: str | None) -> None:
    """Refuse a request this process is not configured to answer.

    Fails closed, and observably: the Rust runner sees `NonZeroExit`, declines,
    and the caller's deterministic ranking stands unchanged.
    """
    if request.model != model:
        raise pins.RerankError(
            "request model "
            + repr(request.model)
            + " does not match this backend's model "
            + repr(model)
        )
    if request.adapter != adapter:
        raise pins.RerankError(
            "request adapter "
            + repr(request.adapter)
            + " does not match this backend's adapter "
            + repr(adapter)
        )


def encode(request: Request, scores: Sequence[float]) -> str:
    """Serialize the response for `request`, one finite score per candidate.

    Scores arrive in submitted order and are emitted in submitted order. The
    protocol calls array order irrelevant; emitting the submitted order anyway
    makes two runs over one request byte-identical, which is what lets a test
    assert on the payload rather than on a parse of it.
    """
    if len(scores) != len(request.candidates):
        raise pins.RerankError(
            "scorer returned "
            + str(len(scores))
            + " scores for "
            + str(len(request.candidates))
            + " candidates",
            pins.EX_SOFTWARE,
        )
    rows = []
    for candidate, score in zip(request.candidates, scores):
        value = float(score)
        if not math.isfinite(value):
            raise pins.RerankError(
                "non-finite score for candidate " + repr(candidate.id),
                pins.EX_SOFTWARE,
            )
        rows.append({"id": candidate.id, "score": value})
    body = {
        "protocol_version": pins.PROTOCOL_VERSION,
        "request_id": request.request_id,
        "model": request.model,
        "adapter": request.adapter,
        "score_direction": pins.SCORE_DIRECTION,
        "scores": rows,
    }
    # `allow_nan=False` is belt and braces over the finiteness check above:
    # Python's json writes bare `Infinity` and `NaN`, which are not JSON and
    # which the Rust side would report as a parse failure rather than as the
    # scorer bug they are.
    return json.dumps(body, separators=(",", ":"), ensure_ascii=False, allow_nan=False)


def _check_version(value: object) -> None:
    """Refuse a protocol version this build does not implement.

    The wire contract prescribes exactly this: a scorer that receives a version
    it does not implement exits non-zero with a diagnostic rather than
    answering.
    """
    if isinstance(value, bool) or not isinstance(value, int):
        raise pins.RerankError("protocol_version must be an integer")
    if value != pins.PROTOCOL_VERSION:
        raise pins.RerankError(
            "protocol_version "
            + str(value)
            + " is not supported; this backend speaks "
            + str(pins.PROTOCOL_VERSION)
        )


def _decode_candidates(value: object) -> tuple:
    """Validate the candidate list and every candidate object in it."""
    if not isinstance(value, list):
        raise pins.RerankError("candidates must be a JSON array")
    if not value:
        raise pins.RerankError("candidates must not be empty")
    seen: set = set()
    decoded = []
    for index, raw in enumerate(value):
        if not isinstance(raw, dict):
            raise pins.RerankError("candidate at index " + str(index) + " is not an object")
        _exact_keys(raw, pins.CANDIDATE_KEYS, "candidate at index " + str(index))
        identity = _require_str(raw, "id")
        if not identity:
            raise pins.RerankError("candidate at index " + str(index) + " has an empty id")
        if identity in seen:
            raise pins.RerankError("duplicate candidate id " + repr(identity))
        seen.add(identity)
        rank = raw["rank"]
        if isinstance(rank, bool) or not isinstance(rank, int) or rank < 0:
            raise pins.RerankError(
                "candidate " + repr(identity) + " has a rank that is not a non-negative integer"
            )
        decoded.append(Candidate(id=identity, rank=rank, text=_require_str(raw, "text")))
    return tuple(decoded)


def _exact_keys(payload: dict, expected: Iterable[str], what: str) -> None:
    """Enforce `deny_unknown_fields` in both directions, naming what differs."""
    wanted = set(expected)
    found = set(payload)
    unknown = sorted(found - wanted)
    if unknown:
        raise pins.RerankError(what + " has unknown key(s): " + ", ".join(unknown))
    missing = sorted(wanted - found)
    if missing:
        raise pins.RerankError(what + " is missing key(s): " + ", ".join(missing))


def _require_str(payload: dict, key: str) -> str:
    """Read a key that the contract declares is a string."""
    value = payload[key]
    if not isinstance(value, str):
        raise pins.RerankError(key + " must be a string, found " + type(value).__name__)
    return value
