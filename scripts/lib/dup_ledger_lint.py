#!/usr/bin/env python3
"""Reject dup-ledger.txt rows whose evidence only restates the pair.

A ledger row is an *earned* exemption: the two functions look alike to
similarity-rs but no helper can remove the shared part without a worse
function. The `evidence` column is where that reason lives. Rows that carry
only the pair's own names plus a category phrase ("retrieval/graph twin
local_hit/shared_hit") record no reason at all, so the gate rejects them and
the ledger cannot become a free pass for any new twin.

Usage: dup_ledger_lint.py LEDGER_FILE   (exit 1 and name each bad row)
"""

import pathlib
import re
import sys

MIN_REASON_WORDS = 5
# `framework-forced` rows name the framework whose expansion forces the shape
# (`rmcp-tool-router-pre-expansion`), which is the whole reason in one token.
MIN_FRAMEWORK_CHARS = 12
# Category phrases earlier rows used in place of a reason.
BOILERPLATE_PHRASES = re.compile(
    r"retrieval/graph|sync lifecycle|serve handler|store twin (?:surface|accessors)"
    r"|per-language|cli deliberate|docs/dup-debt\.md|deliberate residual|utility twin",
    re.IGNORECASE,
)
FILLER_WORDS = {"twin", "twins", "in", "the", "a", "an", "of", "and", "to", "deliberate"}
WORD = re.compile(r"[A-Za-z0-9_`'-]+")


def reason_words(path: str, fn_a: str, fn_b: str, evidence: str) -> list[str]:
    """Words of `evidence` left once names, path pieces and filler are removed."""
    text = BOILERPLATE_PHRASES.sub(" ", evidence)
    own = {fn_a.lower(), fn_b.lower()} | {p.lower() for p in re.split(r"[/._]", path) if p}
    kept = []
    for raw in WORD.findall(text):
        word = raw.strip("`'-").lower()
        if word and word not in own and word not in FILLER_WORDS:
            kept.append(word)
    return kept


def row_problem(row: str) -> str | None:
    """Why `row` is unacceptable, or None when its evidence states a reason."""
    parts = row.rstrip("\n").split("|", 4)
    if len(parts) < 5:
        return "needs path|fnA|fnB|category|evidence"
    path, fn_a, fn_b, category, evidence = (p.strip() for p in parts)
    if category == "framework-forced":
        if len(evidence) < MIN_FRAMEWORK_CHARS:
            return f"framework-forced evidence must name the framework ({MIN_FRAMEWORK_CHARS}+ chars)"
        return None
    kept = reason_words(path, fn_a, fn_b, evidence)
    if len(kept) < MIN_REASON_WORDS:
        return (
            f"evidence states no reason ({len(kept)} of {MIN_REASON_WORDS} words beyond "
            "names and category phrases) — say why a shared helper would be worse"
        )
    return None


def lint(ledger: pathlib.Path) -> list[str]:
    """One message per offending row of `ledger`."""
    problems = []
    for number, row in enumerate(ledger.read_text().splitlines(), start=1):
        if not row.strip() or row.startswith("#"):
            continue
        problem = row_problem(row)
        if problem:
            problems.append(f"{ledger.name}:{number}: {problem}: {row.split('|', 3)[0:3]}")
    return problems


def main(argv: list[str]) -> int:
    """CLI entry: lint the ledger named by `argv[1]`."""
    if len(argv) != 2:
        sys.stderr.write(__doc__ or "")
        return 2
    problems = lint(pathlib.Path(argv[1]))
    for problem in problems:
        sys.stderr.write(problem + "\n")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
