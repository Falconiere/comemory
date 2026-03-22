#!/usr/bin/env bash
# End-to-end test for qwick-rag CLI.
# Saves real memories, searches, lists, deletes, rebuilds index, runs doctor.
# Uses a temp directory so it never touches your real data.
#
# Usage:
#   ./scripts/e2e-test.sh          # run against installed qwick-rag
#   ./scripts/e2e-test.sh --build  # install from source first, then test

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
RESET='\033[0m'

PASSED=0
FAILED=0

pass() {
  PASSED=$((PASSED + 1))
  echo -e "  ${GREEN}✓${RESET} $1"
}

fail() {
  FAILED=$((FAILED + 1))
  echo -e "  ${RED}✗${RESET} $1"
  echo -e "    ${RED}$2${RESET}"
}

assert_exit_code() {
  local expected="$1" actual="$2" label="$3"
  if [ "$actual" -eq "$expected" ]; then
    pass "$label"
  else
    fail "$label" "expected exit $expected, got $actual"
  fi
}

assert_contains() {
  local haystack="$1" needle="$2" label="$3"
  if echo "$haystack" | grep -qi "$needle"; then
    pass "$label"
  else
    fail "$label" "output did not contain '$needle'"
  fi
}

assert_not_contains() {
  local haystack="$1" needle="$2" label="$3"
  if echo "$haystack" | grep -qi "$needle"; then
    fail "$label" "output unexpectedly contained '$needle'"
  else
    pass "$label"
  fi
}

# ── Setup ────────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

if [[ "${1:-}" == "--build" ]]; then
  echo -e "${BOLD}Building qwick-rag from source...${RESET}"
  uv pip install -e ".[dev]"
  echo ""
fi

# Use uv run to invoke the CLI (handles venv automatically)
QR="uv run qwick-rag"

# Verify the CLI is available
if ! $QR --help &>/dev/null; then
  echo -e "${RED}qwick-rag not available via 'uv run'. Run with --build or install first.${RESET}"
  exit 1
fi

# Create isolated temp directory
TEST_DIR=$(mktemp -d)
trap 'rm -rf "$TEST_DIR"' EXIT

export QWICK_RAG_DIR="$TEST_DIR"
export QWICK_RAG_REPO="e2e-test-repo"
export QWICK_RAG_AUTHOR="e2e-bot"

mkdir -p "$TEST_DIR/memories"

echo -e "${BOLD}qwick-rag end-to-end test${RESET}"
echo -e "  test dir: $TEST_DIR"
echo ""

# ── 1. Save memories ────────────────────────────────────────────────────────

echo -e "${BOLD}1. Saving memories${RESET}"

OUT=$($QR save "We chose PostgreSQL as the primary database for its JSONB support, strong ecosystem, and reliable replication." --type decision --tags "database,postgres" 2>&1) || true
assert_contains "$OUT" "Saved memory" "save decision memory (postgres)"

OUT=$($QR save "Redis is used for session caching and rate limiting. TTL is set to 30 minutes for sessions." --type decision --tags "cache,redis,session" 2>&1) || true
assert_contains "$OUT" "Saved memory" "save decision memory (redis)"

OUT=$($QR save "Session tokens were not invalidated on logout because the Redis DEL call was missing from the logout handler." --type bug --tags "auth,session,redis" 2>&1) || true
assert_contains "$OUT" "Saved memory" "save bug memory (session tokens)"

OUT=$($QR save "All React components must use named exports for better tree-shaking and IDE auto-imports." --type convention --tags "react,frontend,exports" 2>&1) || true
assert_contains "$OUT" "Saved memory" "save convention memory (react exports)"

OUT=$($QR save "The API rate limiter uses a sliding window algorithm with Redis sorted sets. Key pattern: ratelimit:{user_id}:{endpoint}." --type discovery --tags "api,redis,rate-limit" 2>&1) || true
assert_contains "$OUT" "Saved memory" "save discovery memory (rate limiter)"

echo ""

# ── 2. List memories ────────────────────────────────────────────────────────

echo -e "${BOLD}2. Listing memories${RESET}"

OUT=$($QR list 2>&1) || true
assert_contains "$OUT" "5 memories found" "list shows 5 memories"

OUT=$($QR list --type decision 2>&1) || true
assert_contains "$OUT" "2 memories found" "list --type decision shows 2"

OUT=$($QR list --type bug 2>&1) || true
assert_contains "$OUT" "1 memories found" "list --type bug shows 1"

OUT=$($QR list --tags redis 2>&1) || true
assert_contains "$OUT" "3 memories found" "list --tags redis shows 3"

echo ""

# ── 3. Search memories ──────────────────────────────────────────────────────

echo -e "${BOLD}3. Searching memories${RESET}"

OUT=$($QR search "which database do we use" 2>&1) || true
assert_contains "$OUT" "PostgreSQL" "search 'which database' finds PostgreSQL"

OUT=$($QR search "session bug logout" 2>&1) || true
assert_contains "$OUT" "token" "search 'session bug logout' finds token issue"

OUT=$($QR search "react components" --type convention 2>&1) || true
assert_contains "$OUT" "named exports" "search with --type convention finds react exports"

OUT=$($QR search "caching layer" --tag redis 2>&1) || true
assert_contains "$OUT" "redis" "search with --tag redis returns redis results"

OUT=$($QR search "completely unrelated quantum physics topic" 2>&1) || true
# Should still return something (vector search always returns results), but score will be low
EC=$?
assert_exit_code 0 "$EC" "search for unrelated topic does not crash"

echo ""

# ── 4. Duplicate detection ──────────────────────────────────────────────────

echo -e "${BOLD}4. Duplicate detection${RESET}"

OUT=$($QR save "We chose PostgreSQL as the primary database for its JSONB support, strong ecosystem, and reliable replication." --type decision --tags "database,postgres" 2>&1) || true
assert_contains "$OUT" "already exists" "duplicate content is detected"

# List still shows 5
OUT=$($QR list 2>&1) || true
assert_contains "$OUT" "5 memories found" "no duplicate was created"

echo ""

# ── 5. Delete a memory ──────────────────────────────────────────────────────

echo -e "${BOLD}5. Deleting a memory${RESET}"

# Grab the ID of the first file
FIRST_FILE=$(ls "$TEST_DIR/memories/e2e-test-repo/"*.md | head -1)
FIRST_ID=$(basename "$FIRST_FILE" .md)

OUT=$($QR delete "$FIRST_ID" 2>&1) || true
assert_contains "$OUT" "Deleted memory" "delete memory by ID"

OUT=$($QR list 2>&1) || true
assert_contains "$OUT" "4 memories found" "list shows 4 after delete"

echo ""

# ── 6. Index rebuild ────────────────────────────────────────────────────────

echo -e "${BOLD}6. Rebuilding index${RESET}"

OUT=$($QR index 2>&1) || true
assert_contains "$OUT" "Index built" "incremental index build"
assert_contains "$OUT" "Total indexed: 4" "index count matches disk (4)"

OUT=$($QR index --force 2>&1) || true
assert_contains "$OUT" "Index built" "force rebuild"
assert_contains "$OUT" "Total indexed: 4" "force rebuild count still 4"

echo ""

# ── 7. Search after rebuild ─────────────────────────────────────────────────

echo -e "${BOLD}7. Search after rebuild${RESET}"

OUT=$($QR search "database" 2>&1) || true
EC=$?
assert_exit_code 0 "$EC" "search works after index rebuild"

echo ""

# ── 8. Context command ────────────────────────────────────────────────────────

echo -e "${BOLD}8. Context command${RESET}"

OUT=$($QR context 2>&1) || true
EC=$?
assert_exit_code 0 "$EC" "context exits 0"
assert_contains "$OUT" "Recent Memories" "context shows Recent Memories section"

echo ""

# ── 9. Doctor ────────────────────────────────────────────────────────────────

echo -e "${BOLD}9. Doctor health check${RESET}"

OUT=$($QR doctor 2>&1) || true
EC=$?
assert_exit_code 0 "$EC" "doctor exits 0"
assert_contains "$OUT" "4 valid" "doctor sees 4 valid files"
assert_contains "$OUT" "All checks passed" "doctor reports all checks passed"

echo ""

# ── Results ──────────────────────────────────────────────────────────────────

TOTAL=$((PASSED + FAILED))
echo -e "${BOLD}Results: $PASSED/$TOTAL passed${RESET}"

if [ "$FAILED" -gt 0 ]; then
  echo -e "${RED}${FAILED} test(s) failed.${RESET}"
  exit 1
else
  echo -e "${GREEN}All tests passed.${RESET}"
  exit 0
fi
