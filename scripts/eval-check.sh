#!/usr/bin/env bash
# Score the lexical retrieval pipeline against the frozen golden corpus.
# Seeds a throwaway data dir from tests/golden/corpus/*.md via `comemory
# rebuild` (real markdown -> real SQLite), runs `comemory eval`, and enforces
# tests/golden/floor.toml when present (else report-only). Distinguishes an
# eval *error* (config bug) from an eval *below floor* (regression).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"
cd "$PROJECT_ROOT"

require_cmd jq "brew install jq"

corpus_dir="$PROJECT_ROOT/tests/golden/corpus"
golden="$PROJECT_ROOT/tests/golden/memory.yml"
floor="$PROJECT_ROOT/tests/golden/floor.toml"

# The golden corpus is a future Phase-1b artifact. Until it lands, skip the eval
# gate as a no-op rather than failing CI.
if [[ ! -d "$corpus_dir" ]] || ! compgen -G "$corpus_dir/*.md" >/dev/null; then
  log_info "eval-check" "no golden corpus yet — skipping eval gate"
  log_ok "eval-check" "skipped (no golden corpus)"
  exit 0
fi

[[ -f "$golden" ]]     || die "eval-check" "missing golden file: $golden"

# The binary is not on PATH in a fresh checkout/CI; build and use the artifact
# directly (mirrors scripts/e2e.sh).
run_cargo build --release --quiet
bin="$PROJECT_ROOT/target/release/comemory"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/.comemory/memories"
cp "$corpus_dir"/*.md "$work/.comemory/memories/"
export COMEMORY_DATA_DIR="$work/.comemory"

"$bin" rebuild >/dev/null

# Preflight: every golden `relevant` id must resolve to a corpus file, or recall
# drops for the wrong reason (corpus <-> golden drift).
corpus_ids="$(find "$work/.comemory/memories" -name '*.md' -exec basename {} \; \
  | sed -E 's/^([0-9a-f]+)-.*/\1/' | sort -u)"
missing=""
while IFS= read -r id; do
  [[ -z "$id" ]] && continue
  grep -qx "$id" <<<"$corpus_ids" || missing+="$id "
done < <(grep -oE '[0-9a-f]{8}' "$golden" | sort -u)
[[ -z "$missing" ]] || die "eval-check" "golden ids absent from corpus: $missing"

# Capture eval exit status separately so a config error is not mislabelled as a
# floor miss (golden::resolve returns an error on zero pairs).
set +e
report="$("$bin" --json eval --golden "$golden" --golden-only --k 3)"
status=$?
set -e
[[ $status -eq 0 ]] \
  || die "eval-check" "eval errored (exit $status) — config bug, not a floor miss"

recall="$(jq -e '.recall_at_k' <<<"$report")"
mrr="$(jq -e '.mrr' <<<"$report")"
log_info "eval-check" "recall@3=$recall mrr=$mrr"

if [[ -f "$floor" ]]; then
  min_recall="$(grep -E '^recall_at_k' "$floor" | grep -oE '[0-9.]+' | head -1)"
  min_mrr="$(grep -E '^mrr' "$floor" | grep -oE '[0-9.]+' | head -1)"
  awk -v v="$recall" -v f="$min_recall" 'BEGIN{exit !(v+0 >= f+0)}' \
    || die "eval-check" "recall@3 $recall < floor $min_recall"
  awk -v v="$mrr" -v f="$min_mrr" 'BEGIN{exit !(v+0 >= f+0)}' \
    || die "eval-check" "mrr $mrr < floor $min_mrr"
  log_ok "eval-check" "recall@3>=$min_recall mrr>=$min_mrr"
else
  log_info "eval-check" "no tests/golden/floor.toml — report-only"
  log_ok "eval-check" "report-only (write tests/golden/floor.toml to enforce)"
fi
