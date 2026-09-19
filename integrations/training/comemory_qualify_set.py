"""Turn a withheld holdout split into a reviewed #208 benchmark set.

This is the only component in this directory that reads `holdout.jsonl`, and it
reads it to evaluate, never to train. What it produces is an ordinary benchmark
set: `comemory benchmark` loads it, re-runs retrieval for every task, captures
one candidate pool per task, and scores every arm over that one snapshot. The
reviewed judgments are carried forward from `comemory judge` verdicts; nothing
here invents one.

Two mapping decisions are worth stating because they are not obvious.

`decay` is pinned to `0.0` whatever the export was captured under, because a
zero decay is what actually removes the wall clock from ACT-R activation, and a
benchmark whose numbers drift with the calendar is not reproducible. The value
it replaced is recorded in the provenance sidecar.

A document judgment pins `revision_hash` but **not** `chunk_ordinal`. The
revision is the document's content version, which is what a stale judgment
should be detected against. Which chunk wins is a ranking outcome — precisely
what the benchmark is measuring — so pinning it would mark a judgment stale
exactly when an arm did something interesting.
"""

from __future__ import annotations

import json
import os

import comemory_qualify_yaml as yaml_out
import comemory_train_data as data
import comemory_train_pins as pins

PROVENANCE_VERSION = 1
SET_VERSION = 1
FROZEN_DECAY = 0.0


def build(directory: str, options: dict) -> tuple:
    """Return `(set, provenance)` for one export's holdout split."""
    manifest = data.read_manifest(os.path.join(directory, pins.DATASET_MANIFEST_FILE))
    rows = _holdout_rows(directory, manifest)
    counters = {
        "dropped_unjudged": 0,
        "dropped_provenance": 0,
        "dropped_vector_scenario": 0,
        "dropped_domain_scope": 0,
        "dropped_contradictory_target": 0,
        "truncated_source_pools": 0,
    }
    tasks = _tasks(rows, counters, options)
    if not tasks:
        raise pins.RecipeError(
            "the holdout split carries no judged observation, so there is nothing to "
            "qualify against. Export with a holdout ratio above zero and "
            "--include-holdout, and record `comemory judge` verdicts for held-out runs.",
            pins.EX_DATAERR,
        )
    ranking, replaced = _ranking(manifest, options)
    document = {
        "version": SET_VERSION,
        "name": options["name"],
        "description": _description(manifest),
        "ranking": ranking,
        "defaults": {"k": options["k"], "max_text_bytes": options["max_text_bytes"]},
        "budgets": options["budgets"],
        "tasks": tasks,
    }
    provenance = {
        "provenance_version": PROVENANCE_VERSION,
        "dataset_id": manifest.get("dataset_id"),
        "snapshot_digest": manifest.get("snapshot_digest"),
        "dataset_manifest_sha256": data.sha256_file(
            os.path.join(directory, pins.DATASET_MANIFEST_FILE)
        ),
        "holdout": _holdout_entry(manifest),
        "set_name": options["name"],
        "set_ranking": ranking,
        "decay_replaced": replaced,
        "tasks": len(tasks),
        "judgments": sum(len(task["judgments"]) for task in tasks),
        "k": options["k"],
        "budgets": options["budgets"],
        "pin_content_version": options["pin_content_version"],
        "export_retrieval_revisions": data.revisions(manifest),
        "counters": counters,
    }
    return document, provenance


def _holdout_rows(directory: str, manifest: dict) -> list:
    """Read and verify `holdout.jsonl` against the manifest that describes it."""
    path = os.path.join(directory, pins.HOLDOUT_FILE)
    if not os.path.isfile(path):
        raise pins.RecipeError(
            "no " + pins.HOLDOUT_FILE + " in " + directory
            + "; re-run `comemory export-dataset` with --include-holdout. The holdout is "
            "withheld by default, and it is what this qualification scores against.",
            pins.EX_UNAVAILABLE,
        )
    entry = data.file_entry(manifest, pins.HOLDOUT_FILE)
    found = data.sha256_file(path)
    declared = (entry or {}).get("sha256")
    if entry is None or not declared:
        raise pins.RecipeError(
            pins.DATASET_MANIFEST_FILE + " carries no digest for " + pins.HOLDOUT_FILE
            + ", so the held-out set being scored cannot be shown to be the one the "
            "export produced"
        )
    if declared != found:
        raise pins.RecipeError(
            pins.HOLDOUT_FILE + " does not match the manifest: manifest says "
            + str(declared) + ", file is " + found
        )
    rows = []
    with open(path, "r", encoding="utf-8") as handle:
        for number, line in enumerate(handle, start=1):
            if line.strip():
                row = data.parse_row(line, pins.HOLDOUT_FILE, number)
                data.require_row_version(row, pins.HOLDOUT_FILE, number)
                data.require_row_split(row, pins.HOLDOUT_FILE, number, pins.HOLDOUT_SPLIT)
                rows.append(row)
    return rows


def _holdout_entry(manifest: dict) -> dict:
    """The manifest's record of the holdout, digest included."""
    entry = data.file_entry(manifest, pins.HOLDOUT_FILE) or {}
    return {
        "path": pins.HOLDOUT_FILE,
        "sha256": entry.get("sha256"),
        "rows": entry.get("rows"),
    }


def _tasks(rows: list, counters: dict, options: dict) -> list:
    """One task per judged holdout observation, in observation-id order."""
    grouped: dict = {}
    for row in rows:
        grouped.setdefault(str(row.get("observation_id")), []).append(row)
    tasks = []
    for observation_id in sorted(grouped):
        task = _task(observation_id, grouped[observation_id], counters, options)
        if task is not None:
            tasks.append(task)
    return tasks


def _task(observation_id: str, rows: list, counters: dict, options: dict) -> dict | None:
    """One observation as a task, or `None` when it cannot be expressed."""
    if any(row.get("pool_truncated") for row in rows):
        counters["truncated_source_pools"] += 1
    filters = rows[0].get("filters") or {}
    if (filters.get("vector") or {}).get("kind") == "supplied":
        counters["dropped_vector_scenario"] += 1
        return None
    scope = _scope(filters.get("domains") or [])
    if scope is None:
        counters["dropped_domain_scope"] += 1
        return None
    judgments = _judgments(rows, counters, options)
    if judgments is None:
        return None
    if not judgments:
        counters["dropped_unjudged"] += 1
        return None
    if any(j["target"]["domain"] not in _legs(scope) for j in judgments):
        counters["dropped_domain_scope"] += 1
        return None
    task = {
        "id": observation_id,
        "domain": scope,
        "query": str(rows[0].get("query", "")),
        "judgments": judgments,
    }
    narrowing = _filters(filters, _legs(scope))
    if narrowing:
        task["filters"] = narrowing
    return task


def _judgments(rows: list, counters: dict, options: dict) -> list | None:
    """The reviewed judgments of one observation, or `None` on a contradiction."""
    seen: dict = {}
    for row in rows:
        label = row.get("label")
        if not isinstance(label, dict):
            continue
        if label.get("provenance") != pins.REVIEWED_PROVENANCE:
            counters["dropped_provenance"] += 1
            continue
        target = _target(row.get("identity") or {}, options["pin_content_version"])
        if target is None:
            continue
        key = json.dumps(target, sort_keys=True)
        relevance = int(label.get("relevance", 0))
        if key in seen and seen[key] != relevance:
            counters["dropped_contradictory_target"] += 1
            return None
        seen[key] = relevance
    return [
        {"relevance": seen[key], "target": json.loads(key)} for key in sorted(seen)
    ]


def _target(identity: dict, pin: bool) -> dict | None:
    """A candidate identity as a human-writable judgment target."""
    domain = identity.get("domain")
    if domain == "memory":
        target = {"domain": "memory", "id": identity.get("memory_id")}
        return _pin(target, "content_hash", identity.get("content_hash"), pin)
    if domain == "code":
        target = {
            "domain": "code",
            "repo": identity.get("repo"),
            "path": identity.get("path"),
            "symbol": identity.get("symbol"),
        }
        return _pin(target, "blob_oid", identity.get("blob_oid"), pin)
    if domain == "document":
        target = {"domain": "document", "path": identity.get("path")}
        return _pin(target, "revision_hash", identity.get("revision_hash"), pin)
    return None


def _pin(target: dict, key: str, value, pin: bool) -> dict:
    """Attach the content-version pin, unless the operator turned it off."""
    if pin and value:
        target[key] = value
    return target


def _scope(domains: list) -> str | None:
    """The task scope a captured domain set maps onto, or `None`."""
    names = sorted({str(name) for name in domains})
    if names == ["code", "document", "memory"]:
        return "all"
    if len(names) == 1:
        return names[0]
    return None


def _legs(scope: str) -> tuple:
    """Which corpora a scope runs, so an inert filter is never emitted."""
    return ("memory", "code", "document") if scope == "all" else (scope,)


def _filters(filters: dict, legs: tuple) -> dict:
    """Every recorded filter that narrows a leg this task actually runs."""
    narrowing: dict = {}
    owners = {
        "repo": ("memory", "code"),
        "kind": ("memory",),
        "since": ("memory",),
        "until": ("memory",),
        "as_of": ("memory",),
        "lang": ("code",),
    }
    for key, owned_by in owners.items():
        value = filters.get(key)
        if value and any(leg in legs for leg in owned_by):
            narrowing[key] = value
    globs = filters.get("path_globs") or []
    if globs and "document" in legs:
        narrowing["path"] = list(globs)
    return narrowing


def _ranking(manifest: dict, options: dict) -> tuple:
    """The six pinned knobs, with ACT-R decay frozen, and what it replaced."""
    revisions = manifest.get("retrieval_revisions") or []
    hashes = sorted({str(r.get("knobs_hash")) for r in revisions})
    if len(hashes) > 1:
        raise pins.RecipeError(
            "this export spans " + str(len(hashes)) + " retrieval configurations ("
            + ", ".join(h[:12] for h in hashes) + "); one benchmark set pins one "
            "configuration, and silently choosing among them would qualify against a "
            "configuration half the data never saw"
        )
    knobs = (revisions[0].get("knobs") if revisions else None) or {}
    ranking = {
        "rrf_k": float(knobs.get("rrf_k", 60.0)),
        "decay": float(options["decay"]),
        "mmr_lambda": float(knobs.get("mmr_lambda", 0.7)),
        "bm25_weights": [float(v) for v in (knobs.get("bm25_weights") or [1.0, 3.0])],
        "graph_hops": int(knobs.get("graph_hops", 2)),
        "graph_seeds": int(knobs.get("graph_seeds", 8)),
    }
    replaced = knobs.get("decay")
    return ranking, (float(replaced) if replaced is not None else None)


def _description(manifest: dict) -> str:
    """One line naming the export this set was generated from."""
    return (
        "Generated by comemory_qualify.py build-set from the withheld holdout split of "
        "dataset " + str(manifest.get("dataset_id")) + " (snapshot "
        + str(manifest.get("snapshot_digest"))[:12] + "). Every judgment is a reviewed "
        "`comemory judge` verdict carried forward; none was invented here."
    )


def to_yaml(document: dict) -> str:
    """The set as a YAML document, through the standard-library emitter."""
    return yaml_out.dump(document)
