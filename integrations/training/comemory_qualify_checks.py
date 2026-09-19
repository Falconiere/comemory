"""The integrity checks and the operational budgets a qualification is read against.

Split from the decision itself along the seam that matters: this file decides
what is *true* about a run, and `comemory_qualify_decide` decides what that
means. Every function here returns a row carrying the values it compared, not a
bare boolean, because a record that says only `fail` sends a reader back to the
inputs to work out what differed.

A check that cannot run returns `skipped`, never `pass`.
"""

from __future__ import annotations

import json
import os

import comemory_train_manifest as adapter_manifest
import comemory_train_pins as pins

SCORED_FRACTION_TOLERANCE = 1e-9
RANKING_KEYS = ("rrf_k", "decay", "mmr_lambda", "bm25_weights", "graph_hops", "graph_seeds")


def checks(report: dict, provenance: dict, sidecars: dict, arms: dict, options: dict) -> list:
    """Every integrity check, each with the values it actually compared."""
    rows = [_versions(report)]
    rows.extend(_sidecar_checks(report, sidecars))
    rows.append(_ranking_check(report, provenance))
    rows.append(_stale_check(report))
    rows.extend(_arm_checks(arms, sidecars, options))
    rows.append(_neural_check(sidecars))
    rows.extend(_adapter_checks(options, provenance))
    return rows


def _versions(report: dict) -> dict:
    """The artifact speaks a contract version this harness reads."""
    ok = (
        report.get("artifact_version") == pins.SUPPORTED_ARTIFACT_VERSION
        and report.get("observation_version") == pins.SUPPORTED_OBSERVATION_VERSION
    )
    return _check(
        "artifact_versions",
        ok,
        "artifact_version " + str(report.get("artifact_version")) + ", observation_version "
        + str(report.get("observation_version")),
    )


def _sidecar_checks(report: dict, sidecars: dict) -> list:
    """Every arm scored the corpus and configuration the verdict came from."""
    retrieval = report.get("retrieval") or {}
    corpus = (retrieval.get("corpus") or {}).get("digest")
    knobs = retrieval.get("knobs_hash")
    rows = []
    for name, car in sorted(sidecars.items()):
        ok = car.get("corpus_digest") == corpus and car.get("knobs_hash") == knobs
        rows.append(
            _check(
                "scored_the_reported_run[" + name + "]",
                ok,
                "sidecar corpus " + str(car.get("corpus_digest"))[:12] + " knobs "
                + str(car.get("knobs_hash"))[:12] + "; artifact corpus " + str(corpus)[:12]
                + " knobs " + str(knobs)[:12],
            )
        )
    return rows


def _ranking_check(report: dict, provenance: dict) -> dict:
    """The run applied the ranking the generated set pinned."""
    declared = provenance.get("set_ranking") or {}
    applied = (report.get("retrieval") or {}).get("knobs") or {}
    differences = [
        key
        for key in RANKING_KEYS
        if _number(declared.get(key)) != _number(applied.get(key))
    ]
    return _check(
        "pinned_ranking_applied",
        not differences,
        "every pinned knob matches" if not differences else "differs on " + ", ".join(differences),
    )


def _stale_check(report: dict) -> dict:
    """No reviewed judgment describes a content version this run did not see."""
    stale = sum(int(task.get("judgments_stale", 0)) for task in report.get("tasks") or [])
    return _check("no_stale_judgments", stale == 0, str(stale) + " stale judgment(s)")


def _arm_checks(arms: dict, sidecars: dict, options: dict) -> list:
    """Each compared arm scored the whole pool, unambiguously."""
    rows = []
    for name in (options["base_arm"], options["lora_arm"]):
        arm = arms.get(name)
        if arm is None:
            rows.append(_check("arm_present[" + name + "]", False, "no arm named " + name))
            continue
        fraction = float(arm.get("scored_fraction", 0.0))
        rows.append(
            _check(
                "scored_the_whole_pool[" + name + "]",
                fraction >= 1.0 - SCORED_FRACTION_TOLERANCE,
                "scored_fraction " + format(fraction, ".6f"),
            )
        )
    for name, car in sorted(sidecars.items()):
        ambiguous = int(car.get("ambiguous_refs", 0))
        rows.append(
            _check(
                "unambiguous_references[" + name + "]",
                ambiguous == 0,
                str(ambiguous) + " candidate(s) shared a reference within one pool",
            )
        )
    return rows


def _neural_check(sidecars: dict) -> dict:
    """No compared arm was scored by a deterministic non-neural stand-in."""
    offenders = [
        name
        for name, car in sorted(sidecars.items())
        if str(car.get("model")) in pins.NON_NEURAL_MODEL_LABELS
    ]
    return _check(
        "scorers_are_neural",
        not offenders,
        "every arm used a model"
        if not offenders
        else "arm(s) " + ", ".join(offenders) + " used a non-neural scorer, which measures "
        "no relevance quality",
    )


def _adapter_checks(options: dict, provenance: dict) -> list:
    """The adapter was qualified against the holdout withheld from its training."""
    document = adapter(options)
    if document is None:
        return [
            _check(
                "adapter_provenance",
                None,
                "no --adapter-manifest was supplied, so the adapter's own record of which "
                "holdout it withheld could not be compared with the one scored here",
            )
        ]
    recomputed = adapter_manifest.adapter_id(document)
    holdout = (document.get("dataset") or {}).get("holdout") or {}
    declared = provenance.get("holdout") or {}
    selection = document.get("selection") or {}
    return [
        _check(
            "adapter_id_matches",
            recomputed == document.get("adapter_id"),
            "recorded " + str(document.get("adapter_id")) + ", recomputed " + recomputed,
        ),
        _check(
            "qualified_against_its_own_holdout",
            bool(holdout.get("sha256")) and holdout.get("sha256") == declared.get("sha256"),
            "adapter withheld " + str(holdout.get("sha256"))[:12] + ", this set was built "
            "from " + str(declared.get("sha256"))[:12],
        ),
        _check(
            "holdout_not_used_in_selection",
            selection.get("holdout_used") is False and holdout.get("read") is False,
            "holdout_used " + str(selection.get("holdout_used")) + ", holdout read "
            + str(holdout.get("read")),
        ),
    ]


def budgets(report: dict, lora: dict | None, sidecar: dict | None) -> list:
    """The quality verdict, and the two operational budgets this harness owns."""
    verdict = (lora or {}).get("verdict")
    rows = [
        _check("quality_verdict", verdict == "improved", "benchmark verdict " + str(verdict)),
        _check(
            "retrieval_latency_within_budget",
            bool((lora or {}).get("latency_within_budget")),
            "p95 " + str(((lora or {}).get("latency_ms") or {}).get("p95")) + "ms against "
            + str((report.get("budgets") or {}).get("max_p95_task_ms")) + "ms",
        ),
    ]
    if sidecar is None:
        rows.append(_check("fallback_rate", None, "no scoring sidecar for the adapter arm"))
        rows.append(_check("scoring_latency", None, "no scoring sidecar for the adapter arm"))
        return rows
    rate = float(sidecar.get("fallback_rate", 1.0))
    p95 = float((sidecar.get("latency_ms") or {}).get("p95", 0.0))
    rows.append(
        _check(
            "fallback_rate",
            rate <= pins.MAX_FALLBACK_RATE,
            format(rate, ".4f") + " against the ceiling " + str(pins.MAX_FALLBACK_RATE),
        )
    )
    rows.append(
        _check(
            "scoring_latency",
            p95 <= pins.MAX_SCORING_P95_MS,
            "scorer p95 " + format(p95, ".1f") + "ms against " + str(pins.MAX_SCORING_P95_MS) + "ms",
        )
    )
    return rows


def _check(name: str, ok, detail: str) -> dict:
    """One check: `pass`, `fail`, or `skipped` when it could not run."""
    status = "skipped" if ok is None else ("pass" if ok else "fail")
    return {"name": name, "status": status, "detail": detail}


def _number(value):
    """Compare numeric knobs by value, and sequences element by element."""
    if isinstance(value, (list, tuple)):
        return [_number(item) for item in value]
    if isinstance(value, bool) or value is None:
        return value
    if isinstance(value, (int, float)):
        return round(float(value), 6)
    return value


def adapter(options: dict) -> dict | None:
    """The adapter manifest, when one was supplied."""
    path = options.get("adapter_manifest")
    return read(path) if path else None



def read(path: str) -> dict:
    """Read one JSON document, or refuse naming the file."""
    if not os.path.isfile(path):
        raise pins.RecipeError("no such file: " + path, pins.EX_DATAERR)
    with open(path, "r", encoding="utf-8") as handle:
        try:
            return json.load(handle)
        except ValueError as exc:
            raise pins.RecipeError(path + " is not valid JSON: " + str(exc)) from exc
