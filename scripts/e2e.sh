#!/usr/bin/env bash
# Real binary, happy-path smoke. Stand-in for the full e2e flow.
# Slices that land later (save/index-code/context) extend the assertions here.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

QWICK_HOME=$(mktemp -d)
trap 'rm -rf "$QWICK_HOME"' EXIT

export COMEMORY_DATA_DIR="$QWICK_HOME/.comemory"
cd "$PROJECT_ROOT"
cargo build --release --quiet
BIN="$PROJECT_ROOT/target/release/comemory"

"$BIN" --version | grep -q "comemory" || die "e2e" "version check failed"
log_ok "e2e" "version smoke passed"

# ── index-code → search-code → feedback --used-code round-trip ───────────
REPO_DIR="$QWICK_HOME/fixture-repo"
mkdir -p "$REPO_DIR/src"
# `pub fn` on purpose: pins the extractor's coverage of visibility-modified
# definitions (the common shape in real repos) end to end.
cat > "$REPO_DIR/src/lib.rs" <<'EOF'
pub fn parse_frontmatter(input: &str) -> Option<&str> {
    input.strip_prefix("---")
}
EOF
git -C "$REPO_DIR" init --quiet
git -C "$REPO_DIR" -c user.email=e2e@example.com -c user.name=e2e add -A
git -C "$REPO_DIR" -c user.email=e2e@example.com -c user.name=e2e \
  commit --quiet -m "fixture"

"$BIN" index-code --repo fixture --path "$REPO_DIR" --json >/dev/null \
  || die "e2e" "index-code failed"

SEARCH_JSON=$(cd "$REPO_DIR" \
  && "$BIN" search-code "parse frontmatter" --repo fixture --json)
SYMBOL_ID=$(printf '%s' "$SEARCH_JSON" | sed -n 's/.*"symbol_id":\([0-9][0-9]*\).*/\1/p')
QUERY_ID=$(printf '%s' "$SEARCH_JSON" | sed -n 's/.*"query_id":"\([^"]*\)".*/\1/p')
[[ -n "$SYMBOL_ID" && -n "$QUERY_ID" ]] \
  || die "e2e" "search-code returned no ranked hit / query_id"

"$BIN" feedback "$QUERY_ID" --used-code "$SYMBOL_ID" --json >/dev/null \
  || die "e2e" "feedback --used-code failed"
log_ok "e2e" "index-code → search-code → feedback round-trip passed"
