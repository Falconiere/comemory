#!/usr/bin/env bash
# Helpers for scripts/build-perf.sh — sourced, not executed.
# Depends on scripts/lib/common.sh being sourced first.

# Return current epoch seconds with millisecond precision (portable: macOS + Linux).
perf_now_ms() {
  python3 -c 'import time; print(int(time.time()*1000))'
}

# Run a single command, print wall-clock seconds (3 decimals) to stdout.
# Args: <label> <cmd> [<args>...]
# Returns non-zero if the timed command fails.
perf_time_once() {
  local label="$1"; shift
  local start end rc
  start="$(perf_now_ms)"
  "$@" >/dev/null; rc=$?
  if [ "$rc" -ne 0 ]; then
    printf "perf_time_once: command failed (rc=%s, label=%s): %s\n" \
      "$rc" "$label" "$*" >&2
    return "$rc"
  fi
  end="$(perf_now_ms)"
  awk -v s="$start" -v e="$end" 'BEGIN { printf "%.3f", (e - s) / 1000.0 }'
}

# Build a single, shell-safely-escaped command string from positional args.
# Used to pass varargs to tools (like hyperfine) that take one shell string.
perf_shell_escape() {
  local s=""
  local arg
  for arg in "$@"; do
    s+="$(printf '%q' "$arg") "
  done
  printf "%s" "${s% }"
}

# Run a command N times via hyperfine if present; else fall back to perf_time_once.
# Args: <label> <runs> <cmd> [<args>...]
# Emits "p50_s p95_s" on stdout. With fallback, p95 == p50.
perf_time_runs() {
  local label="$1"; local runs="$2"; shift 2
  if command -v hyperfine >/dev/null 2>&1; then
    local json cmd_str p50 p95
    json="$(mktemp)"
    cmd_str="$(perf_shell_escape "$@")"
    hyperfine --warmup 1 --runs "$runs" --export-json "$json" \
      --command-name "$label" "$cmd_str" >/dev/null
    p50="$(jq -r '.results[0].median' "$json")"
    p95="$(jq -r '.results[0].max' "$json")"
    rm -f "$json"
    awk -v p50="$p50" -v p95="$p95" 'BEGIN { printf "%.3f %.3f", p50, p95 }'
  else
    local single
    single="$(perf_time_once "$label" "$@")"
    printf "%s %s" "$single" "$single"
  fi
}

# Extract top-N crate-unit durations from a cargo-timings JSON file.
# Args: <timings.json> <n>
# Emits a JSON array: [{name, version, duration_s}, ...]
perf_top_crates() {
  local file="$1"; local n="$2"
  jq -c --argjson n "$n" '
    [ .invocations[]? | select(.target?) |
      { name: .package_id // .target.name,
        version: (.package_id // "") | capture("@(?<v>[^@]+)$")?.v // "",
        duration_s: (.duration | tonumber | (.*1000|round)/1000) } ]
    | sort_by(-.duration_s) | .[0:$n]
  ' "$file"
}
