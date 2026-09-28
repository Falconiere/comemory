#!/usr/bin/env bash
# check-all gate: docs/guides/http-api.md must document every live /api/v1
# route in comemory::serve::routes::table(). Presence-only (the doc is
# hand-authored, not regenerated) — see tests/http_api_docs_check.rs.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE/.."
exec cargo nextest run --test http_api_docs_check
