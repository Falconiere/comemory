"""Decide whether the opt-in model suite may run, and say exactly why not.

Driven by `run-model-tests.sh`. It lives in Python rather than in that script
because the versions it enforces are declared once, in
`comemory_rerank_pins.REQUIREMENTS`, and re-typing them in shell would create a
second place for them to drift from `requirements.txt`.

Every problem is collected and printed rather than the first one aborting, so
one run tells an operator everything they have to install rather than making
them discover it one package at a time.
"""

from __future__ import annotations

import importlib
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))

import comemory_rerank_pins as pins


def main() -> int:
    """Report every unmet prerequisite. Returns 0 only when there are none."""
    problems = _library_problems()
    problems.extend(_snapshot_problems())
    if not problems:
        return pins.EX_OK
    for problem in problems:
        _say(problem)
    _say("fix the library versions with: " + pins.INSTALL_HINT)
    _say("fetch the pinned weights with: " + pins.DOWNLOAD_HINT)
    return pins.EX_UNAVAILABLE


def _library_problems() -> list:
    """Every pinned library that is absent, or present at the wrong version."""
    problems = []
    for name, wanted in sorted(pins.REQUIREMENTS.items()):
        try:
            module = importlib.import_module(name)
        except ImportError as exc:
            problems.append("missing library " + name + "==" + wanted + " (" + str(exc) + ")")
            continue
        found = getattr(module, "__version__", None)
        if found != wanted:
            problems.append(
                "library " + name + " is " + repr(found) + ", but this configuration is "
                "pinned to " + wanted + "; a result from another build is not the result "
                "this configuration produces"
            )
    return problems


def _snapshot_problems() -> list:
    """Whether the pinned model revision is already in the local cache."""
    try:
        from huggingface_hub import try_to_load_from_cache
    except ImportError:
        # Already reported as a missing library; saying it twice helps nobody.
        return []
    missing = []
    for filename in ("config.json", "tokenizer.json", pins.BASE_WEIGHT_FILES[0]):
        cached = try_to_load_from_cache(
            pins.MODEL_ID, filename, revision=pins.MODEL_REVISION
        )
        if not isinstance(cached, str):
            missing.append(filename)
    if not missing:
        return []
    return [
        "the pinned snapshot of " + pins.MODEL_ID + " at revision " + pins.MODEL_REVISION
        + " is not in the local cache (missing " + ", ".join(missing) + ")"
    ]


def _say(message: str) -> None:
    """Write one diagnostic line to stderr."""
    sys.stderr.write(pins.DIAG_PREFIX + message + "\n")


if __name__ == "__main__":
    sys.exit(main())
