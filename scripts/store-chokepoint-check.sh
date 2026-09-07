#!/usr/bin/env bash
# Ratchets src/store/ toward being the ONLY production module that imports
# rusqlite (docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md).
# Two checks:
#
# Check A — a TWO-SIDED file-count ratchet, unlike scripts/dup-check.sh's
# one-sided one. Production means: every .rs file under src/ (tracked via
# `git ls-files`, plus untracked-but-not-ignored via `git ls-files --others
# --exclude-standard` so the gate still catches a brand-new scratch file),
# excluding src/store.rs and any path under src/store/ (together they ARE the
# store module — Binding Rule 2), excluding any path containing
# /tests/, and excluding src/errors.rs (its `Error::Sqlite(#[from]
# rusqlite::Error)` is a documented permanent exception — see the design's
# Non-Goal 1). A file counts if it contains the bare string "rusqlite"
# anywhere, including a doc comment — deliberate, since prose about the
# driver outside store/ is itself a sign the abstraction moved.
#
# The count is compared against store-leak-baseline.txt (repo root,
# tracked, a single integer). Unlike dup-check.sh, BOTH directions fail:
# higher means a new leak was introduced; lower means the baseline is
# stale and must be lowered to lock in the improvement. This is what
# forces every PR in the 13-PR series to record its own reduction rather
# than merely not regress — the baseline reaches 0 and then never moves.
#
# Check B — no store/ helper leaks a driver cursor type through its return
# position. `pub fn $F(...) -> $R { ... }` is scanned across src/store.rs
# and src/store/, and $R is rejected if it names Statement, Rows, or
# rusqlite::Error (Connection is fine — store::connection::open returns it
# by design, and src/store.rs re-exports it for every other module to
# name). The pattern requires a real function body to parse as valid Rust
# (a bodyless `pub fn` is only legal inside a trait), so it MISSES a
# generic signature such as `pub fn open<P: AsRef<Path>>(...)` — a
# backstop, not a proof; review still catches what this scan structurally
# cannot see. When ast-grep is not installed, check B is skipped with a
# warning (mirroring how dup-check.sh degrades when similarity-rs is
# absent) — check A never skips.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"
cd "$PROJECT_ROOT"

STEP="store-chokepoint-check"
require_cmd git

BASELINE_FILE="$PROJECT_ROOT/store-leak-baseline.txt"
if [[ ! -f "$BASELINE_FILE" ]]; then
  log_err "$STEP" "missing $BASELINE_FILE (baseline ratchet file)"
  exit 1
fi
baseline_count="$(head -n 1 "$BASELINE_FILE" | tr -d '[:space:]')"
if ! [[ "$baseline_count" =~ ^[0-9]+$ ]]; then
  log_err "$STEP" "$BASELINE_FILE must contain a single integer count"
  exit 1
fi

mapfile -t all_files < <(
  { git ls-files 'src/**/*.rs' 'src/*.rs'; \
    git ls-files --others --exclude-standard 'src/**/*.rs' 'src/*.rs'; } | sort -u
)

offenders=()
for f in "${all_files[@]}"; do
  [[ -f "$f" ]] || continue
  # src/store.rs is the store module's ROOT file, not a sibling of it:
  # CLAUDE.md Binding Rule 2 — "src/store.rs declares mod migrate;,
  # src/store/migrate.rs holds it". The two together ARE the store layer,
  # and AC-9 has src/store.rs permanently re-export Connection, so it can
  # never be a leak.
  [[ "$f" == src/store/* || "$f" == src/store.rs ]] && continue
  [[ "$f" == *"/tests/"* ]] && continue
  [[ "$f" == "src/errors.rs" ]] && continue
  # Word-anchored so an identifier that merely CONTAINS the crate name
  # (`not_rusqlite`) is not a hit. A mention in prose or a doc comment IS
  # still a hit, deliberately: documentation about the driver outside the
  # store module is itself evidence the abstraction moved.
  # `libsqlite3-sys` is matched too — it is a DIRECT dependency (the FTS5
  # tokenizer FFI), so confining only `rusqlite` would leave a second door open.
  if grep -qE '\b(rusqlite|libsqlite3_sys|libsqlite3-sys)\b' "$f" 2>/dev/null; then
    offenders+=("$f")
  fi
done

current_count="${#offenders[@]}"

check_a_failed=0
if (( current_count > baseline_count )); then
  log_err "$STEP" \
    "rusqlite leak count $current_count exceeds baseline $baseline_count — a new leak was introduced outside src/store/. Offending files:"
  for f in "${offenders[@]}"; do
    printf '  %s\n' "$f" >&2
  done
  check_a_failed=1
elif (( current_count < baseline_count )); then
  log_err "$STEP" \
    "leak reduced to $current_count — lower store-leak-baseline.txt to $current_count"
  check_a_failed=1
else
  log_ok "$STEP" "rusqlite leak count $current_count matches baseline $baseline_count"
fi

check_b_failed=0
scan_err="$(mktemp)"
trap 'rm -f "$scan_err"' EXIT
if ! command -v ast-grep >/dev/null 2>&1; then
  log_info "$STEP" "ast-grep not installed; skipping check B (return-type scan)"
else
  require_cmd jq
  # A `kind: function_item` rule, NOT a `pub fn ... -> $R { }` pattern. The
  # pattern form only matches the plainest signature: it silently skips
  # generics (`pub fn open<P: AsRef<Path>>`), `where` clauses, and attributed
  # functions — i.e. it would pass exactly the shapes most likely to leak.
  # The kind rule matches every function regardless of shape; `startswith("pub")`
  # then keeps only the ones that can actually cross the module boundary
  # (attributes are sibling nodes, so `#[inline] pub fn` still starts "pub").
  ast_status=0
  json_out="$(ast-grep scan --inline-rules 'id: store-return-types
language: rust
rule:
  kind: function_item
  has: {field: return_type, pattern: $R}' \
    --json src/store.rs src/store/ 2>"$scan_err")" || ast_status=$?
  if (( ast_status != 0 )); then
    # Never swallow this. A crashed or wrongly invoked scan still prints `[]`,
    # indistinguishable from "ran fine, found nothing" — the failure mode that
    # left dup-check.sh inert twice (docs/dup-debt.md). Fail loudly instead.
    log_err "$STEP" "ast-grep failed (exit $ast_status); check B could not run:"
    sed 's/^/  /' "$scan_err" >&2
    check_b_failed=1
  else
    # Word-anchored: `StatementCache` and `RowsAffected` are legitimate return
    # types and must not trip the gate. `rusqlite::Error` is matched literally.
    mapfile -t bad_returns < <(
      printf '%s' "${json_out:-[]}" \
        | jq -r '.[]
            | select(.text | startswith("pub"))
            | select(.metaVariables.single.R.text
                | test("\\b(Statement|Rows)\\b|rusqlite::Error"))
            | "\(.file): \(.metaVariables.single.R.text)"'
    )
    if (( ${#bad_returns[@]} > 0 )); then
      log_err "$STEP" "store/ function(s) return a driver cursor type:"
      for line in "${bad_returns[@]}"; do
        printf '  %s\n' "$line" >&2
      done
      check_b_failed=1
    fi
  fi
  if (( check_b_failed == 0 )); then
    log_ok "$STEP" "no store/ function returns Statement, Rows, or rusqlite::Error"
  fi
fi

if (( check_a_failed || check_b_failed )); then
  exit 1
fi
log_ok "$STEP" "gate passed"
