"""Turn one captured benchmark artifact into one arm's scores, and measure it.

The artifact `comemory benchmark --report` writes already holds every task's
candidate pool, in `pool_position` order, with the bounded passage text. This
module walks it, asks a scorer for one relevance value per candidate over the
#211 wire protocol, and writes the scores file `comemory benchmark --scores`
reads back. Every arm therefore reorders the same captured pool: the comparison
is paired by construction, and there is no code path here that could add a
candidate retrieval never returned.

**The wire id is the pool position, not the candidate reference.** #213
established that a candidate reference is not unique within a pool — its code
form is `(repo, path, symbol, blob_oid)` while `code_symbols` is unique on
`(repo, path, symbol, line_start)` — and #211 refuses a request carrying a
repeated candidate id, which would decline the whole task. Pool position is
unique by construction, and the scores are mapped back through it.

**Operational cost is measured here because the binary cannot see it.** The
artifact's own `latency_ms` is retrieval's wall clock and is documented as
excluding an arm's inference. Scoring latency, peak child memory and the
fallback rate belong to the child process, so they go into a sidecar beside the
scores rather than being confused with a 7 ms retrieval.
"""

from __future__ import annotations

import datetime
import hashlib
import json
import os
import resource
import subprocess
import sys
import time

import comemory_qualify_wire as wire
import comemory_train_pins as pins

SIDECAR_VERSION = 1


def score(report_path: str, options: dict) -> tuple:
    """Return `(scores, sidecar)` for one arm over one captured artifact."""
    report = _read_report(report_path)
    tasks = report.get("tasks") or []
    if not tasks:
        raise pins.RecipeError(
            report_path + " holds no task, so a scores file over it would compare nothing"
        )
    state = {
        "scores": {},
        "failures": [],
        "latencies": [],
        "submitted": 0,
        "scored": 0,
        "ambiguous": 0,
        "empty_pools": 0,
        "invocations": 0,
    }
    for task in tasks:
        _one_task(task, options, state)
    return _documents(report, report_path, options, state)


def _one_task(task: dict, options: dict, state: dict) -> None:
    """Score one task's pool, recording either its scores or its failure."""
    task_id = str(task.get("task_id"))
    observation = task.get("observation") or {}
    candidates = sorted(
        observation.get("candidates") or [], key=lambda c: int(c.get("pool_position", 0))
    )
    if not candidates:
        state["empty_pools"] += 1
        return
    request = _request(task_id, str(observation.get("query", "")), candidates, options)
    state["submitted"] += len(candidates)
    state["invocations"] += 1
    started = time.perf_counter()
    outcome = _invoke(options, request)
    state["latencies"].append((time.perf_counter() - started) * 1000.0)
    if "failure" in outcome:
        failure = dict(outcome["failure"])
        failure["task_id"] = task_id
        state["failures"].append(failure)
        return
    _record(task_id, candidates, outcome["scores"], state)


def _request(task_id: str, query: str, candidates: list, options: dict) -> dict:
    """One #211 request over this task's pool, in the order retrieval produced."""
    return {
        "protocol_version": pins.PROTOCOL_VERSION,
        "request_id": _request_id(options["arm"], task_id),
        "model": options["model"],
        "adapter": options["adapter"],
        "query": query,
        "candidates": [
            {
                "id": str(candidate.get("pool_position")),
                "rank": int(candidate.get("pool_position", 0)),
                "text": str((candidate.get("text") or {}).get("text", "")),
            }
            for candidate in candidates
        ],
    }


def _request_id(arm: str, task_id: str) -> str:
    """`rr-<yyyymmdd>-<8 lowercase hex>`, the shape #211 mints and validates.

    The hex half is derived from the arm and the task rather than drawn at
    random, so two runs of one arm over one artifact send the same identifiers
    and a captured transcript is comparable.
    """
    day = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%d")
    digest = hashlib.sha256((arm + "\x00" + task_id).encode("utf-8")).hexdigest()
    return "rr-" + day + "-" + digest[:8]


def _invoke(options: dict, request: dict) -> dict:
    """Run the scorer as a child under one deadline and validate its answer."""
    if not isinstance(options["command"], list):
        # The entry point always builds a list, and a list is what keeps the
        # query and the passage text off any shell command line. Checking it
        # here anyway costs nothing and makes the guarantee local to the call
        # that depends on it, rather than an invariant a future caller has to
        # know about.
        raise pins.RecipeError(
            "the scorer command must be an argument vector, not "
            + type(options["command"]).__name__,
            pins.EX_USAGE,
        )
    body = json.dumps(request, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    if len(body) > pins.MAX_REQUEST_BYTES:
        return wire.failure("request_too_large", None, str(len(body)) + " bytes")
    try:
        done = subprocess.run(
            options["command"],
            input=body,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=options["timeout_ms"] / 1000.0,
            check=False,
        )
    except OSError as exc:
        # A missing program, a directory, a file without the execute bit: every
        # one of them is a scorer that could not start, which is the same
        # fallback in production as one that started and failed.
        return wire.failure("spawn_failed", None, str(exc))
    except subprocess.TimeoutExpired:
        return wire.failure("timed_out", None, "exceeded " + str(options["timeout_ms"]) + "ms")
    if done.returncode != 0:
        return wire.failure("non_zero_exit", done.returncode, wire.excerpt(done.stderr))
    return wire.validate(request, done.stdout, wire.excerpt(done.stderr))


def _record(task_id: str, candidates: list, scores: dict, state: dict) -> None:
    """Map wire ids back onto candidate references, counting every ambiguity."""
    table: dict = {}
    for candidate in candidates:
        reference = str(candidate.get("candidate_ref"))
        value = scores.get(str(candidate.get("pool_position")))
        if value is None:
            continue
        if reference in table:
            state["ambiguous"] += 1
            continue
        table[reference] = value
        state["scored"] += 1
    state["scores"][task_id] = table


def _documents(report: dict, report_path: str, options: dict, state: dict) -> tuple:
    """The scores file `comemory benchmark` reads, and the operational sidecar."""
    scores = {
        "arm": options["arm"],
        "scorer_version": _scorer_version(options),
        "scores": state["scores"],
    }
    retrieval = report.get("retrieval") or {}
    invocations = state["invocations"]
    sidecar = {
        "sidecar_version": SIDECAR_VERSION,
        "arm": options["arm"],
        "model": options["model"],
        "adapter": options["adapter"],
        "backend": list(options["command"]),
        "report": os.path.basename(report_path),
        "report_sha256": _digest(report_path),
        "set_name": report.get("set_name"),
        "corpus_digest": (retrieval.get("corpus") or {}).get("digest"),
        "knobs_hash": retrieval.get("knobs_hash"),
        "tasks": len(report.get("tasks") or []),
        "invocations": invocations,
        "failures": len(state["failures"]),
        "fallback_rate": (len(state["failures"]) / invocations) if invocations else 0.0,
        "candidates_submitted": state["submitted"],
        "candidates_scored": state["scored"],
        "ambiguous_refs": state["ambiguous"],
        "tasks_without_candidates": state["empty_pools"],
        "failure_reasons": state["failures"],
        "latency_ms": _latency(state["latencies"]),
        "peak_child_rss": _peak_child_rss(state["invocations"]),
        "timeout_ms": options["timeout_ms"],
    }
    return scores, sidecar


def _scorer_version(options: dict) -> str:
    """The identity recorded in the artifact beside this arm's numbers."""
    if options["adapter"]:
        return options["model"] + " (" + options["adapter"] + ")"
    return options["model"]


def _latency(samples: list) -> dict:
    """The child's own wall clock, at the percentile rule the report uses."""
    if not samples:
        return {"p50": 0.0, "p90": 0.0, "p95": 0.0, "max": 0.0, "mean": 0.0}
    ordered = sorted(samples)
    last = len(ordered) - 1

    def at(percent: float) -> float:
        index = int(round((percent / 100.0) * last))
        return round(ordered[min(index, last)], 3)

    return {
        "p50": at(50.0),
        "p90": at(90.0),
        "p95": at(95.0),
        "max": round(ordered[-1], 3),
        "mean": round(sum(ordered) / len(ordered), 3),
    }


def _peak_child_rss(invocations: int) -> dict:
    """Peak resident memory over every child, with its unit named.

    `getrusage` reports this field in bytes on macOS and in kilobytes on Linux.
    Recording the platform beside the number is the difference between a
    measurement and a number.

    `RUSAGE_CHILDREN` only accumulates children that have been waited on, so a
    run where every spawn failed would report a peak of zero. That is an absent
    measurement, not a measured zero, and it is spelled `null`.
    """
    return {
        "value": (
            resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss if invocations else None
        ),
        "unit": "bytes" if sys.platform == "darwin" else "kilobytes",
        "platform": sys.platform,
    }


def _read_report(path: str) -> dict:
    """Read a benchmark artifact and gate the contract versions it declares."""
    if not os.path.isfile(path):
        raise pins.RecipeError(
            "no benchmark artifact at " + path
            + "; produce one with `comemory benchmark --set <set> --report <file>`",
            pins.EX_DATAERR,
        )
    with open(path, "r", encoding="utf-8") as handle:
        try:
            report = json.load(handle)
        except ValueError as exc:
            raise pins.RecipeError(path + " is not valid JSON: " + str(exc)) from exc
    if report.get("artifact_version") != pins.SUPPORTED_ARTIFACT_VERSION:
        raise pins.RecipeError(
            path + " declares artifact_version " + repr(report.get("artifact_version"))
            + ", but this harness reads " + str(pins.SUPPORTED_ARTIFACT_VERSION)
        )
    if report.get("observation_version") != pins.SUPPORTED_OBSERVATION_VERSION:
        raise pins.RecipeError(
            path + " declares observation_version "
            + repr(report.get("observation_version")) + ", but this harness reads "
            + str(pins.SUPPORTED_OBSERVATION_VERSION)
        )
    return report


def _digest(path: str) -> str:
    """Hex SHA-256 of a file, read in bounded chunks."""
    out = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            out.update(chunk)
    return out.hexdigest()
