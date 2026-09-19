"""Record the qualification outcome: go, no-go, or insufficient evidence.

Three words, and the mapping between evidence and word is a table rather than a
judgment call. `insufficient-evidence` when the benchmark's own verdict is
inconclusive, when an arm is missing, or when an integrity check failed so the
numbers cannot be trusted. `no-go` when every check ran and the evidence does
not support enabling — a regression, a neutral result, or an improvement that
breaches an operational budget. `go` only when the paired interval clears the
predeclared gain, every operational budget is met and every check passed.

**A check that could not run is never recorded as a pass.** Each one appears in
the record as `pass`, `fail` or `skipped` together with the values it compared,
and a `skipped` check caps the outcome at `insufficient-evidence`. This
repository's recurring failure mode is a gate that reports success while
checking nothing, and a qualification that concluded `go` from checks it never
performed would be the most expensive instance of it.

A negative result is a valid result. Nothing here prefers `go`.
"""

from __future__ import annotations

import json
import os

import comemory_qualify_checks as verify
import comemory_train_pins as pins

DECISION_VERSION = 1


def decide(options: dict) -> dict:
    """Build the decision record for one qualification run."""
    report = verify.read(options["report"])
    provenance = verify.read(options["provenance"])
    sidecars = {name: verify.read(path) for name, path in options["scoring"].items()}
    arms = {str(arm.get("name")): arm for arm in report.get("arms") or []}
    lora = arms.get(options["lora_arm"])
    base = arms.get(options["base_arm"])
    checks = verify.checks(report, provenance, sidecars, arms, options)
    budgets = verify.budgets(report, lora, sidecars.get(options["lora_arm"]))
    missing = [
        name
        for name, arm in ((options["base_arm"], base), (options["lora_arm"], lora))
        if arm is None
    ]
    outcome, reasons = _outcome(checks, budgets, lora, missing)
    return {
        "decision_version": DECISION_VERSION,
        "outcome": outcome,
        "reasons": reasons,
        "arms": {
            "baseline": _arm_record(arms.get(pins.BASELINE_ARM)),
            "base": _arm_record(base),
            "lora": _arm_record(lora),
        },
        "missing_arms": missing,
        "checks": checks,
        "budgets": budgets,
        "scoring": {name: _scoring_record(car) for name, car in sidecars.items()},
        "provenance": {
            "dataset_id": provenance.get("dataset_id"),
            "snapshot_digest": provenance.get("snapshot_digest"),
            "holdout": provenance.get("holdout"),
            "set_name": report.get("set_name"),
            "set_size": report.get("set_size"),
            "k": report.get("k"),
            "corpus_digest": ((report.get("retrieval") or {}).get("corpus") or {}).get("digest"),
            "knobs_hash": (report.get("retrieval") or {}).get("knobs_hash"),
            "export_retrieval_revisions": provenance.get("export_retrieval_revisions"),
            "retrieval_configuration_drift": _drift(report, provenance),
            "adapter_id": (verify.adapter(options) or {}).get("adapter_id"),
            "decay_replaced": provenance.get("decay_replaced"),
            "build_counters": provenance.get("counters"),
        },
        "declared_budgets": report.get("budgets"),
    }


def _outcome(checks: list, budgets: list, lora: dict | None, missing: list) -> tuple:
    """Read the three-word outcome off the checks and the budgets."""
    failed = [row["name"] for row in checks if row["status"] == "fail"]
    skipped = [row["name"] for row in checks + budgets if row["status"] == "skipped"]
    verdict = (lora or {}).get("verdict")
    if missing:
        return pins.OUTCOME_INSUFFICIENT, ["no arm named " + name for name in missing]
    if failed:
        return pins.OUTCOME_INSUFFICIENT, ["integrity check failed: " + name for name in failed]
    if verdict == "inconclusive":
        return pins.OUTCOME_INSUFFICIENT, [
            "the benchmark verdict is inconclusive: fewer judged tasks than the declared "
            "min_tasks, so no comparison is available"
        ]
    if skipped:
        return pins.OUTCOME_INSUFFICIENT, ["check did not run: " + name for name in skipped]
    breached = [row["name"] for row in budgets if row["status"] == "fail"]
    if breached:
        return pins.OUTCOME_NO_GO, ["budget not met: " + name for name in breached]
    return pins.OUTCOME_GO, ["every declared budget met and every integrity check passed"]


def _arm_record(arm: dict | None) -> dict | None:
    """One arm's numbers, per domain and overall, as the record carries them."""
    if arm is None:
        return None
    return {
        "name": arm.get("name"),
        "scorer_version": arm.get("scorer_version"),
        "scored_fraction": arm.get("scored_fraction"),
        "verdict": arm.get("verdict"),
        "overall": arm.get("overall"),
        "per_domain": arm.get("per_domain"),
        "retrieval_latency_ms": arm.get("latency_ms"),
        "latency_within_budget": arm.get("latency_within_budget"),
        "paired_vs_baseline": arm.get("paired_vs_baseline"),
    }


def _scoring_record(sidecar: dict) -> dict:
    """The operational half of an arm: what the child process cost."""
    return {
        "model": sidecar.get("model"),
        "adapter": sidecar.get("adapter"),
        "invocations": sidecar.get("invocations"),
        "failures": sidecar.get("failures"),
        "fallback_rate": sidecar.get("fallback_rate"),
        "ambiguous_refs": sidecar.get("ambiguous_refs"),
        "candidates_scored": sidecar.get("candidates_scored"),
        "scoring_latency_ms": sidecar.get("latency_ms"),
        "peak_child_rss": sidecar.get("peak_child_rss"),
        "failure_reasons": sidecar.get("failure_reasons"),
    }


def _drift(report: dict, provenance: dict) -> dict:
    """Whether the data was captured under the configuration it is scored under.

    Reported, never fatal. Every arm reorders one pool captured in one run, so
    the comparison stays internally valid either way; what a drift qualifies is
    how far the result generalizes to the configuration the labels came from.
    """
    applied = (report.get("retrieval") or {}).get("knobs_hash")
    captured = sorted(
        {str(r.get("knobs_hash")) for r in provenance.get("export_retrieval_revisions") or []}
    )
    return {
        "qualified_under": applied,
        "captured_under": captured,
        "differs": bool(captured) and applied not in captured,
    }
