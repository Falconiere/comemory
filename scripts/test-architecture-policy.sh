#!/usr/bin/env bash
# Validate the migration contract and exercise corrupt copies of its real inputs.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
POLICY=scripts/architecture-policy.json
INVENTORY=docs/designs/2026-09-17-domain-first-migration-inventory.md

fail() { echo "architecture-policy: $*" >&2; exit 1; }

for tool in rg jq ast-grep; do
  command -v "$tool" >/dev/null 2>&1 || fail "missing tool: $tool"
done

# shellcheck source=scripts/lib/architecture-inventory.sh
source "$ROOT/scripts/lib/architecture-inventory.sh"
validate() { validate_inventory; }

for required in "$POLICY" "$INVENTORY"; do
  [[ -f "$required" ]] || fail "missing contract input: $required"
done

if [[ ${1:-} == --validate ]]; then
  shift
  while (($#)); do
    case $1 in
      --policy) POLICY=$2; shift 2 ;;
      --inventory) INVENTORY=$2; shift 2 ;;
      *) fail "unknown argument: $1" ;;
    esac
  done
  validate
  exit
fi

TASK_TMP=$(mktemp -d)
trap 'rm -rf "$TASK_TMP"' EXIT
CHECK="$ROOT/scripts/test-architecture-policy.sh"
assert_fails() {
  local label=$1 expected=$2
  shift 2
  if bash "$CHECK" --validate "$@" >"$TASK_TMP/result" 2>&1; then
    fail "$label was accepted"
  fi
  grep -Fq "$expected" "$TASK_TMP/result" || {
    cat "$TASK_TMP/result" >&2
    fail "$label failed for the wrong reason"
  }
  echo "PASS: $label"
}

bash "$CHECK" --validate
assert_missing_rg_is_clear() {
  local status=0
  PATH=/usr/bin:/bin /bin/bash "$CHECK" --validate >"$TASK_TMP/missing-rg" 2>&1 || status=$?
  [[ $status != 0 ]] && grep -Fq 'missing tool: rg' "$TASK_TMP/missing-rg" || {
    cat "$TASK_TMP/missing-rg" >&2
    fail 'missing rg was not reported clearly'
  }
  echo 'PASS: missing rg preflight'
}
assert_missing_rg_is_clear
jq 'del(.setup_runtime_dependencies)' "$POLICY" >"$TASK_TMP/no-setup.json"
assert_fails 'missing setup runtime dependencies' 'invalid policy or inventory metadata' --policy "$TASK_TMP/no-setup.json"
jq '.setup_runtime_dependencies = null' "$POLICY" >"$TASK_TMP/null-setup.json"
assert_fails 'null setup runtime dependencies' 'invalid policy or inventory metadata' --policy "$TASK_TMP/null-setup.json"
jq 'del(.setup_runtime_dependencies[0].target)' "$POLICY" >"$TASK_TMP/setup-target.json"
assert_fails 'missing setup target' 'invalid setup runtime dependency' --policy "$TASK_TMP/setup-target.json"
jq '.passive_store_models += [.passive_store_models[0]]' "$POLICY" >"$TASK_TMP/duplicate-model.json"
assert_fails 'duplicate passive model' 'invalid policy edge' --policy "$TASK_TMP/duplicate-model.json"
jq '.staged_top_level_dirs += ["unexpected"]' "$POLICY" >"$TASK_TMP/unexpected-dir.json"
assert_fails 'unexpected staged directory' 'invalid policy or inventory metadata' --policy "$TASK_TMP/unexpected-dir.json"
jq '.staged_root_modules += ["unexpected"]' "$POLICY" >"$TASK_TMP/unexpected-root.json"
assert_fails 'unexpected staged root' 'invalid policy or inventory metadata' --policy "$TASK_TMP/unexpected-root.json"
jq '.domains += ["unexpected"]' "$POLICY" >"$TASK_TMP/eleventh-domain.json"
assert_fails 'eleventh domain' 'invalid policy or inventory metadata' --policy "$TASK_TMP/eleventh-domain.json"
sed '/^| src\/domains\/memories\/save.rs |/d' "$INVENTORY" >"$TASK_TMP/missing.md"
assert_fails 'missing production row' 'inventory coverage' --inventory "$TASK_TMP/missing.md"
assert_gate_rejects() {
  local inventory=$1 expected=$2 scope status
  for scope in repository scoped; do
    args=()
    if [[ $scope == scoped ]]; then args=(--file src/lib.rs); fi
    status=0
    bash scripts/architecture-check.sh --inventory "$inventory" "${args[@]+${args[@]}}" >"$TASK_TMP/result" 2>&1 || status=$?
    [[ $status == 1 ]] && grep -Fq "$expected" "$TASK_TMP/result" || {
      cat "$TASK_TMP/result" >&2
      fail "$scope production checker failed to reject invalid inventory (status $status)"
    }
  done
}
grep '^| src/lib.rs |' "$INVENTORY" >"$TASK_TMP/only-lib.md"
assert_gate_rejects "$TASK_TMP/only-lib.md" 'inventory coverage mismatch'
cp "$INVENTORY" "$TASK_TMP/duplicate.md"
grep '^| src/domains/memories/save.rs |' "$INVENTORY" >>"$TASK_TMP/duplicate.md"
assert_fails 'duplicate production row' 'duplicate inventory row' --inventory "$TASK_TMP/duplicate.md"
# `legacy_edges` empties out as its last delivery exemption clears (#170 was the
# last one), and a mutation of `.legacy_edges[0]` on an empty array is a no-op
# that makes the negative case pass while asserting nothing. Each fixture below
# therefore seeds one well-formed entry when the real allowlist is empty and
# breaks that, so the case keeps asserting whatever the policy currently holds.
SEED_EDGE='.legacy_edges = (if (.legacy_edges | length) > 0 then .legacy_edges else
  [{source: "src/domains/graph/view.rs", target: "crate::output::graph",
    class: "delivery", issue: "#170"}] end)'
jq "$SEED_EDGE"' | .legacy_edges[0].issue = "#999"' "$POLICY" >"$TASK_TMP/issue.json"
assert_fails 'unknown removal issue' 'invalid policy edge' --policy "$TASK_TMP/issue.json"
jq "$SEED_EDGE"' | del(.legacy_edges[0].target)' "$POLICY" >"$TASK_TMP/target.json"
assert_fails 'missing exact target' 'invalid policy edge' --policy "$TASK_TMP/target.json"
jq "$SEED_EDGE"' | .legacy_edges[0].target = "crate::cli::not_present"' "$POLICY" >"$TASK_TMP/stale.json"
assert_fails 'stale allowlisted target' 'absent policy edge' --policy "$TASK_TMP/stale.json"
sed '/^| src\/api\/search.rs |/s/domains::retrieval/domains::code/' "$INVENTORY" >"$TASK_TMP/owner.md"
assert_fails 'wrong API owner' 'API ownership mismatch' --inventory "$TASK_TMP/owner.md"
assert_gate_rejects "$TASK_TMP/owner.md" 'API ownership mismatch'
sed '/^| src\/domains\/memories\/save.rs |/s@src/domains/memories/save.rs@src/domains/memories/delete.rs@2' "$INVENTORY" >"$TASK_TMP/target.md"
assert_fails 'duplicate migration target' 'duplicate inventory target' --inventory "$TASK_TMP/target.md"
sed '/^| src\/domains\/memories\/save.rs |/s@src/domains/memories/save.rs@none@2' "$INVENTORY" >"$TASK_TMP/no-target.md"
assert_fails 'missing inventory target' 'invalid policy or inventory metadata' --inventory "$TASK_TMP/no-target.md"
jq "$SEED_EDGE"' | .legacy_edges += [.legacy_edges[0]]' "$POLICY" >"$TASK_TMP/duplicate.json"
assert_fails 'duplicate policy edge' 'invalid policy edge' --policy "$TASK_TMP/duplicate.json"
jq '.store_callbacks[0].target = "crate::domains::graph::cross_link::absent"' "$POLICY" >"$TASK_TMP/callback.json"
assert_fails 'stale store callback' 'absent policy edge' --policy "$TASK_TMP/callback.json"
jq '.passive_store_models[0].target = "crate::domains::memories::Absent"' "$POLICY" >"$TASK_TMP/model.json"
assert_fails 'stale passive model' 'absent policy edge' --policy "$TASK_TMP/model.json"
sed '/^| src\/domains\/memories.rs |/s/comemory::domains::memories; crate-root-alias/private/' "$INVENTORY" >"$TASK_TMP/public.md"
assert_fails 'missing public compatibility choice' 'public path mismatch' --inventory "$TASK_TMP/public.md"
assert_gate_rejects "$TASK_TMP/public.md" 'public path mismatch'
sed '/^| src\/domains\/capture\/redact.rs |/s@src/domains/capture/rules.toml@none@' "$INVENTORY" >"$TASK_TMP/asset.md"
assert_fails 'missing compile-time asset' 'inventory asset/bridge mismatch' --inventory "$TASK_TMP/asset.md"
assert_gate_rejects "$TASK_TMP/asset.md" 'inventory asset/bridge mismatch'
sed '/^| src\/utilities\/context.rs |/s@src/utilities/tests/context.rs@none@' "$INVENTORY" >"$TASK_TMP/bridge.md"
assert_gate_rejects "$TASK_TMP/bridge.md" 'inventory asset/bridge mismatch'

require_guidance() {
  local file=$1 expected=$2 label=$3
  rg -Fq -- "$expected" "$file" || fail "missing $label"
}

require_guidance AGENTS.md 'D3 — `barrelNames: ["mod.rs"]`' 'D3 barrel policy guidance'
require_guidance AGENTS.md 'D4 — `src.nested` replaces the starter map' 'D4 nested policy guidance'
while IFS=$'\t' read -r scope children; do
  require_guidance AGENTS.md "- \`$scope\`: $children" "D4 $scope nested policy guidance"
done < <(jq -r '.src.nested | to_entries[] |
  [.key, (.value | map("`" + . + "`") | join(", "))] | @tsv' guardrails.config.json)
readme_count=$(jq '.src.requireReadme | length' guardrails.config.json)
require_guidance AGENTS.md "D5 — \`src.requireReadme\` replaces the pinned single-\`domains\` rule during staging with exactly $readme_count folders" 'D5 README count guidance'
require_guidance AGENTS.md 'names folders, not files' 'D5 folder policy guidance'
require_guidance AGENTS.md 'single-file module is listed in its parent folder' 'D5 single-file policy guidance'
require_guidance docs/designs/2026-09-17-domain-first-migration.md '[inventory](2026-09-17-domain-first-migration-inventory.md)' 'inventory design link'
require_guidance docs/designs/2026-09-17-domain-first-migration.md '[#162](https://github.com/Falconiere/comemory/issues/162)' '#162 baseline link'
require_guidance docs/designs/2026-09-17-domain-first-migration.md '[#163](https://github.com/Falconiere/comemory/issues/163)' '#163 baseline link'
echo 'PASS: architecture policy and inventory'
