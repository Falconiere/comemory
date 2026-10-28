#!/usr/bin/env python3
"""Real-data tests for dup_ledger_lint.py.

The rejected rows are literal rows this repo's dup-ledger.txt carried when the
lint was added (category restatements with no reason); the accepted rows state
a concrete reason or are framework-forced with the framework named.

Run: python3 scripts/lib/test_dup_ledger_lint.py
"""

import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from dup_ledger_lint import row_problem

BOILERPLATE_ROWS = [
    "src/domains/retrieval/doc_route.rs|local_hit|shared_hit|documented-lifecycle|"
    "retrieval/graph twin local_hit/shared_hit",
    "src/domains/sync/client.rs|device_code|poll_token|documented-lifecycle|"
    "sync lifecycle twin device_code/poll_token",
    "src/serve/routes/memories/write.rs|feedback|save|documented-lifecycle|"
    "serve handler twin feedback/save",
    "src/store/memory_row.rs|live_bodies|live_ids|twin-table|"
    "store twin accessors live_bodies/live_ids",
    "src/domains/code/git_utils.rs|hook_installed|hook_outdated|documented-lifecycle|"
    "deliberate residual hook_installed/hook_outdated in src/domains/code/git_utils.rs",
    "src/cli/capture.rs|run_session|run_sources|documented-lifecycle|"
    "cli deliberate twin run_session/run_sources; docs/dup-debt.md",
]

REASONED_ROWS = [
    "src/store/bandit_arms.rs|load|seed|twin-table|"
    "load is a SELECT decoding ArmState and seed is an INSERT OR IGNORE of eleven "
    "columns, so only the orm builder idiom is shared",
    "src/mcp/tools_read.rs|memory_get|memory_list|framework-forced|"
    "rmcp-tool-router-pre-expansion",
]


def main() -> int:
    """Run every case; exit 1 naming the first failing row."""
    for row in BOILERPLATE_ROWS:
        if row_problem(row) is None:
            sys.stderr.write(f"boilerplate row was accepted: {row}\n")
            return 1
    for row in REASONED_ROWS:
        problem = row_problem(row)
        if problem is not None:
            sys.stderr.write(f"reasoned row was rejected ({problem}): {row}\n")
            return 1
    if row_problem("src/a.rs|f|g|framework-forced|short") is None:
        sys.stderr.write("a framework-forced row naming no framework was accepted\n")
        return 1
    if row_problem("src/a.rs|f|g") is None:
        sys.stderr.write("a malformed row was accepted\n")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
