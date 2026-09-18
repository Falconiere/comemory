#!/usr/bin/env bash
# Run the opt-in model suite, or refuse loudly — never quietly skip.
#
# This is the deliberate counterpart to the conformance suite that ships in the
# normal `cargo nextest run` path. That suite proves the protocol with no model
# at all. This one proves the model, and it needs several hundred megabytes of
# pinned weights and pinned libraries that comemory does not otherwise depend
# on, so it is opt-in.
#
# What it must never do is exit 0 when those prerequisites are missing. A
# skipped model test that reads as a pass is worse than no model test, because
# it converts "we have not checked this" into "we checked this and it was
# fine". So every missing prerequisite is collected, printed with the exact
# command that fixes it, and the script exits 69 (EX_UNAVAILABLE) with the
# words "the model suite did NOT run".
#
# A pinned library present at a DIFFERENT version is refused just as hard as an
# absent one, for the reason scripts/dup-check.sh refuses an unpinned
# similarity-rs: a result produced by another build is not the result this
# configuration produces, so comparing the two is meaningless.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
EX_UNAVAILABLE=69

printf 'comemory-rerank: checking the opt-in model suite prerequisites\n' >&2

if ! command -v python3 >/dev/null 2>&1; then
    printf 'comemory-rerank: python3 is not on PATH\n' >&2
    printf 'comemory-rerank: the model suite did NOT run\n' >&2
    exit "$EX_UNAVAILABLE"
fi

# The preflight is Python because the versions it enforces live in
# comemory_rerank_pins.py, and re-typing them here would create a second place
# for them to drift from requirements.txt.
if ! python3 "$HERE/comemory_rerank_preflight.py"; then
    printf 'comemory-rerank: the model suite did NOT run\n' >&2
    exit "$EX_UNAVAILABLE"
fi

printf 'comemory-rerank: prerequisites satisfied, running the model suite\n' >&2
cd "$HERE"
exec python3 -m unittest discover --start-directory tests --top-level-directory . --verbose
