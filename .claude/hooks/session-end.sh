#!/usr/bin/env bash
# Stop hook: run the fast subset of gates so issues surface at end-of-conversation.

PROJECT_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$PROJECT_ROOT" || exit 0
echo "[session-end] running fast gates..."
bash scripts/fmt-check.sh             2>&1 | tail -3 || true
bash scripts/test-placement-check.sh  2>&1 | tail -3 || true
bash scripts/no-bypass-check.sh       2>&1 | tail -3 || true
bash scripts/module-size-check.sh     2>&1 | tail -3 || true
echo "[session-end] done"
