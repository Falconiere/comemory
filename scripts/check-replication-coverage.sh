#!/usr/bin/env bash
# Pair every replication AC with a case the runner actually dispatches.
# Does not boot the platform. Exit 1 on a dangling AC, an unknown case,
# a runner arm the manifest does not claim, or a report that ran nothing.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

manifest="$HERE/replication/coverage.json"
runner="$HERE/test-replication-e2e.sh"
platform_root=""
report=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --manifest)
      manifest="$2"
      shift 2
      ;;
    --platform-root)
      platform_root="$2"
      shift 2
      ;;
    --report)
      report="$2"
      shift 2
      ;;
    *)
      die "replication-coverage" "unknown argument: $1"
      ;;
  esac
done

[[ -f "$manifest" ]] || die "replication-coverage" "missing manifest: $manifest"
[[ -f "$runner" ]] || die "replication-coverage" "missing runner: $runner"

python3 - "$manifest" "$runner" "$platform_root" <<'PY'
import json, pathlib, re, sys
manifest_path, runner_path, platform_root = sys.argv[1:]
text = pathlib.Path(manifest_path).read_text()
keys = re.findall(r'"(G-\d+)"\s*:', text)
dupes = sorted({key for key in keys if keys.count(key) > 1})
if dupes:
    sys.exit(f"replication-coverage: duplicate AC keys: {', '.join(dupes)}")
data = json.loads(text)
if data.get("version") != 1:
    sys.exit("replication-coverage: manifest version must be 1")
acs = data.get("acs")
if not isinstance(acs, dict):
    sys.exit("replication-coverage: manifest acs must be an object")
required = [f"G-{n}" for n in range(1, 8)]
if sorted(acs) != sorted(required):
    sys.exit(f"replication-coverage: AC keys must be {required}, found {sorted(acs)}")
runner = pathlib.Path(runner_path).read_text()
match = re.search(r"^CASES=\(([^)]*)\)", runner, re.M)
if not match:
    sys.exit("replication-coverage: runner is missing CASES=(...)")
runner_cases = match.group(1).split()
claimed = []
for ac, cases in acs.items():
    if not isinstance(cases, list) or not cases:
        sys.exit(f"replication-coverage: {ac} has no case")
    for case in cases:
        if case not in runner_cases:
            sys.exit(f"replication-coverage: {ac} names unknown case {case}")
        claimed.append(case)
missing = sorted(set(runner_cases) - set(claimed))
if missing:
    sys.exit(f"replication-coverage: runner cases not claimed: {', '.join(missing)}")
if platform_root:
    harness = pathlib.Path(platform_root) / "apps/api/src/routes/__tests__/replication-harness.ts"
    if not harness.is_file():
        sys.exit(f"replication-coverage: missing harness: {harness}")
    body = harness.read_text()
    for case in sorted(set(claimed)):
        if case not in body and case not in {"coverage", "teardown", "missing-runtime"}:
            sys.exit(f"replication-coverage: harness does not mention {case}")
PY

if [[ -n "$report" ]]; then
  [[ -f "$report" ]] || die "replication-coverage" "missing report: $report"
  python3 - "$report" <<'PY'
import json, sys
report = json.load(open(sys.argv[1]))
ran = report.get("tests_ran", 0)
skipped = report.get("skipped", 0)
if ran == 0 or skipped:
    sys.exit(f"replication-coverage: tests_ran={ran} skipped={skipped}")
PY
fi
