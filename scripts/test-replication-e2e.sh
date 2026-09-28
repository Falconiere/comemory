#!/usr/bin/env bash
# Real-process replication harness. Local cases run here. Live cases boot
# the platform checkout named by --platform-root. See
# docs/designs/2026-09-21-replication-e2e-harness.md.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# The caller's git root is the platform checkout when CI invokes this script.
# Pins, the engine SHA, and cargo builds come from the tree that holds it.
ENGINE_ROOT="$(cd "$HERE/.." && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

# The coverage checker reads this list. Keep it in sync with coverage.json.
CASES=(baseline missing-runtime teardown fault-ack corrupt credentials propagation lost-nudge coverage contract memories code documents events exchange daemon install recovery homebrew release-formula)

case_name=""
platform_root=""
engine_bin=""
tap_root=""

runtime_fail() {
  printf 'replication: runtime: %s\n' "$1" >&2
  exit 2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --case)
      case_name="$2"
      shift 2
      ;;
    --platform-root)
      platform_root="$2"
      shift 2
      ;;
    --engine-bin)
      engine_bin="$2"
      shift 2
      ;;
    --tap-root)
      tap_root="$2"
      shift 2
      ;;
    *)
      die "replication" "unknown argument: $1"
      ;;
  esac
done

[[ -n "$case_name" ]] || die "replication" "--case is required"
known=0
for candidate in "${CASES[@]}"; do
  [[ "$candidate" == "$case_name" ]] && known=1
done
[[ "$known" -eq 1 ]] || runtime_fail "unknown case: $case_name"

reap_group() {
  local pid="$1" kill_at deadline
  kill -TERM -- "-$pid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true
  kill_at=$((SECONDS + 1))
  deadline=$((SECONDS + 15))
  while kill -0 "$pid" 2>/dev/null; do
    if (( SECONDS >= deadline )); then
      die "replication" "child $pid still running after 15s"
    fi
    if (( SECONDS >= kill_at )); then
      kill -KILL -- "-$pid" 2>/dev/null || kill -KILL "$pid" 2>/dev/null || true
    fi
    sleep 0.2
  done
}

run_missing_runtime() {
  local bin="${engine_bin:-/bin/false}"
  if [[ -x "$bin" ]] && "$bin" --version >/dev/null 2>&1; then
    die "replication" "missing-runtime was given a binary that prints --version: $bin"
  fi
  runtime_fail "engine binary cannot run: $bin"
}

run_teardown() {
  local pid
  # A new session that ignores TERM, so the deadline must SIGKILL it.
  pid="$(python3 -c '
import os, signal, time
child = os.fork()
if child == 0:
    os.setsid()
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    devnull = os.open(os.devnull, os.O_RDWR)
    os.dup2(devnull, 0)
    os.dup2(devnull, 1)
    os.dup2(devnull, 2)
    time.sleep(120)
    os._exit(0)
print(child, flush=True)
os._exit(0)
')"
  sleep 0.2
  if ! kill -0 "$pid" 2>/dev/null; then
    die "replication" "teardown child exited before the deadline"
  fi
  reap_group "$pid"
  if kill -0 "$pid" 2>/dev/null; then
    die "replication" "teardown child still running after 15s"
  fi
  log_ok "replication" "teardown reaped the child"
}

# The engine-local contract suite: the replica-v1 journal, receipts and
# ordering against real processes and real databases. Needs no platform
# checkout, so public CI runs it.
run_contract() {
  (
    cd "$ENGINE_ROOT"
    cargo nextest run --all-features --test replica_contract --test replica_contract_2
  )
  log_ok "replication" "contract suite passed"
}

# The memory-mutation surface (#251): every writer produces one operation, an
# interrupted write is recovered, and the identity/ordering rules hold. Two
# real spawned engines and the real CLI, in-repo like the contract case — no
# platform runtime to stand up.
run_memories() {
  (
    cd "$ENGINE_ROOT"
    cargo nextest run --all-features --test replica_memories --test replica_memories_2
  )
  log_ok "replication" "memory mutation suite passed"
}

# The code-generation surface (#252): one generation per indexed revision,
# a repository-sized manifest crossing whole, an import that never touches a
# local row, and the readers that see both sides. Real spawned engines and the
# real CLI over real git checkouts, in-repo like the contract case.
run_code() {
  (
    cd "$ENGINE_ROOT"
    cargo nextest run --all-features --test replica_code --test replica_code_2
  )
  log_ok "replication" "code generation suite passed"
}

# The document surface (#253): one portable identity two checkouts agree on,
# a rename journalled old-path-first, an import that touches no local row and
# writes no file, a machine with no registration answering from what it pulled,
# and a revoked repository going quiet. Real spawned engines and the real CLI
# over temp copies of this repository's own docs/, in-repo like the code case.
run_documents() {
  (
    cd "$ENGINE_ROOT"
    cargo nextest run --all-features --test replica_documents --test replica_documents_2
  )
  log_ok "replication" "document revision suite passed"
}

# The event surface (#254): verdicts and activity runs with stable ids and
# their provenance, counted exactly once through replays, echoes, an induced
# fault and a kill, retention and purge reaching the journal, and the text
# policy on what may leave. Real spawned engines, the real CLI and real HTTP,
# in-repo like the documents case.
run_events() {
  (
    cd "$ENGINE_ROOT"
    cargo nextest run --all-features --test replica_events --test replica_events_2 \
      --test replica_events_3
  )
  log_ok "replication" "feedback and activity event suite passed"
}

# The exchange client (#255): negotiation, a durable backlog drained in one
# run, a cursor that means durable contiguous handling, held states that never
# starve the rest, failures and rate limits survived without losing or
# doubling a change, restored and replaced streams, workspace switches, and
# code and documents both ways. Real `comemory serve` hubs behind a real
# fault proxy and the real CLI, in-repo like the contract case.
run_exchange() {
  (
    cd "$ENGINE_ROOT"
    cargo nextest run --all-features --test replica_exchange --test replica_exchange_2 \
      --test replica_exchange_3 --test replica_exchange_4 --test replica_exchange_5 \
      --test replica_exchange_6 --test replica_exchange_7 --test replica_exchange_8
  )
  log_ok "replication" "exchange client suite passed"
}

# The required resident coordinator (#257): discovery, preflight, lifecycle
# (ensure/restart/repair/stop/uninstall), auth/logout/status through it,
# triggers (hooks, save, the workspace channel), `watch` attached to it, and
# backlog/verify robustness. Real processes, real signals, a real engine hub
# behind a real fault proxy, in-repo like the contract case.
run_daemon() {
  (
    cd "$ENGINE_ROOT"
    cargo nextest run --all-features --test replica_daemon --test replica_daemon_2 \
      --test replica_daemon_3 --test replica_daemon_4 --test replica_daemon_5
  )
  log_ok "replication" "required sync daemon suite passed"
}

# Managed install and update (#258): install.sh, comemory upgrade and the
# source wrapper finish on a verified coordinator or roll back. The cargo
# suites run the real branch binary through the real installer against a
# loopback release server; the lifecycle script adds real published old
# binaries and two real branch builds under the headless supervisor. The
# native launchd/systemd modes run on disposable CI runners only.
run_install() {
  (
    cd "$ENGINE_ROOT"
    cargo nextest run --all-features --test install_script --test install_script_2 \
      --test install_daemon --test install_daemon_2 --test cli__upgrade --test upgrade_daemon
    bash scripts/test-daemon-install.sh --headless
  )
  log_ok "replication" "managed install suite passed"
}

# Replica state recovery (#256): seeding survives a legacy upgrade and a
# kill, rebuild and gc keep replica progress and an owed payload, erase
# leaves only the digest barrier, and a revoked repository's pulled cache
# hides without touching its local index; a rebuild waits for the exchange
# gate and pauses writers, and a failed upgrade never reads healthy. Real
# spawned engines, the real CLI and real HTTP, in-repo like the contract case.
run_recovery() {
  (
    cd "$ENGINE_ROOT"
    cargo nextest run --all-features --test replica_recovery --test replica_recovery_2 \
      --test replica_recovery_3 --test replica_recovery_4 --test replica_recovery_5
  )
  log_ok "replication" "replica state recovery suite passed"
}

# Homebrew channel (Falconiere/homebrew-tap#1): the tap owns its suites. The
# formula contract runs against the latest release's real formula; the
# lifecycle suite drives real brew install/reinstall/upgrade/uninstall and
# refuses (exit 2) off a disposable machine, which fails this case rather than
# skipping it. The tap's lifecycle workflow runs the native launchd/systemd
# and headless modes on every push.
run_homebrew() {
  [[ -n "$tap_root" ]] || runtime_fail "--tap-root is required for homebrew"
  [[ -f "$tap_root/scripts/test-brew-lifecycle.sh" ]] \
    || runtime_fail "not a homebrew-tap checkout with the lifecycle suite: $tap_root"
  local mode=--headless
  [[ "$(uname -s)" == Darwin ]] && mode=--native
  bash "$tap_root/scripts/test-formula-contract.sh"
  bash "$tap_root/scripts/test-brew-lifecycle.sh" "$mode"
  log_ok "replication" "homebrew channel suite passed ($mode)"
}

# The Homebrew publishing path (homebrew-tap#1, H-4): release.yml's real
# "Commit formula files" step, run against a real tap clone and a real release,
# must publish a formula that keeps the tap's lifecycle contract.
run_release_formula() {
  bash "$HERE/test-release-formula-step.sh"
  log_ok "replication" "release formula step suite passed"
}

run_coverage() {
  bash "$HERE/check-replication-coverage.sh"
  local bad
  bad="$(mktemp)"
  python3 - "$ENGINE_ROOT/scripts/replication/coverage.json" "$bad" <<'PY'
import json, sys
src, dst = sys.argv[1:]
data = json.load(open(src))
del data["acs"]["G-2"]
json.dump(data, open(dst, "w"))
PY
  if bash "$HERE/check-replication-coverage.sh" --manifest "$bad"; then
    rm -f "$bad"
    die "replication" "coverage accepted a manifest missing G-2"
  fi
  rm -f "$bad"
  local empty
  empty="$(mktemp)"
  printf '%s\n' '{"tests_ran":0,"skipped":0}' >"$empty"
  if bash "$HERE/check-replication-coverage.sh" --report "$empty"; then
    rm -f "$empty"
    die "replication" "coverage accepted a report with tests_ran 0"
  fi
  rm -f "$empty"
  log_ok "replication" "coverage rejected a dangling AC and an empty report"
}

require_ancestor() {
  local pin head
  [[ -d "$platform_root/.git" || -f "$platform_root/.git" ]] \
    || runtime_fail "platform root is not a git checkout: $platform_root"
  pin="$(tr -d '[:space:]' <"$ENGINE_ROOT/scripts/replication/platform.sha")"
  if [[ ! "$pin" =~ ^[0-9a-f]{40}$ ]]; then
    runtime_fail "platform.sha is not a 40-hex SHA: ${pin:-empty}"
  fi
  head="$(git -C "$platform_root" rev-parse HEAD)"
  if ! git -C "$platform_root" merge-base --is-ancestor "$pin" HEAD; then
    runtime_fail "platform.sha $pin is not an ancestor of $head"
  fi
  printf '%s\n' "$head"
}

run_live() {
  [[ -n "$platform_root" ]] || runtime_fail "--platform-root is required for $case_name"
  local head bin report_dir
  head="$(require_ancestor)"
  if [[ -z "$engine_bin" ]]; then
    (cd "$ENGINE_ROOT" && cargo build --quiet)
    engine_bin="$ENGINE_ROOT/target/debug/comemory"
  fi
  [[ -x "$engine_bin" ]] || runtime_fail "engine binary is missing: $engine_bin"
  "$engine_bin" --version >/dev/null 2>&1 || runtime_fail "engine binary failed --version: $engine_bin"
  command -v node >/dev/null 2>&1 || runtime_fail "node is not on PATH (workerd channel)"
  export COMEMORY_BIN="$engine_bin"
  [[ -d "$platform_root/node_modules/wrangler" || -d "$platform_root/apps/api/node_modules/wrangler" ]] \
    || runtime_fail "wrangler is not installed under $platform_root"
  report_dir="$(mktemp -d)"
  local home
  home="$(mktemp -d)"
  export COMEMORY_POLICY_BIN="$engine_bin"
  export COMEMORY_API=""
  export COMEMORY_API_KEY=""
  export HOME="$home"
  export REPLICATION_CASE="$case_name"
  export REPLICATION_REPORT="$report_dir/replication-report.json"
  export REPLICATION_ENGINE_SHA
  REPLICATION_ENGINE_SHA="$(git -C "$ENGINE_ROOT" rev-parse HEAD)"
  export REPLICATION_PLATFORM_SHA="$head"
  (
    cd "$platform_root/apps/api"
    bun test ./src/routes/__tests__/replication-harness.ts
  )
  bash "$HERE/check-replication-coverage.sh" --report "$REPLICATION_REPORT" \
    --platform-root "$platform_root"
  cat "$REPLICATION_REPORT"
}

case "$case_name" in
  missing-runtime) run_missing_runtime ;;
  teardown) run_teardown ;;
  coverage) run_coverage ;;
  contract) run_contract ;;
  memories) run_memories ;;
  code) run_code ;;
  documents) run_documents ;;
  events) run_events ;;
  exchange) run_exchange ;;
  daemon) run_daemon ;;
  install) run_install ;;
  recovery) run_recovery ;;
  homebrew) run_homebrew ;;
  release-formula) run_release_formula ;;
  baseline | fault-ack | corrupt | credentials | propagation | lost-nudge) run_live ;;
esac
