"""Validate one scorer response, and spell every way it can be refused.

Split from the scoring pass along the seam that matters: that file decides which
requests to send and what the run cost, and this one decides whether an answer
may be believed at all. Every refusal here is a recorded FALLBACK rather than an
abort — it is exactly the ladder production walks when `[rerank]` is enabled and
a scorer misbehaves, so the qualification measures the same failure the operator
would see, and counts it.

Nothing partial is ever accepted: a response missing one score, repeating one id
or carrying one non-finite value contributes no scores for its task at all,
which shows up as a lower `scored_fraction` rather than as a silently shorter
arm.
"""

from __future__ import annotations

import json
import math

import comemory_train_pins as pins

STDERR_EXCERPT_BYTES = 2048

# The response object's whole key set, mirroring the backend's own
# `deny_unknown_fields` in both directions.
RESPONSE_KEYS = (
    "protocol_version",
    "request_id",
    "model",
    "adapter",
    "score_direction",
    "scores",
)

def validate(request: dict, stdout: bytes, stderr: str) -> dict:
    """Refuse every response shape that would make this arm's scores a lie."""
    try:
        payload = json.loads(stdout.decode("utf-8"))
    except (UnicodeDecodeError, ValueError) as exc:
        return failure("invalid_response", None, "unparsable stdout: " + str(exc), stderr)
    if not isinstance(payload, dict):
        return failure("invalid_response", None, "response is not an object", stderr)
    # Exactly the six documented keys, in both directions. The protocol says a
    # response carries these and no others, so an omitted `adapter` must not be
    # allowed to read as a matching `null` and an invented key must not pass
    # unnoticed — an additive field is a version bump, not a silent extension.
    found, wanted = set(payload), set(RESPONSE_KEYS)
    if found != wanted:
        return failure(
            "invalid_response",
            None,
            "response keys " + ", ".join(sorted(found)) + "; expected "
            + ", ".join(sorted(wanted)),
            stderr,
        )
    for key in ("request_id", "model", "adapter"):
        if payload.get(key) != request[key]:
            return failure(
                "identity_mismatch",
                None,
                key + " echoed " + repr(payload.get(key)) + ", expected " + repr(request[key]),
                stderr,
            )
    if payload.get("protocol_version") != pins.PROTOCOL_VERSION:
        return failure(
            "invalid_response", None,
            "protocol_version " + repr(payload.get("protocol_version")), stderr,
        )
    if payload.get("score_direction") != pins.SCORE_DIRECTION:
        return failure(
            "invalid_response", None,
            "score_direction " + repr(payload.get("score_direction")), stderr,
        )
    return scores(request, payload, stderr)


def scores(request: dict, payload: dict, stderr: str) -> dict:
    """The per-candidate scores, or a refusal naming the exact divergence."""
    wanted = {candidate["id"] for candidate in request["candidates"]}
    found: dict = {}
    for row in payload.get("scores") or []:
        if not isinstance(row, dict):
            return failure("invalid_response", None, "a score row is not an object", stderr)
        identity = row.get("id")
        value = row.get("score")
        if identity not in wanted:
            return failure("invalid_response", None, "unknown id " + repr(identity), stderr)
        if identity in found:
            return failure("invalid_response", None, "repeated id " + repr(identity), stderr)
        if not isinstance(value, (int, float)) or isinstance(value, bool):
            return failure("invalid_response", None, "score for " + repr(identity), stderr)
        number = float(value)
        if not math.isfinite(number):
            return failure("invalid_response", None, "non-finite score", stderr)
        found[identity] = number
    missing = sorted(wanted - set(found))
    if missing:
        return failure(
            "invalid_response", None, "no score for " + ", ".join(missing[:5]), stderr
        )
    return {"scores": found}


def failure(kind: str, code, detail: str, stderr: str = "") -> dict:
    """One recorded failure, which is a fallback rather than an abort."""
    return {
        "failure": {
            "kind": kind,
            "code": code,
            "detail": detail,
            "stderr_excerpt": stderr,
        }
    }

def excerpt(stream: bytes) -> str:
    """A bounded, diagnostic-only excerpt of a child's stderr."""
    return stream[:STDERR_EXCERPT_BYTES].decode("utf-8", "replace").strip()

