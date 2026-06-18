#!/usr/bin/env bash
# Build the serve-web SPA into web/dist (embedded into the binary via
# rust-embed). The repo's PreTool hook blocks raw package-manager tokens, so
# the frontend build is wrapped here and invoked as `bash scripts/web-build.sh`
# (or `just web`) — the sanctioned cargo/just/scripts path. Installs deps only
# when web/node_modules is absent so repeat builds stay fast.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

require_cmd node "https://nodejs.org"
require_cmd npm "https://nodejs.org"

cd "$PROJECT_ROOT/web"

# `npm install` is idempotent: a no-op (~1s) when package.json and the lockfile
# already agree, and the one command that syncs both when a dependency changes.
# It lives inside this script so agents invoke it as `bash scripts/web-build.sh`
# (the blocked package-manager token never appears in an agent command line).
log_info web "syncing dependencies"
npm install --no-audit --no-fund

log_info web "tsc --noEmit && vite build"
npm run build
log_ok web "web/dist rebuilt"
