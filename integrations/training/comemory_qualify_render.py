"""Render a decision record as markdown a human can read without the JSON.

The document leads with the outcome and its reasons, then shows the evidence in
the order a sceptical reader asks for it: which integrity checks ran and what
they compared, which budgets were met, what each arm scored overall and per
corpus, and what the scorer cost. A value that was not measured is spelled
`not measured` rather than omitted, because an absent row reads as a zero and a
zero reads as a measurement.
"""

from __future__ import annotations

_STATUS = {"pass": "pass", "fail": "FAIL", "skipped": "skipped (did not run)"}


def markdown(decision: dict) -> str:
    """The whole decision as one markdown document."""
    out = ["# Qualification decision", ""]
    out.append("**Outcome: `" + str(decision.get("outcome")) + "`**")
    out.append("")
    for reason in decision.get("reasons") or []:
        out.append("- " + reason)
    out.append("")
    out.extend(_provenance(decision.get("provenance") or {}))
    out.extend(_table("Integrity checks", decision.get("checks") or []))
    out.extend(_table("Budgets", decision.get("budgets") or []))
    out.extend(_arms(decision.get("arms") or {}))
    out.extend(_scoring(decision.get("scoring") or {}))
    return "\n".join(out).rstrip() + "\n"


def _provenance(provenance: dict) -> list:
    """What was measured, over which data, under which configuration."""
    holdout = provenance.get("holdout") or {}
    drift = provenance.get("retrieval_configuration_drift") or {}
    size = provenance.get("set_size") or {}
    rows = [
        ("Benchmark set", str(provenance.get("set_name"))),
        ("Tasks / judgments", str(size.get("tasks")) + " / " + str(size.get("judgments"))),
        ("Cut (k)", str(provenance.get("k"))),
        ("Dataset", str(provenance.get("dataset_id"))),
        ("Snapshot digest", _short(provenance.get("snapshot_digest"))),
        ("Holdout digest", _short(holdout.get("sha256"))),
        ("Adapter", str(provenance.get("adapter_id") or "not supplied")),
        ("Corpus digest", _short(provenance.get("corpus_digest"))),
        ("Ranking knobs", _short(provenance.get("knobs_hash"))),
        (
            "Captured under another configuration",
            "yes — " + ", ".join(_short(h) for h in drift.get("captured_under") or [])
            if drift.get("differs")
            else "no",
        ),
        ("ACT-R decay replaced", str(provenance.get("decay_replaced"))),
    ]
    out = ["## What was measured", "", "| Fact | Value |", "| --- | --- |"]
    out.extend("| " + name + " | " + value + " |" for name, value in rows)
    out.append("")
    return out


def _table(title: str, rows: list) -> list:
    """One check or budget table, statuses spelled out in full."""
    out = ["## " + title, "", "| Check | Status | What was compared |", "| --- | --- | --- |"]
    for row in rows:
        out.append(
            "| `" + str(row.get("name")) + "` | " + _STATUS.get(str(row.get("status")), "?")
            + " | " + str(row.get("detail")) + " |"
        )
    out.append("")
    return out


def _arms(arms: dict) -> list:
    """Every arm's quality, overall and per corpus, with its paired interval."""
    out = ["## Arms", "", "| Arm | Scorer | Verdict | pool recall | recall@k | MRR | nDCG@k | paired delta |",
           "| --- | --- | --- | --- | --- | --- | --- | --- |"]
    for key in ("baseline", "base", "lora"):
        arm = arms.get(key)
        if arm is None:
            out.append("| " + key + " | not measured | not measured | — | — | — | — | — |")
            continue
        overall = arm.get("overall") or {}
        out.append(
            "| " + str(arm.get("name")) + " | " + str(arm.get("scorer_version") or "—")
            + " | " + str(arm.get("verdict")) + " | " + _num(overall.get("pool_recall"))
            + " | " + _num(overall.get("recall_at_k")) + " | " + _num(overall.get("mrr"))
            + " | " + _num(overall.get("ndcg_at_k")) + " | " + _delta(arm) + " |"
        )
    out.append("")
    out.extend(_per_domain(arms))
    return out


def _per_domain(arms: dict) -> list:
    """The per-corpus breakdown, which is where a mixed result usually hides."""
    out = ["### Per domain", "", "| Arm | Domain | judged tasks | pool recall | recall@k | nDCG@k |",
           "| --- | --- | --- | --- | --- | --- |"]
    empty = True
    for key in ("baseline", "base", "lora"):
        arm = arms.get(key)
        for entry in (arm or {}).get("per_domain") or []:
            domain, summary = entry[0], entry[1]
            if not summary.get("judged_tasks"):
                continue
            empty = False
            out.append(
                "| " + str(arm.get("name")) + " | " + str(domain) + " | "
                + str(summary.get("judged_tasks")) + " | " + _num(summary.get("pool_recall"))
                + " | " + _num(summary.get("recall_at_k")) + " | "
                + _num(summary.get("ndcg_at_k")) + " |"
            )
    if empty:
        out.append("| — | — | not measured | — | — | — |")
    out.append("")
    return out


def _scoring(scoring: dict) -> list:
    """What each scorer cost: latency, memory and the fallback rate."""
    out = ["## Operational cost", "",
           "| Arm | Model | Adapter | invocations | failures | fallback rate | scorer p50 / p95 ms | peak child RSS |",
           "| --- | --- | --- | --- | --- | --- | --- | --- |"]
    if not scoring:
        out.append("| not measured | — | — | — | — | — | — | — |")
    for name in sorted(scoring):
        row = scoring[name]
        latency = row.get("scoring_latency_ms") or {}
        rss = row.get("peak_child_rss") or {}
        out.append(
            "| " + name + " | " + str(row.get("model")) + " | " + str(row.get("adapter") or "—")
            + " | " + str(row.get("invocations")) + " | " + str(row.get("failures"))
            + " | " + _num(row.get("fallback_rate")) + " | " + _num(latency.get("p50"))
            + " / " + _num(latency.get("p95")) + " | " + str(rss.get("value")) + " "
            + str(rss.get("unit")) + " |"
        )
    out.append("")
    out.extend(_failures(scoring))
    return out


def _failures(scoring: dict) -> list:
    """Every recorded fallback, because a rate without its reasons is a number."""
    rows = [
        (name, failure)
        for name in sorted(scoring)
        for failure in scoring[name].get("failure_reasons") or []
    ]
    if not rows:
        return []
    out = ["### Fallbacks", "", "| Arm | Task | Kind | Code | Detail |", "| --- | --- | --- | --- | --- |"]
    for name, failure in rows:
        out.append(
            "| " + name + " | " + str(failure.get("task_id")) + " | "
            + str(failure.get("kind")) + " | " + str(failure.get("code"))
            + " | " + str(failure.get("detail")).replace("|", "\\|") + " |"
        )
    out.append("")
    return out


def _delta(arm: dict) -> str:
    """The paired delta against the baseline, with its interval."""
    delta = arm.get("paired_vs_baseline")
    if not delta:
        return "—"
    interval = delta.get("ci") or [None, None]
    return (
        _num(delta.get("mean_delta")) + " [" + _num(interval[0]) + ", " + _num(interval[1])
        + "] over " + str(delta.get("tasks")) + " tasks"
    )


def _num(value) -> str:
    """A number at three decimals, or the honest absence of one."""
    if value is None:
        return "not measured"
    return format(float(value), ".3f")


def _short(digest) -> str:
    """The first twelve characters of a digest, enough to compare by eye."""
    text = str(digest or "")
    return text[:12] if text else "not measured"
