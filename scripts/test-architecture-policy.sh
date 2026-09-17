#!/usr/bin/env bash
# Validate the migration contract and exercise corrupt copies of its real inputs.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
POLICY=scripts/architecture-policy.json
INVENTORY=docs/designs/2026-09-17-domain-first-migration-inventory.md

fail() { echo "architecture-policy: $*" >&2; exit 1; }

validate() {
  scratch=$(mktemp -d)
  trap 'rm -rf "$scratch"' EXIT
  rg --files src -g '*.rs' -g '!src/**/tests/**' | sort >"$scratch/files"
  jq -Rn '[inputs | select(startswith("| src/")) | split("|")[1:-1]
    | map(gsub("^ +| +$"; ""))
    | if length != 7 then error("malformed inventory row") else . end
    | {path:.[0], public:.[1], bridge:.[2], assets:.[3], owner:.[4], target:.[5], issue:.[6]}]' \
    "$INVENTORY" >"$scratch/rows"
  jq -e 'group_by(.path) | all(length == 1)' "$scratch/rows" >/dev/null || fail 'duplicate inventory row'
  jq -r '.[].path' "$scratch/rows" | sort >"$scratch/listed"
  diff -u "$scratch/files" "$scratch/listed" || fail 'inventory coverage mismatch'
  jq -e --slurpfile rows "$scratch/rows" '
    . as $p | .version == 1 and
    (["version","staged_top_level_dirs","staged_root_modules","domains","legacy_modules",
      "owner_dependencies","legacy_edges","store_callbacks","passive_store_models",
      "setup_runtime_dependencies"] - keys | length == 0) and
    (.setup_runtime_dependencies | type == "array") and
    .staged_top_level_dirs == ("api ast capture cli cloud config consolidate document domains eval graph memory output prune retrieval serve source stats store sync upgrade utilities"|split(" ")) and
    .staged_root_modules == ("api ast capture cli cloud config consolidate document embed errors eval fetch git_utils graph http_error index lib main memory output prelude prune retrieval serve simhash source stats store sync test_common upgrade"|split(" ")) and
    .domains == ("memories code documents graph retrieval learning sync capture maintenance integrations"|split(" ")) and
    all($rows[0][];
      (.owner | test("^(domains::[a-z_]+|delivery::(cli|serve)|shared::(config|utilities|root)|infrastructure::store)$")) and
      (if (.owner | startswith("domains::")) then
        (.owner | ltrimstr("domains::")) as $domain | ($p.domains | index($domain)) != null
      else true end)) and
    all($rows[0][]; (.target | test("^src/([a-z_]+/)*[a-z_]+\\.rs$")) and
      (.issue | test("^(retain|#(166|167|168|169|170|171|172|173|174|175|176|177|178))$")) and
      (.public == "private" or (.public | test("^comemory(::[A-Za-z_][A-Za-z_0-9]*)*; (preserve|crate-root-alias|breaking 0\\.[0-9]+\\.[0-9]+ .+#.+)$")))) and
    all(.legacy_modules[]; (.module | test("^crate::[a-z_]+$")) and
      (.issue | test("^#(166|167|168|169|170|171|172|173|174|175|176|177|178)$"))) and
    all(.owner_dependencies[]; (.source | ltrimstr("domains::")) as $s |
      (.target | ltrimstr("domains::")) as $t |
      ($p.domains | index($s)) != null and ($p.domains | index($t)) != null and $s != $t)
  ' "$POLICY" >/dev/null || fail 'invalid policy or inventory metadata'
  jq -e '
    {memories:"save delete list show update restore trash refresh_refs",
     code:"ast index_code ingest_code index_runs repos repo_admin hooks install_hooks",
     documents:"index sources unindex", graph:"graph graph_nodes graph_recompute edges",
     retrieval:"search search_code context find suggest config_retrieval",
     learning:"feedback eval mine tune bandit learning learning_proposals",
     sync:"sync memory_store", integrations:"install setup",
     maintenance:"doctor gc gc_policy prune consolidate rebuild reembed stats overview"}
    | to_entries | map(.key as $owner | .value | split(" ")[] |
        {key:.,value:("domains::"+$owner)}) | from_entries
  ' >"$scratch/api_owners" <<<null
  jq -e --slurpfile owners "$scratch/api_owners" '
    all(.[] | select(.path|startswith("src/api/"));
      (.path|split("/")[2]|split(".")[0]) as $core |
      .owner == (if $core == "completions" then "delivery::cli" else $owners[0][$core] end))
  ' "$scratch/rows" >/dev/null || fail 'API ownership mismatch'
  jq -e 'group_by(.target) | all(length == 1)' "$scratch/rows" >/dev/null || fail 'duplicate inventory target'
  jq -e --slurpfile rows "$scratch/rows" '
    (.owner_dependencies | group_by([.source,.target]) | all(length == 1)) and
    (.legacy_modules | group_by(.module) | all(length == 1)) and
    all(.legacy_modules[]; . as $entry | any($rows[0][];
      .path == (($entry.module|sub("^crate::";"src/"))+".rs") and .owner == $entry.owner and .issue == $entry.issue))
  ' "$POLICY" >/dev/null || fail 'invalid ownership policy'
  jq -e --slurpfile rows "$scratch/rows" '
    (.setup_runtime_dependencies | group_by([.source,.target]) | all(length == 1)) and
    all(.setup_runtime_dependencies[]; . as $entry |
      (.source | type == "string" and test("^src/([a-z_]+/)*[a-z_]+\\.rs$")) and
      (.target | type == "string" and test("^crate(::[A-Za-z_][A-Za-z_0-9]*)+$")) and
      (.owner | type == "string" and test("^domains::[a-z_]+$")) and
      any($rows[0][]; .path == (($entry.target|sub("^crate::";"src/")|gsub("::";"/"))+".rs") and .owner == $entry.owner))
  ' "$POLICY" >/dev/null || fail 'invalid setup runtime dependency'
  jq -e '
    [.legacy_edges[], .store_callbacks[]] as $edges |
    all($edges[]; (.source | type == "string" and test("^src/([a-z_]+/)*[a-z_]+\\.rs$")) and
      (.target | type == "string" and test("^crate(::[A-Za-z_][A-Za-z_0-9]*)+$")) and
      (.class | IN("delivery", "store-callback")) and (.issue | IN("#166", "#167", "#169", "#170", "#177"))) and
    ($edges | group_by([.source,.target]) | all(length == 1)) and
    (.passive_store_models | group_by([.source,.target]) | all(length == 1)) and
    all(.passive_store_models[]; (.source | startswith("src/store/")) and
      (.target | test("^crate(::[A-Za-z_][A-Za-z_0-9]*)+$")))
  ' "$POLICY" >/dev/null || fail 'invalid policy edge'
  ast-grep scan --inline-rules '
id: architecture-inventory
language: Rust
rule:
  any:
    - kind: use_declaration
    - kind: scoped_identifier
    - kind: attribute_item
    - kind: macro_invocation
    - kind: mod_item
' --json=compact src >"$scratch/ast"
  jq '
    [.[] | select(.file | contains("/tests/") | not) |
      select(.text|test("^pub mod [a-z_]+;$")) | . as $node |
      (.text|capture("^pub mod (?<name>[a-z_]+);$").name) as $name |
      {parent:.file,child:((if .file == "src/lib.rs" then "src"
        else .file|rtrimstr(".rs") end)+"/"+$name+".rs")}] as $edges |
    def reach:
      . as $seen | (.+[$edges[] | select(.parent as $p | $seen|index($p)) | .child]|unique) |
      if . == $seen then . else reach end;
    ["src/lib.rs"] | reach | map(select(. != "src/lib.rs"))
  ' "$scratch/ast" >"$scratch/public"
  jq -e --slurpfile public "$scratch/public" '
    all(.[]; . as $row | if ($public[0]|index($row.path)) != null then
      (.public|startswith(("comemory::"+($row.path|ltrimstr("src/")|rtrimstr(".rs")|gsub("/";"::")))+"; "))
      else .public == "private" end)
  ' "$scratch/rows" >/dev/null || fail 'public path mismatch'
  jq '
    def imports:
      sub("^(pub(\\([^)]*\\))? )?use +"; "") | gsub(" as [A-Za-z_][A-Za-z_0-9]*"; "") |
      [scan("[A-Za-z_][A-Za-z_0-9]*|[{},;*]")] |
      reduce .[] as $t ({stack:[[]],cur:[],out:[]};
        if $t == "{" then .stack += [.cur]
        elif $t == "," or $t == ";" or $t == "}" then
          (if .cur|length > 0 then .out += [.cur|map(select(. != "self"))|join("::")] else . end) |
          if $t == "}" then .stack = .stack[:-1] | .cur=[] else .cur=.stack[-1] end
        else .cur += [$t] end) | .out[];
    [.[] | select(.file | contains("/tests/") | not) |
      . as $node | if (.text | test("^(pub(\\([^)]*\\))? )?use ")) then
        .text | imports | {source:$node.file,target:.}
      elif (.text | test("^[A-Za-z_][A-Za-z_0-9]*(::[A-Za-z_][A-Za-z_0-9]*)+$")) then
        {source:.file,target:.text} else empty end] | group_by(.source) |
    [.[] as $raw | $raw[] | . as $edge | if (.target|startswith("crate::")) then . else
      ($edge.target|split("::")) as $parts |
      $raw[] | select(.source == $edge.source and (.target|startswith("crate::")) and
        (.target|split("::")|last) == $parts[0]) |
      {source:$edge.source,target:([.target]+$parts[1:]|join("::"))} end] | unique
  ' "$scratch/ast" >"$scratch/refs"
  jq -r --slurpfile refs "$scratch/refs" '
    [.legacy_edges[],.store_callbacks[],.passive_store_models[],.setup_runtime_dependencies[]] | .[] |
    . as $edge | if any($refs[0][]; .source == $edge.source and
      (.target == $edge.target or (.target|startswith($edge.target+"::")))) then empty
    else "absent policy edge: \(.source) -> \(.target)" end
  ' "$POLICY" >"$scratch/absent" || fail 'policy edge validation failed'
  [[ ! -s "$scratch/absent" ]] || fail "$(cat "$scratch/absent")"
  jq -r '
    def resolve: split("/") | reduce .[] as $p ([];
      if $p == ".." then .[:-1] elif $p == "." then . else .+[$p] end) | join("/");
    [.[] | select(.file | contains("/tests/") | not) | . as $node |
      if (.text|test("^#\\[path *= *\"")) then
        {kind:"bridge", relative:(.text|capture("\"(?<p>[^\"]+)\"").p)}
      elif (.text|test("^include(_str|_bytes)?!")) then
        {kind:"assets", relative:(.text|capture("\"(?<p>[^\"]+)\"").p)}
      else empty end |
      {source:$node.file,kind:.kind,path:(($node.file|split("/")|.[:-1]|join("/"))+"/"+.relative|resolve)}]
    | unique | .[] | [.source,.kind,.path] | @tsv
  ' "$scratch/ast" >"$scratch/resources"
  while IFS=$'\t' read -r source kind path; do
    [[ -f "$path" ]] || fail "missing $kind: $source -> $path"
  done <"$scratch/resources"
  jq -Rn '[inputs|split("\t")|{source:.[0],kind:.[1],path:.[2]}]' "$scratch/resources" >"$scratch/resource_json"
  jq -e --slurpfile expected "$scratch/resource_json" '
    [.[] | . as $row | ["bridge","assets"][] as $kind |
      $row[$kind] | select(. != "none") | split("; ")[] |
      {source:$row.path,kind:$kind,path:.}] | sort == ($expected[0]|sort)
  ' "$scratch/rows" >/dev/null || fail 'inventory asset/bridge mismatch'
  echo "PASS: $(wc -l <"$scratch/files" | tr -d " ") production files, policy edges, assets and bridges"
  rm -rf "$scratch"
  trap - EXIT
}

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
sed '/^| src\/api\/save.rs |/d' "$INVENTORY" >"$TASK_TMP/missing.md"
assert_fails 'missing production row' 'inventory coverage' --inventory "$TASK_TMP/missing.md"
cp "$INVENTORY" "$TASK_TMP/duplicate.md"
grep '^| src/api/save.rs |' "$INVENTORY" >>"$TASK_TMP/duplicate.md"
assert_fails 'duplicate production row' 'duplicate inventory row' --inventory "$TASK_TMP/duplicate.md"
jq '.legacy_edges[0].issue = "#999"' "$POLICY" >"$TASK_TMP/issue.json"
assert_fails 'unknown removal issue' 'invalid policy edge' --policy "$TASK_TMP/issue.json"
jq 'del(.legacy_edges[0].target)' "$POLICY" >"$TASK_TMP/target.json"
assert_fails 'missing exact target' 'invalid policy edge' --policy "$TASK_TMP/target.json"
jq '.legacy_edges[0].target = "crate::cli::not_present"' "$POLICY" >"$TASK_TMP/stale.json"
assert_fails 'stale allowlisted target' 'absent policy edge' --policy "$TASK_TMP/stale.json"
sed '/^| src\/api\/save.rs |/s/domains::memories/domains::code/' "$INVENTORY" >"$TASK_TMP/owner.md"
assert_fails 'wrong API owner' 'API ownership mismatch' --inventory "$TASK_TMP/owner.md"
sed '/^| src\/api\/save.rs |/s@src/domains/memories/save.rs@src/domains/memories/delete.rs@' "$INVENTORY" >"$TASK_TMP/target.md"
assert_fails 'duplicate migration target' 'duplicate inventory target' --inventory "$TASK_TMP/target.md"
sed '/^| src\/api\/save.rs |/s@src/domains/memories/save.rs@none@' "$INVENTORY" >"$TASK_TMP/no-target.md"
assert_fails 'missing inventory target' 'invalid policy or inventory metadata' --inventory "$TASK_TMP/no-target.md"
jq '.legacy_edges += [.legacy_edges[0]]' "$POLICY" >"$TASK_TMP/duplicate.json"
assert_fails 'duplicate policy edge' 'invalid policy edge' --policy "$TASK_TMP/duplicate.json"
jq '.store_callbacks[0].target = "crate::graph::cross_link::absent"' "$POLICY" >"$TASK_TMP/callback.json"
assert_fails 'stale store callback' 'absent policy edge' --policy "$TASK_TMP/callback.json"
jq '.passive_store_models[0].target = "crate::memory::Absent"' "$POLICY" >"$TASK_TMP/model.json"
assert_fails 'stale passive model' 'absent policy edge' --policy "$TASK_TMP/model.json"
sed '/^| src\/memory.rs |/s/comemory::memory; crate-root-alias/private/' "$INVENTORY" >"$TASK_TMP/public.md"
assert_fails 'missing public compatibility choice' 'public path mismatch' --inventory "$TASK_TMP/public.md"
sed '/^| src\/capture\/redact.rs |/s@src/capture/rules.toml@none@' "$INVENTORY" >"$TASK_TMP/asset.md"
assert_fails 'missing compile-time asset' 'inventory asset/bridge mismatch' --inventory "$TASK_TMP/asset.md"

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
