#!/usr/bin/env bash
# Real binary, happy-path smoke (release build): version, index-code →
# search-code → feedback.
# Full user-journey coverage lives in tests/cli_scenario_*.rs (nextest).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

QWICK_HOME=$(mktemp -d)
trap 'rm -rf "$QWICK_HOME"' EXIT

export COMEMORY_DATA_DIR="$QWICK_HOME/.comemory"
# This smoke tests the command cores, not the required daemon (#257); the
# hermetic switch keeps it from spawning one for a throwaway home.
export COMEMORY_SYNC_DAEMON=0
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

# ── required sync daemon smoke (#257) ─────────────────────────────────────
# Everything above deliberately opts the daemon out (`COMEMORY_SYNC_DAEMON=0`)
# to smoke the command cores in isolation; this section is the one place in
# this script that exercises the required daemon itself — the release
# binary, `process` supervision (never the developer's own launchd/systemd),
# and an isolated `$HOME` so a real unit is never written outside `$QWICK_HOME`.
export COMEMORY_DATA_DIR="$QWICK_HOME/.comemory-daemon"
export HOME="$QWICK_HOME/daemon-home"
mkdir -p "$HOME"
export COMEMORY_DAEMON_SUPERVISOR=process
unset COMEMORY_SYNC_DAEMON
# Stop the isolated coordinator even when a later smoke assertion fails.
trap '"$BIN" sync daemon stop --json >/dev/null 2>&1 || true; rm -rf "$QWICK_HOME"' EXIT

"$BIN" sync daemon ensure --json | grep -q '"ready":true' \
  || die "e2e" "sync daemon ensure failed"
"$BIN" sync daemon status --json | grep -q '"state":"running"' \
  || die "e2e" "sync daemon status did not report running"
"$BIN" save --repo e2e-daemon-smoke \
  "a decision made while the required daemon smoke was running" --json >/dev/null \
  || die "e2e" "save failed with the daemon required"
"$BIN" sync daemon stop --json >/dev/null || die "e2e" "sync daemon stop failed"
log_ok "e2e" "required sync daemon smoke passed"

