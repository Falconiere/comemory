#!/usr/bin/env bash
# check-all gate. The checker itself stays scripts/check-replication-coverage.sh
# so a caller can pass --manifest, --report, and --platform-root.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
exec bash "$HERE/check-replication-coverage.sh"
