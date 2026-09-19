#!/usr/bin/env bash
# Run the opt-in training suite, or refuse loudly — never quietly skip.
#
# This is the deliberate counterpart to the offline suite that ships in the
# normal `cargo nextest run` path. That suite proves the contracts: the pinned
# identity, the readable-file allowlist, the adapter shape the reference backend
# will accept, the YAML the benchmark set is written in. This one proves the
# model, and it needs several hundred megabytes of pinned weights and pinned
# libraries that comemory does not otherwise depend on, so it is opt-in.
#
# What it must never do is exit 0 when those prerequisites are missing. A
# skipped model test that reads as a pass is worse than no model test, because
# it converts "we have not checked this" into "we checked this and it was
# fine". So every missing prerequisite is printed with the command that fixes
# it, and the script exits 69 (EX_UNAVAILABLE) with the words "the training
# suite did NOT run".
#
# The preflight is the REFERENCE BACKEND's. integrations/training and
# integrations/reranker pin the same seven libraries at the same seven
# versions, deliberately and provably — tests/test_offline_recipe.py asserts the
# two requirements files agree line for line — because training and serving must
# run the same build or the save/reload parity check compares two of them. A
# second preflight would therefore be a second place for one pinned set to
# drift from itself.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
BACKEND="$(cd "$HERE/../reranker" && pwd)"
EX_UNAVAILABLE=69

printf 'comemory-train: checking the opt-in training suite prerequisites\n' >&2

if ! command -v python3 >/dev/null 2>&1; then
    printf 'comemory-train: python3 is not on PATH\n' >&2
    printf 'comemory-train: the training suite did NOT run\n' >&2
    exit "$EX_UNAVAILABLE"
fi

if ! python3 "$BACKEND/comemory_rerank_preflight.py"; then
    printf 'comemory-train: a pinned library is missing or at another version\n' >&2
    printf 'comemory-train: install them with: pip install -r integrations/training/requirements.txt\n' >&2
    printf 'comemory-train: the training suite did NOT run\n' >&2
    exit "$EX_UNAVAILABLE"
fi

printf 'comemory-train: prerequisites satisfied, running the training suite\n' >&2
exec python3 -m unittest discover \
    --start-directory "$HERE/tests" --pattern 'test_model_*.py' --verbose
