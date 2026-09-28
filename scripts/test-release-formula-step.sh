#!/usr/bin/env bash
# Runs release.yml's real "Commit formula files" step (publish-homebrew-formula)
# against a real clone of the Homebrew tap, with a real release's formula and
# dist manifest as the plan. It pushes to a local bare remote instead of
# GitHub; that is the only substitution.
# Homebrew tap issue: Falconiere/homebrew-tap#1 (F-1).
#
#   bash scripts/test-release-formula-step.sh [--tap-url URL] [--tap-ref REF] [--release TAG]
#
# When the tap ships scripts/comemory_formula.rb, the published formula must
# pass its `check`, and a formula that breaks the contract must stop the step
# before any commit reaches the remote. Before then, the step must publish
# today's completions-only formula with a ::warning:: annotation.
#
# The step runs `brew update` and `brew style`, so this refuses unless CI=true
# or COMEMORY_DISPOSABLE_ENV=1.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ENGINE_ROOT="$(cd "$HERE/.." && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

tap_url="https://github.com/Falconiere/homebrew-tap.git"
tap_ref="main"
release=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --tap-url) tap_url="$2"; shift 2 ;;
    --tap-ref) tap_ref="$2"; shift 2 ;;
    --release) release="$2"; shift 2 ;;
    *) die "release-formula" "unknown argument: $1" ;;
  esac
done

if [[ "${CI:-}" != true && "${COMEMORY_DISPOSABLE_ENV:-}" != 1 ]]; then
  log_err "release-formula" "refusing: the release step runs brew update; set CI=true or COMEMORY_DISPOSABLE_ENV=1 on a disposable machine"
  exit 2
fi
for cmd in brew gh git jq ruby; do require_cmd "$cmd"; done

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export GIT_CONFIG_GLOBAL="$work/gitconfig"   # the step writes `git config --global`
touch "$GIT_CONFIG_GLOBAL"

# The step body, exactly as release.yml runs it.
ruby -ryaml -e '
  job = YAML.load_file(ARGV[0]).fetch("jobs").fetch("publish-homebrew-formula")
  step = job.fetch("steps").find { |s| s["name"] == "Commit formula files" }
  abort "release.yml: no \"Commit formula files\" step" unless step
  print step.fetch("run")' "$ENGINE_ROOT/.github/workflows/release.yml" >"$work/step.sh"

gh release download ${release:+"$release"} --repo Falconiere/comemory \
  --pattern comemory.rb --pattern dist-manifest.json --dir "$work/release"
version="$(jq -r '.releases[] | select(.app_name == "comemory") | .app_version' "$work/release/dist-manifest.json")"

# fresh_tap NAME: a clone of the tap whose `origin` is a local bare repository.
fresh_tap() {
  local name="$1"
  git clone -q --branch "$tap_ref" "$tap_url" "$work/$name"
  git init -q --bare "$work/$name.git"
  git -C "$work/$name" remote set-url origin "$work/$name.git"
  git -C "$work/$name" push -q -u origin "HEAD:refs/heads/$tap_ref"
}

# run_step NAME FORMULA: the download-artifact step drops the formula into Formula/, then the step runs.
run_step() {
  local name="$1" formula="$2"
  cp "$formula" "$work/$name/Formula/comemory.rb"
  (
    cd "$work/$name"
    PLAN="$(cat "$work/release/dist-manifest.json")" GITHUB_USER="axo bot" GITHUB_EMAIL="admin+bot@axo.dev" \
      bash --noprofile --norc -eo pipefail "$work/step.sh"
  ) >"$work/$name.log" 2>&1
}

remote_head() { git -C "$work/$1.git" rev-parse "refs/heads/$tap_ref"; }

fresh_tap published
before="$(remote_head published)"
if ! run_step published "$work/release/comemory.rb"; then
  cat "$work/published.log"
  die "release-formula" "the release step failed on the real v$version formula"
fi
after="$(remote_head published)"
[[ "$after" != "$before" ]] || die "release-formula" "the release step pushed nothing"
[[ "$(git -C "$work/published.git" log -1 --format=%s "$after")" == "comemory $version" ]] \
  || die "release-formula" "unexpected commit subject on the remote"
git -C "$work/published.git" show "$after:Formula/comemory.rb" >"$work/published.rb"
completions="$(grep -c 'generate_completions_from_executable(' "$work/published.rb" || true)"
[[ "$completions" == 1 ]] || die "release-formula" "published formula has $completions completions blocks"

if [[ -f "$work/published/scripts/comemory_formula.rb" ]]; then
  ruby "$work/published/scripts/comemory_formula.rb" check "$work/published.rb" \
    || die "release-formula" "the published formula breaks the tap's lifecycle contract"
  ! grep -q '::warning::tap has no scripts/comemory_formula.rb' "$work/published.log" \
    || die "release-formula" "warned about a missing script the tap has"
  log_ok "release-formula" "v$version published with completions and the lifecycle caveats"

  # A cargo-dist shape the contract rejects must stop the step before its commit.
  fresh_tap broken
  ruby -e 'p = ARGV[0]; s = File.read(p); s.sub!(/^  def install$/, "  def post_install\n    system bin/\"comemory\", \"--version\"\n  end\n\n  def install") || abort("no install method"); File.write(p, s)' \
    "$work/release/comemory.rb"
  before="$(remote_head broken)"
  if run_step broken "$work/release/comemory.rb"; then
    cat "$work/broken.log"
    die "release-formula" "the release step accepted a formula with post_install"
  fi
  grep -q 'contract: post_install is forbidden' "$work/broken.log" \
    || { cat "$work/broken.log"; die "release-formula" "the step failed for another reason"; }
  [[ "$(remote_head broken)" == "$before" ]] || die "release-formula" "a rejected formula reached the remote"
  log_ok "release-formula" "a contract-breaking formula stopped the step before its commit"
else
  grep -q '::warning::tap has no scripts/comemory_formula.rb' "$work/published.log" \
    || die "release-formula" "published without the lifecycle script and without a warning"
  log_ok "release-formula" "v$version published with completions and a warning (tap has no lifecycle script yet)"
fi
