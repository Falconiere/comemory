#!/usr/bin/env bash
# Enforce the staged domain contract without changing the vendored guardrails.
set -eu
SCRIPT_ROOT=$(cd "$(dirname "$0")/.." && pwd)
ROOT=$SCRIPT_ROOT
POLICY=
INVENTORY=
FILES=()
SCOPED=false
bad() { printf 'architecture-check: %s\n' "$*" >&2; exit 3; }
while [ "$#" -gt 0 ]; do
  case "$1" in
    --file)
      SCOPED=true
      shift
      [ "$#" -gt 0 ] || bad 'missing argument for --file'
      case "$1" in --*) bad 'missing argument for --file' ;; esac
      while [ "$#" -gt 0 ]; do
        case "$1" in --*) break ;; esac
        FILES+=("$1")
        shift
      done ;;
    --root|--policy|--inventory)
      [ "$#" -ge 2 ] && [ -n "$2" ] || bad "missing argument for $1"
      case "$2" in --*) bad "missing argument for $1" ;; esac
      case "$1" in
        --root) ROOT=$2 ;; --policy) POLICY=$2 ;; --inventory) INVENTORY=$2 ;;
      esac
      shift 2 ;;
    *) bad "unknown argument: $1" ;;
  esac
done
for tool in jq ast-grep find sort rg diff; do
  command -v "$tool" >/dev/null 2>&1 || bad "missing tool: $tool"
done
[ -d "$ROOT/src" ] || bad "missing input: $ROOT/src"
ROOT=$(cd "$ROOT" && pwd)
POLICY=${POLICY:-$ROOT/scripts/architecture-policy.json}
INVENTORY=${INVENTORY:-$ROOT/docs/designs/2026-09-17-domain-first-migration-inventory.md}
for input in "$POLICY" "$INVENTORY"; do [ -f "$input" ] || bad "missing input: $input"; done
POLICY=$(cd "$(dirname "$POLICY")" && pwd)/$(basename "$POLICY")
INVENTORY=$(cd "$(dirname "$INVENTORY")" && pwd)/$(basename "$INVENTORY")
TASK_TMP=$(mktemp -d)
trap 'rm -rf "$TASK_TMP"' EXIT
# Metadata is validated before parsing Rust, including records outside --file.
jq -e '
  def unique_by_key(f): group_by(f) | all(length == 1);
  def name: type == "string" and test("^[a-z_][a-z_0-9]*$");
  def path: type == "string" and test("^src/([a-z_][a-z_0-9]*/)*[a-z_][a-z_0-9]*\\.rs$");
  def target: type == "string" and test("^crate(::[A-Za-z_][A-Za-z_0-9]*)+$");
  def issue: type == "string" and test("^#(166|167|168|169|170|171|172|173|174|175|176|177|178)$");
  def owner($p): . as $o | type == "string" and
    (test("^(delivery::(cli|serve)|shared::(config|utilities|root)|infrastructure::store)$") or
      any($p.domains[]; $o == "domains::"+.));
  . as $p | type == "object" and .version == 1 and
  all([.staged_top_level_dirs,.staged_root_modules,.domains,.legacy_modules,
    .owner_dependencies,.legacy_edges,.store_callbacks,.passive_store_models,
    .setup_runtime_dependencies][]; type == "array") and
  all([.staged_top_level_dirs,.staged_root_modules,.domains][]; all(.[]; name) and (unique_by_key(.))) and
  all(.legacy_modules[]; (.module|type == "string" and test("^crate::[a-z_]+$")) and
    (.owner|owner($p)) and (.issue|issue)) and
  (.legacy_modules|unique_by_key(.module)) and
  all(.owner_dependencies[]; (.source|owner($p)) and (.target|owner($p)) and
    (.source|startswith("domains::")) and (.target|startswith("domains::")) and .source != .target) and
  (.owner_dependencies|unique_by_key([.source,.target])) and
  all(.legacy_edges[]; (.source|path) and (.target|target) and .class == "delivery" and (.issue|issue)) and
  all(.store_callbacks[]; (.source|path) and (.source|startswith("src/store/")) and
    (.target|target) and .class == "store-callback" and .issue == "#177") and
  ([.legacy_edges[],.store_callbacks[]]|unique_by_key([.source,.target])) and
  all(.passive_store_models[]; (.source|path) and (.source|startswith("src/store/")) and (.target|target)) and
  (.passive_store_models|unique_by_key([.source,.target])) and
  all(.setup_runtime_dependencies[]; (.source|path) and (.target|target) and (.owner|owner($p))) and
  (.setup_runtime_dependencies|unique_by_key([.source,.target]))
' "$POLICY" >/dev/null 2>&1 || bad 'invalid policy'
jq -Rn '[inputs | select(startswith("| src/")) | split("|")[1:-1] | map(gsub("^ +| +$"; "")) |
  if length != 7 then error("invalid inventory row") else
    {path:.[0],owner:.[4],target:.[5],issue:.[6]} end]' "$INVENTORY" >"$TASK_TMP/inventory" || bad 'invalid inventory'
jq -e --slurpfile policy "$POLICY" '
  length > 0 and (group_by(.path)|all(length == 1)) and (group_by(.target)|all(length == 1)) and
  all(.[]; (.path|test("^src/([a-z_]+/)*[a-z_]+\\.rs$")) and
    (.target|test("^src/([a-z_]+/)*[a-z_]+\\.rs$")) and
    (.owner|test("^(domains::[a-z_]+|delivery::(cli|serve)|shared::(config|utilities|root)|infrastructure::store)$")) and
    (if (.owner|startswith("domains::")) then (.owner|ltrimstr("domains::")) as $d |
      ($policy[0].domains|index($d)) != null else true end))
' "$TASK_TMP/inventory" >/dev/null || bad 'invalid inventory'
cd "$ROOT"
for file in "${FILES[@]+${FILES[@]}}"; do
  case "$file" in "$ROOT"/*) file=${file#"$ROOT/"} ;; ./*) file=${file#./} ;; esac
  case "$file" in */../*|*/./*) bad "invalid file argument: $file" ;; esac
  case "$file" in tests/*|src/*/tests/*) continue ;; src/*.rs) ;; *) bad "invalid file argument: $file" ;; esac
  [ -f "$file" ] || bad "missing input: $file"
  printf '%s\n' "$file"
done >"$TASK_TMP/selected"
jq -Rn '[inputs]' "$TASK_TMP/selected" >"$TASK_TMP/scope"
find src -type f -name '*.rs' ! -path '*/tests/*' | sort >"$TASK_TMP/files"
find src -type d ! -path '*/tests' ! -path '*/tests/*' ! -path '*/proptest-regressions*' | sort >"$TASK_TMP/dirs"
jq -Rn '[inputs]' "$TASK_TMP/files" >"$TASK_TMP/file_json"
jq -Rn '[inputs]' "$TASK_TMP/dirs" >"$TASK_TMP/dir_json"
# Both entry modes enforce the complete contract before dependency analysis.
# shellcheck source=scripts/lib/architecture-inventory.sh
source "$SCRIPT_ROOT/scripts/lib/architecture-inventory.sh"
INVENTORY_EDGE_CHECK=false INVENTORY_QUIET=true validate_inventory
# Different rule ids retain node kinds; ancestor predicates remove path prefixes
# and import children, preventing a grouped import from becoming a broad exemption.
status=0
ast-grep scan --inline-rules '
id: imports
language: Rust
rule: {kind: use_declaration}
---
id: paths
language: Rust
rule:
  all:
    - any:
        - kind: scoped_identifier
        - kind: scoped_type_identifier
    - not:
        inside:
          kind: use_declaration
          stopBy: end
    - not:
        inside:
          any:
            - kind: scoped_identifier
            - kind: scoped_type_identifier
---
id: modules
language: Rust
rule:
  kind: mod_item
  has:
    field: name
    pattern: $NAME
---
id: lexical-scopes
language: Rust
rule: {kind: block}
---
id: function-scopes
language: Rust
rule: {kind: function_item}
---
id: receiver-bindings
language: Rust
rule:
  all:
    - any:
        - kind: parameter
        - kind: let_declaration
    - has: {field: pattern, pattern: $RECEIVER}
    - any:
        - has: {field: type, pattern: $TYPE}
        - all:
            - kind: let_declaration
            - not: {has: {field: type, pattern: $TYPE}}
---
id: method-calls
language: Rust
rule:
  kind: field_expression
  pattern: $RECEIVER.$METHOD
  inside: {kind: call_expression, field: function}
---
id: module-scopes
language: Rust
rule:
  kind: declaration_list
  inside: {kind: mod_item}
---
id: enum-variants
language: Rust
rule:
  kind: enum_variant
  has:
    field: name
    pattern: $VARIANT
  inside:
    kind: enum_item
    stopBy: end
    has:
      field: name
      pattern: $ENUM
---
id: substance
language: Rust
rule:
  any:
    - kind: function_item
    - kind: struct_item
    - kind: enum_item
    - kind: trait_item
    - kind: impl_item
    - kind: const_item
    - kind: static_item
    - kind: type_item
    - kind: macro_definition
' --globs '!**/tests/**' --json=compact src >"$TASK_TMP/ast" 2>"$TASK_TMP/ast_errors" || status=$?
[ "$status" -le 1 ] || bad "Rust parser failed: $(cat "$TASK_TMP/ast_errors")"
jq -e 'type == "array"' "$TASK_TMP/ast" >/dev/null || bad 'Rust parser returned invalid output'
jq '
  def module_path: ltrimstr("src/") | rtrimstr(".rs") | split("/") |
    if . == ["lib"] or . == ["main"] then ["crate"] else ["crate"]+. end;
  def tokens:
    [scan("/\\*|\\*/|//|\\n|(?:r#)?[A-Za-z_][A-Za-z_0-9]*|[{},;*]")] |
    reduce .[] as $t ({depth:0,line:false,out:[]};
      if $t == "\n" then .line=false elif .line then .
      elif $t == "/*" then .depth += 1 elif $t == "*/" then .depth -= 1
      elif .depth > 0 then . elif $t == "//" then .line=true
      else .out += [$t|ltrimstr("r#")] end) | .out;
  def imports:
    tokens | .[(index("use")+1):] |
    reduce .[] as $t ({stack:[[]],cur:[],alias:null,rename:false,out:[]};
      if $t == "{" then .stack += [.cur] | .cur=.stack[-1]
      elif $t == "as" then .rename=true
      elif $t == "," or $t == ";" or $t == "}" then
        (if (.cur|length)>0 then .out += [{parts:.cur,alias:.alias}] else . end) |
        .alias=null | .rename=false |
        if $t == "}" then .stack=.stack[:-1] | .cur=[] else .cur=.stack[-1] end
      elif .rename then .alias=$t | .rename=false else .cur += [$t] end) | .out[];
  def canonical($parts;$base):
    if $parts[0] == "crate" then $parts
    elif $parts[0] == "self" then $base+$parts[1:]
    elif $parts[0] == "super" then canonical($parts[1:];$base[:-1]) as $rest |
      if $parts[1] == "super" or $parts[1] == "self" then $rest else $base[:-1]+$parts[1:] end
    else $parts end | if .[-1] == "self" then .[:-1] else . end;
  [.[] | select(.ruleId == "modules" and (.text|test("\\{")))] as $modules |
  def base_for($node):
    ($node.file|module_path) + ([$modules[] | select(.file == $node.file and
      .range.byteOffset.start < $node.range.byteOffset.start and
      .range.byteOffset.end >= $node.range.byteOffset.end) |
      {start:.range.byteOffset.start,name:(.metaVariables.single.NAME.text|ltrimstr("r#"))}] |
      sort_by(.start)|map(.name));
  ([.[] | select(.ruleId == "modules") | {source:.file,base:base_for(.),
    name:(.metaVariables.single.NAME.text|ltrimstr("r#"))}] |
    map({key:([.source]+.base+[.name]|join("::")),value:true}) | from_entries) as $declared |
  ([.[] | select(.ruleId == "lexical-scopes" or .ruleId == "module-scopes" or .ruleId == "function-scopes")] |
    group_by(.file) | map({key:.[0].file,value:map({start:.range.byteOffset.start,
      end:.range.byteOffset.end,module:(.ruleId == "module-scopes")})}) | from_entries) as $scopes |
  [ .[] | select(.ruleId | IN("imports","paths","enum-variants","receiver-bindings","method-calls")) | . as $node |
    base_for(.) as $base |
    ([$scopes[$node.file][]? | select(.start < $node.range.byteOffset.start and
      .end >= $node.range.byteOffset.end)] | sort_by(.start) | last //
      {start:-1,end:9007199254740991,module:true}) as $scope |
    (if .ruleId == "imports" then .text|imports
      elif .ruleId == "receiver-bindings" then
        {parts:((.metaVariables.single.TYPE.text // "") |
          gsub("\u0027[A-Za-z_][A-Za-z_0-9]*"; "") | tokens | map(select(. != "mut"))),alias:null}
      elif .ruleId == "method-calls" then
        {parts:[.metaVariables.single.RECEIVER.text,.metaVariables.single.METHOD.text],alias:null}
      elif .ruleId == "enum-variants" then {parts:($base+
        [.metaVariables.single.ENUM.text,.metaVariables.single.VARIANT.text]|map(ltrimstr("r#"))),alias:null} else
      {parts:(.text|tokens),alias:null} end) |
    {source:$node.file,parts:canonical(.parts;$base),alias:.alias,kind:$node.ruleId,
      receiver:($node.metaVariables.single.RECEIVER.text // null),
      base:$base,scope:$scope,
      at:(if $node.ruleId == "receiver-bindings" and ($node.text|tokens|first) == "let" and
        ($node.metaVariables.single.TYPE.text // "") == ""
        then $node.range.byteOffset.end else $node.range.byteOffset.start end)}
  ] as $raw |
    ([$raw[] | select(.kind == "imports") | . + {name:(.alias // .parts[-1]),
      binding:(.base+[(.alias // .parts[-1])])}]) as $aliases |
    ($aliases|group_by(.name)|map({key:.[0].name,value:.})|from_entries) as $names |
    ($aliases|group_by(.binding)|map({key:(.[0].binding|join("::")),value:.})|from_entries) as $bindings |
    def resolve($parts;$context;$seen):
        ((if $parts[0] == "crate" then [range(1;($parts|length)+1) as $n |
          $bindings[$parts[:$n]|join("::")][]?] else $names[$parts[0]] // [] end) |
          [.[] | select(
          if $parts[0] == "crate" then .scope.module and
            (.source == $context.source or $context.base[:(.base|length)] == .base)
          else .source == $context.source and .base == $context.base and
            .scope.start < $context.at and .scope.end > $context.at end)] |
          sort_by([(.binding|length),.scope.start]) | last) as $a |
        if $a == null then
          if $declared[([$context.source]+$context.base+[$parts[0]]|join("::"))] == true
          then $context.base+$parts else $parts end
        else
          (if $parts[0] == "crate" then ($a.binding|length) else 1 end) as $used |
          ([$a.source,$a.at,$a.name]|tojson) as $key |
          if $a.parts == $parts[:$used] or ($seen|index($key)) != null then $parts else
            resolve($a.parts+$parts[$used:];$a;$seen+[$key])
          end
        end;
    ($raw | map(select(.kind == "receiver-bindings")) | group_by([.source,.receiver]) |
      map({key:([.[0].source,.[0].receiver]|tojson),value:.}) | from_entries) as $receivers |
    def receiver_type($entry):
      [$receivers[[$entry.source,$entry.receiver]|tojson][]? | select(
        .scope.start < $entry.at and .scope.end > $entry.at and
        .at <= $entry.at)] | sort_by(.scope.start,.at) | last;
    [$raw[] | select(.kind != "receiver-bindings") | . as $entry |
      .parts=(if .kind == "method-calls" then receiver_type($entry) as $binding |
        if $binding == null or ($binding.parts|length) == 0 then []
        else resolve($binding.parts;$binding;[])+[.parts[-1]] end
        else resolve(.parts;$entry;[]) end) | select(.parts[0] == "crate") |
    {source,target:(.parts|join("::")),kind,
      binding:(if .kind == "imports" and .scope.module then
        (.base+[(.alias // .parts[-1])]|join("::")) else null end)}
  ] | unique | {edges:map(select(.kind != "enum-variants")),
    variants:map(select(.kind == "enum-variants")|.target)}
' "$TASK_TMP/ast" >"$TASK_TMP/edges" || bad 'path normalization failed'

jq -nr --slurpfile p "$POLICY" --slurpfile inv "$TASK_TMP/inventory" \
  --slurpfile ast "$TASK_TMP/ast" --slurpfile edges "$TASK_TMP/edges" \
  --slurpfile files "$TASK_TMP/file_json" --slurpfile dirs "$TASK_TMP/dir_json" \
  --slurpfile scope "$TASK_TMP/scope" --argjson scoped "$SCOPED" '
  $p[0] as $p | $inv[0] as $inv | $edges[0] as $refs | $refs.edges as $edges |
  def prefix($parent): . == $parent or startswith($parent+"::");
  def module_path: ltrimstr("src/")|rtrimstr(".rs")|gsub("/";"::")|"crate::"+.;
  ([$p.legacy_modules[]|{key:.module,value:.owner}] +
   [$inv[]|{key:(.path|module_path),value:.owner}] | from_entries) as $owners |
  def owned($target):
    if ($target|startswith("crate::domains::")) then ($target|split("::")|.[1:3]|join("::"))
    else ($target|split("::")) as $parts |
      [range(1;($parts|length)+1) as $n | $owners[$parts[:$n]|join("::")]|select(. != null)] | last end;
  def selected($source): ($scoped|not) or any($scope[0][];
    . == $source or startswith(($source|rtrimstr(".rs"))+"/"));
  def exemption($list;$edge): any($list[]; . as $entry | .source == $edge.source and ($edge.target|prefix($entry.target)));
  def diagnostic($source;$target;$why): {source:$source,target:$target,why:$why};
  ([$edges[]|select(.binding != null)]|group_by(.target)|
    map({key:.[0].target,value:map(.binding)})|from_entries) as $enum_imports |
  def enum_aliases:
    . as $known | (. + [$known[]|split("::") as $parts |
      $enum_imports[$parts[:-1]|join("::")][]? | .+"::"+$parts[-1]] | unique) |
    if . == $known then . else enum_aliases end;
  ($refs.variants | unique | enum_aliases) as $variants |
  [
    $dirs[0][] | select((split("/")|length)==2) | . as $d |
      select(($p.staged_top_level_dirs|index($d|split("/")[1])) == null) |
      diagnostic(.;"";"unapproved root directory"),
    empty
  ] + [
    $files[0][] | select((split("/")|length)==2) | . as $f |
      select(($p.staged_root_modules|index($f|ltrimstr("src/")|rtrimstr(".rs"))) == null and
        . != "src/domains.rs" and . != "src/utilities.rs") | diagnostic(.;"";"unapproved root module")
  ] + [
    ([$files[0][],$dirs[0][]|select(startswith("src/domains/"))|split("/")[2]|rtrimstr(".rs")] +
     [$ast[0][]|select(.file == "src/domains.rs" and .ruleId == "modules")|
       .metaVariables.single.NAME.text|ltrimstr("r#")] | unique)[] as $d |
    ("src/domains/"+$d) as $base |
    if ($p.domains|index($d)) == null then diagnostic($base;"";"unapproved domain") else
      (if any($ast[0][]; (.file == ($base+".rs") or (.file|startswith($base+"/"))) and .ruleId == "substance")
       then empty else diagnostic($base;"";"empty domain scaffold") end),
      (if ($files[0]|index($base+".rs")) == null then diagnostic($base;"";"missing sibling module") else empty end)
    end
  ] + [
    if any($files[0][],$dirs[0][]; . == "src/domains" or startswith("src/domains/")) then
      (if ($files[0]|index("src/domains.rs")) == null then
        diagnostic("src/domains";"";"missing sibling module") else empty end),
      (if any($ast[0][]; (.file|startswith("src/domains/")) and .ruleId == "substance") then empty
        else diagnostic("src/domains";"";"empty domain scaffold") end)
    else empty end
  ] + [
    $dirs[0][] | select(startswith("src/domains/") and (split("/")|length)>3) | . as $d |
    if any($inv[]; .target|startswith($d+"/")) then empty
      else diagnostic($d;"";"unapproved domain directory") end,
    if ($files[0]|index($d+".rs")) == null then diagnostic($d;"";"missing sibling module") else empty end
  ] + [
    [$p.legacy_edges[],$p.store_callbacks[],$p.passive_store_models[],$p.setup_runtime_dependencies[]][] as $entry |
    if any($edges[]; .source == $entry.source and (.target|prefix($entry.target))) then empty
      else diagnostic($entry.source;$entry.target;"stale policy edge") end
  ] + [
    $edges[] | select(selected(.source)) | . as $edge |
    owned(.source|module_path) as $source_owner | owned(.target) as $target_owner |
    if (.source == "src/store.rs" or (.source|startswith("src/store/"))) and
       (($target_owner // "")|startswith("domains::")) then
      if exemption($p.store_callbacks;$edge) then empty
      elif any($p.store_callbacks[]; .source == $edge.source and (.target|startswith($edge.target+"::"))) and
        .kind == "imports" then empty
      elif any($p.passive_store_models[]; . as $model | .source == $edge.source and
        ($edge.target == .target or ($edge.target|IN($model.target+"::new",$model.target+"::default",$model.target+"::from")) or
          (($edge.target|startswith($model.target+"::")) and ($variants|index($edge.target)) != null))) then empty
      else diagnostic(.source;.target;"store service dependency") end
    elif (($source_owner // "")|startswith("domains::")) or (.source|startswith("src/api/")) or
      .source == "src/domains.rs" then
      if (.target|test("^crate::(cli|serve|output)(::|$)")) then
        if exemption($p.legacy_edges;$edge) then empty else diagnostic(.source;.target;"delivery dependency") end
      elif (.source|startswith("src/domains/")) and (.target|test("^crate::api(::|$)")) then
        diagnostic(.source;.target;"legacy core dependency")
      elif (($target_owner // "")|startswith("domains::")) and $source_owner != $target_owner and
        ($source_owner // ""|startswith("domains::")) and
        (any($p.owner_dependencies[]; .source == $source_owner and .target == $target_owner)|not) then
        diagnostic(.source;.target;"unapproved owner dependency")
      else empty end
    else empty end
  ] | unique | sort_by(.source,.target,.why) | group_by(.source)[] |
    .[0].source+":", (.[]|"  "+.why+(if .target == "" then "" else " -> "+.target end))
' >"$TASK_TMP/diagnostics" 2>"$TASK_TMP/jq_errors" || bad "analysis failed: $(cat "$TASK_TMP/jq_errors")"
if [ -s "$TASK_TMP/diagnostics" ]; then cat "$TASK_TMP/diagnostics" >&2; exit 1; fi
exit 0
