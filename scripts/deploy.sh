#!/usr/bin/env bash
# scripts/deploy.sh — publish a comemory release from this machine.
#
# GitHub Actions are disabled, so a pushed tag builds nothing. This script does
# what release.yml + release-finalize.yml did: bump, build the three dist
# targets (macOS natively, Linux in Docker), package, verify, then push, create
# the GitHub Release, update the Homebrew tap and smoke-install it.
# Nothing is pushed until every archive is built and verified.
#
# Usage:
#   bash scripts/deploy.sh --tag vX.Y.Z --change-log <text> [--dry-run]
#   bash scripts/deploy.sh --resume vX.Y.Z
#
# Releases a stable vX.Y.Z from an up-to-date main: full local gates, the bump
# commit lands on main, the release is marked latest, the tap is updated.
#   --dry-run    bump, builds, archives and checks only; no gates, no commit on
#                any branch, no push, no release, no tap
#   --resume     finish a pushed vX.Y.Z whose release, tap, latest mark or smoke
#                install failed: reuses target/deploy/vX.Y.Z/, skips each step
#                already done, never rebuilds; only for the newest release tag
#
# Exit codes: 0 done · 1 refused or failed before any push (with --resume:
# refused before any write) · 2 failed after the push (prints the --resume
# command) · 64 usage error.
# Runbook: docs/release.md §3b.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"
# shellcheck source=scripts/lib/deploy/prepare.sh
source "$HERE/lib/deploy/prepare.sh"
# shellcheck source=scripts/lib/deploy/build.sh
source "$HERE/lib/deploy/build.sh"
# shellcheck source=scripts/lib/deploy/publish.sh
source "$HERE/lib/deploy/publish.sh"
# shellcheck source=scripts/lib/deploy/resume.sh
source "$HERE/lib/deploy/resume.sh"

usage() {
  log_err deploy "$1"
  printf 'usage: bash scripts/deploy.sh --tag vX.Y.Z --change-log <text> [--dry-run]\n' >&2
  printf '       bash scripts/deploy.sh --resume vX.Y.Z\n' >&2
  exit 64
}

DEPLOY_TAG="" DEPLOY_NOTES="" DEPLOY_DRY_RUN=0 DEPLOY_RESUME=""
while (($#)); do
  case "$1" in
    --tag | --change-log | --resume)
      [[ $# -ge 2 ]] || usage "$1 needs a value"
      case "$1" in
        --tag) DEPLOY_TAG="$2" ;;
        --change-log) DEPLOY_NOTES="$2" ;;
        *) DEPLOY_RESUME="$2" ;;
      esac
      shift 2 ;;
    --dry-run) DEPLOY_DRY_RUN=1; shift ;;
    -h | --help) awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"; exit 0 ;;
    *) usage "unknown argument: $1" ;;
  esac
done

cd "$PROJECT_ROOT"
if [[ -n "$DEPLOY_RESUME" ]]; then
  if [[ -n "$DEPLOY_TAG$DEPLOY_NOTES" ]] || ((DEPLOY_DRY_RUN)); then
    usage "--resume excludes --tag, --change-log and --dry-run"
  fi
  DEPLOY_TAG="$DEPLOY_RESUME"
  deploy_resume_preflight
  deploy_finish
  exit 0
fi
deploy_validate_args
deploy_preflight
trap deploy_cleanup EXIT
deploy_prepare_bump
deploy_build_all
deploy_package
deploy_verify_assets
deploy_stage_formula
if ((DEPLOY_DRY_RUN)); then
  deploy_print_skipped
  log_ok deploy:dry-run "assets verified; nothing was committed to a branch or pushed"
  printf '%s\n' "$DEPLOY_ASSETS"
  exit 0
fi
deploy_publish
