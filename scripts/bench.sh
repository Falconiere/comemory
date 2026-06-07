#!/usr/bin/env bash
# Reproducible bench runner. Pins thread count and warmup so successive runs
# are comparable. Output lands in docs/bench/latest.md plus the criterion
# HTML report under target/criterion/.

set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

OUT="docs/bench"
mkdir -p "$OUT"

{
  echo "# comemory bench results"
  echo ""
  echo "Rust: $(rustc --version)"
  echo "Host: $(uname -m) $(uname -s)"
  echo "Run at: $(date -u +%FT%TZ)"
  echo ""
  echo '```'
} > "$OUT/latest.md"

RUST_LOG=warn cargo bench --all-features 2>&1 | tee -a "$OUT/latest.md"

echo '```' >> "$OUT/latest.md"

echo "wrote $OUT/latest.md"
