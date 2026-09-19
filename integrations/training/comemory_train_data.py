"""Read a `comemory export-dataset` directory, and refuse everything it must.

This module is the structural half of "final test data stays out of training,
mining and selection". It opens exactly the three files named by
`comemory_train_pins.READABLE_FILES` and there is no code path here, or in
anything that calls it, that opens a fourth. `holdout.jsonl` is *recorded* —
its digest names which held-out set the resulting adapter has to be qualified
against — and never parsed. The offline suite proves that by overwriting the
holdout with bytes that are not JSON and asserting the output does not change.

Everything else it does is a refusal. A dataset whose manifest declares a
version this recipe does not know, whose file digests disagree with the manifest
that describes them, whose rows carry a split other than their file's, or whose
train and validation splits share a group, is refused rather than trained on: a
silent acceptance here becomes an unexplainable number three steps later.
"""

from __future__ import annotations

import hashlib
import json
import os
from dataclasses import dataclass, field

import comemory_train_pins as pins


@dataclass(frozen=True)
class Example:
    """One reviewed (query, passage, grade) triple, ready for the loop."""

    observation_id: str
    group_id: str
    domain: str
    candidate_ref: str
    query: str
    text: str
    relevance: int
    target: float


@dataclass
class Dataset:
    """Everything the recipe needs from one export, and its provenance."""

    directory: str
    dataset_id: str
    snapshot_digest: str
    manifest_sha256: str
    split_policy: dict
    files_read: list
    holdout: dict
    retrieval_revisions: list
    train: list = field(default_factory=list)
    validation: list = field(default_factory=list)
    counters: dict = field(default_factory=dict)

    def rows_by_split(self) -> dict:
        """Reviewed rows admitted per split. The holdout is never counted."""
        return {
            pins.TRAIN_SPLIT: len(self.train),
            pins.VALIDATION_SPLIT: len(self.validation),
        }

    def rows_by_domain(self) -> dict:
        """Admitted rows per corpus, over both readable splits."""
        return _tally(e.domain for e in self.train + self.validation)

    def rows_by_relevance(self) -> dict:
        """Admitted rows per grade, over both readable splits."""
        return _tally(str(e.relevance) for e in self.train + self.validation)

    def positive_observations(self) -> int:
        """Train observations carrying at least one grade above zero."""
        return len({e.observation_id for e in self.train if e.relevance > 0})


def load(directory: str) -> Dataset:
    """Load and validate one export, or raise `RecipeError`."""
    manifest_path = os.path.join(directory, pins.DATASET_MANIFEST_FILE)
    manifest = read_manifest(manifest_path)
    dataset = Dataset(
        directory=directory,
        dataset_id=str(manifest.get("dataset_id", "")),
        snapshot_digest=str(manifest.get("snapshot_digest", "")),
        manifest_sha256=sha256_file(manifest_path),
        split_policy=_split_policy(manifest),
        files_read=[],
        holdout=_holdout_entry(manifest, directory),
        retrieval_revisions=revisions(manifest),
        counters={"dropped_unjudged": 0, "dropped_provenance": 0},
    )
    train_rows = _load_split(dataset, manifest, "train.jsonl", pins.TRAIN_SPLIT)
    validation_rows = _load_split(
        dataset, manifest, "validation.jsonl", pins.VALIDATION_SPLIT
    )
    _refuse_group_overlap(train_rows, validation_rows)
    dataset.train = [_example(row) for row in train_rows]
    dataset.validation = [_example(row) for row in validation_rows]
    _refuse_empty(dataset)
    return dataset


def sha256_file(path: str) -> str:
    """Hex SHA-256 of a file, read in bounded chunks."""
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_manifest(path: str) -> dict:
    """Read the export manifest and gate every version it declares."""
    if not os.path.isfile(path):
        raise pins.RecipeError(
            "no " + pins.DATASET_MANIFEST_FILE + " in " + os.path.dirname(path)
            + "; point --dataset at a `comemory export-dataset --out` directory",
            pins.EX_DATAERR,
        )
    try:
        with open(path, "r", encoding="utf-8") as handle:
            manifest = json.load(handle)
    except ValueError as exc:
        raise pins.RecipeError(path + " is not valid JSON: " + str(exc)) from exc
    if not isinstance(manifest, dict):
        raise pins.RecipeError(path + " must be a JSON object")
    _require_version(manifest, "manifest_version", pins.SUPPORTED_MANIFEST_VERSION, path)
    _require_version(manifest, "record_version", pins.SUPPORTED_RECORD_VERSION, path)
    _require_version(
        manifest, "observation_version", pins.SUPPORTED_OBSERVATION_VERSION, path
    )
    return manifest


def _require_version(payload: dict, key: str, supported: int, where: str) -> None:
    """Refuse a contract version this recipe does not implement."""
    found = payload.get(key)
    if found != supported:
        raise pins.RecipeError(
            where + " declares " + key + " " + repr(found) + ", but this recipe reads "
            + str(supported) + "; a consumer of an unknown version must refuse, not guess",
            pins.EX_DATAERR,
        )


def _split_policy(manifest: dict) -> dict:
    """The split configuration, carried into the adapter manifest verbatim."""
    split = manifest.get("split") or {}
    return {
        "policy": split.get("policy"),
        "seed": split.get("seed"),
        "ratios": split.get("ratios"),
        "groups": split.get("groups"),
    }


def revisions(manifest: dict) -> list:
    """Every distinct retrieval revision the export was captured under."""
    rows = manifest.get("retrieval_revisions") or []
    return [
        {"knobs_hash": r.get("knobs_hash"), "corpus_digest": r.get("corpus_digest")}
        for r in rows
        if isinstance(r, dict)
    ]


def _holdout_entry(manifest: dict, directory: str) -> dict:
    """What the manifest says about the holdout, which is never read.

    The digest is recorded so the qualification can prove that the held-out set
    an adapter is scored against is the one withheld from its own training. The
    file itself is not opened here, and the `read` flag says so in the manifest
    the recipe writes.
    """
    entry = file_entry(manifest, pins.HOLDOUT_FILE)
    present = os.path.isfile(os.path.join(directory, pins.HOLDOUT_FILE))
    return {
        "path": pins.HOLDOUT_FILE,
        "sha256": (entry or {}).get("sha256"),
        "rows": (entry or {}).get("rows"),
        "present_in_directory": present,
        "read": False,
    }


def file_entry(manifest: dict, name: str) -> dict | None:
    """The manifest's entry for one output file, written or withheld."""
    for key in ("files", "withheld"):
        for entry in manifest.get(key) or []:
            if isinstance(entry, dict) and entry.get("path") == name:
                return entry
    return None


def _load_split(dataset: Dataset, manifest: dict, name: str, split: str) -> list:
    """Verify one readable file against the manifest, then parse its rows."""
    if name not in pins.READABLE_FILES:
        raise pins.RecipeError(
            name + " is not in the trainer's readable-file allowlist "
            + ", ".join(pins.READABLE_FILES),
            pins.EX_SOFTWARE,
        )
    path = os.path.join(dataset.directory, name)
    if not os.path.isfile(path):
        raise pins.RecipeError(
            "no " + name + " in " + dataset.directory
            + "; `comemory export-dataset` always writes it",
            pins.EX_DATAERR,
        )
    entry = file_entry(manifest, name)
    if entry is None:
        raise pins.RecipeError(
            pins.DATASET_MANIFEST_FILE + " has no entry for " + name
            + ", so its contents cannot be verified against the export that produced them",
            pins.EX_DATAERR,
        )
    found = sha256_file(path)
    if entry.get("sha256") != found:
        raise pins.RecipeError(
            name + " does not match the manifest: manifest says "
            + str(entry.get("sha256")) + ", file is " + found,
            pins.EX_DATAERR,
        )
    dataset.files_read.append({"path": name, "sha256": found, "rows": entry.get("rows")})
    return _parse_rows(dataset, path, name, split)


def _parse_rows(dataset: Dataset, path: str, name: str, split: str) -> list:
    """Parse one JSONL file, gating every row and counting every drop."""
    rows = []
    with open(path, "r", encoding="utf-8") as handle:
        for number, line in enumerate(handle, start=1):
            if not line.strip():
                continue
            row = parse_row(line, name, number)
            require_row_version(row, name, number)
            require_row_split(row, name, number, split)
            label = row.get("label")
            if not isinstance(label, dict):
                dataset.counters["dropped_unjudged"] += 1
                continue
            if label.get("provenance") != pins.REVIEWED_PROVENANCE:
                dataset.counters["dropped_provenance"] += 1
                continue
            rows.append(row)
    return rows


def parse_row(line: str, name: str, number: int) -> dict:
    """One JSONL line as an object, or a refusal naming the line."""
    try:
        row = json.loads(line)
    except ValueError as exc:
        raise pins.RecipeError(
            name + " line " + str(number) + " is not valid JSON: " + str(exc)
        ) from exc
    if not isinstance(row, dict):
        raise pins.RecipeError(name + " line " + str(number) + " is not a JSON object")
    return row


def require_row_version(row: dict, name: str, number: int) -> None:
    """Refuse a record written at a version this recipe does not read."""
    found = row.get("record_version")
    if found != pins.SUPPORTED_RECORD_VERSION:
        raise pins.RecipeError(
            name + " line " + str(number) + " declares record_version " + repr(found)
            + ", but this recipe reads " + str(pins.SUPPORTED_RECORD_VERSION)
        )


def require_row_split(row: dict, name: str, number: int, split: str) -> None:
    """Refuse a row filed under a split other than its own file's."""
    found = row.get("split")
    if found != split:
        raise pins.RecipeError(
            name + " line " + str(number) + " carries split " + repr(found)
            + " but is in the " + split + " file; the split boundary is what keeps "
            "held-out data out of training"
        )


def _refuse_group_overlap(train_rows: list, validation_rows: list) -> None:
    """Refuse a group appearing in both readable splits.

    Deliberately redundant with the exporter's grouped-hash splitter. It costs
    one pass and it proves the property on the data actually in hand, rather
    than trusting that the producer held it.
    """
    train_groups = {row.get("group_id") for row in train_rows}
    for row in validation_rows:
        group = row.get("group_id")
        if group in train_groups:
            raise pins.RecipeError(
                "group " + repr(group) + " appears in both train.jsonl and "
                "validation.jsonl, so selecting on validation would select on data the "
                "model trained against"
            )


def _refuse_empty(dataset: Dataset) -> None:
    """Refuse a dataset that cannot support training or selection."""
    if not dataset.train:
        raise pins.RecipeError(
            "train.jsonl holds no reviewed row; export with a train ratio above zero "
            "and at least one `comemory judge` verdict"
        )
    if not dataset.validation:
        raise pins.RecipeError(
            "validation.jsonl holds no reviewed row; checkpoint selection reads "
            "validation only, so a run without it would select on training data or on "
            "nothing"
        )
    if dataset.positive_observations() == 0:
        raise pins.RecipeError(
            "no train observation carries a grade above zero, so the set states only "
            "what is irrelevant and carries no ranking signal"
        )


def _example(row: dict) -> Example:
    """One reviewed row as a training example, with its rescaled target."""
    relevance = int((row.get("label") or {}).get("relevance", 0))
    return Example(
        observation_id=str(row.get("observation_id", "")),
        group_id=str(row.get("group_id", "")),
        domain=str(row.get("domain", "")),
        candidate_ref=str(row.get("candidate_ref", "")),
        query=str(row.get("query", "")),
        text=str(row.get("text", "")),
        relevance=relevance,
        target=min(max(relevance, 0), pins.MAX_RELEVANCE) / float(pins.MAX_RELEVANCE),
    )


def _tally(values) -> dict:
    """Count occurrences into a key-sorted dictionary."""
    counts: dict = {}
    for value in values:
        counts[value] = counts.get(value, 0) + 1
    return dict(sorted(counts.items()))
