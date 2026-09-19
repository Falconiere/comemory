#!/usr/bin/env bash
# Shared complete inventory validation for the gate and its regression harness.
validate_inventory() (
  fail() { printf 'architecture-policy: %s\n' "$*" >&2; exit 1; }
  fail_file() {
    printf 'architecture-policy: ' >&2
    cat "$1" >&2
    exit 1
  }
  scratch=$(mktemp -d)
  trap 'rm -rf "$scratch"' EXIT
  find src -type f -name '*.rs' ! -path '*/tests/*' | sort >"$scratch/files"
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
      "setup_runtime_dependencies","shared_domain_dependencies"] - keys | length == 0) and
    (.setup_runtime_dependencies | type == "array") and
    .staged_top_level_dirs == ("cli config domains mcp serve store utilities"|split(" ")) and
    .staged_root_modules == ("cli config errors lib main mcp prelude serve store test_common"|split(" ")) and
    .domains == ("memories code documents graph retrieval learning sync capture maintenance integrations"|split(" ")) and
    all($rows[0][];
      (.owner | test("^(domains::[a-z_]+|delivery::(cli|serve|mcp)|shared::(config|utilities|root)|infrastructure::store)$")) and
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
  # Ownership, as one total function from path to owner, checked over every
  # row. #165 mapped only rows under `src/api/`, and when #175 moved the last
  # core out `all` quantified over an empty selection and passed asserting
  # nothing. #175's successor anchored on `src/domains/<cap>/`, which covers
  # 196 of 411 rows and says nothing about the rest — so a file at the source
  # ROOT could still claim a delivery owner, which is exactly what
  # `src/output.rs` did. Stating the map for every path leaves no unconstrained
  # region and no selection that can go empty. The reverse clause is kept for
  # the one path the forward map deliberately exempts: a `<cap>` segment that
  # is not a declared domain belongs to the `unapproved domain` diagnostic, and
  # claiming it here would replace that message with this one.
  jq -e --slurpfile policy "$POLICY" '
    ($policy[0].domains) as $domains |
    def expected:
      if startswith("src/domains/") then
        (split("/")[2] | rtrimstr(".rs")) as $capability |
        (if ($domains | index($capability)) == null then null
         else "domains::" + $capability end)
      elif . == "src/cli.rs" or startswith("src/cli/") then "delivery::cli"
      elif . == "src/serve.rs" or startswith("src/serve/") then "delivery::serve"
      elif . == "src/mcp.rs" or startswith("src/mcp/") then "delivery::mcp"
      elif . == "src/store.rs" or startswith("src/store/") then "infrastructure::store"
      elif . == "src/config.rs" or startswith("src/config/") then "shared::config"
      elif . == "src/utilities.rs" or startswith("src/utilities/") then "shared::utilities"
      else "shared::root" end;
    all(.[]; . as $row |
      (($row.path | expected) as $want | $want == null or $row.owner == $want) and
      (if ($row.owner | startswith("domains::")) then
        ($row.owner | ltrimstr("domains::")) as $capability |
        $row.path == "src/domains/" + $capability + ".rs" or
          ($row.path | startswith("src/domains/" + $capability + "/"))
      else true end))
  ' "$scratch/rows" >/dev/null || fail 'capability ownership mismatch'
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
  # The shared layer's declared escapes. `config` and `utilities` sit under
  # every capability, so an edge back into one inverts the layering; the list
  # names the ones that survive, each with a written reason and a source the
  # ledger agrees is a shared file. `test-architecture-policy.sh` additionally
  # requires the REAL list to stay non-empty, for the same reason `legacy_edges`
  # and `store_callbacks` seed a fixture entry: `all` over an empty array passes
  # while asserting nothing.
  jq -e --slurpfile rows "$scratch/rows" '
    (.shared_domain_dependencies | group_by([.source,.target]) | all(length == 1)) and
    all(.shared_domain_dependencies[]; . as $entry |
      (.source | type == "string" and test("^src/(config|utilities)/([a-z_]+/)*[a-z_]+\\.rs$")) and
      (.target | type == "string" and test("^crate::domains(::[A-Za-z_][A-Za-z_0-9]*)+$")) and
      (.owner | type == "string" and test("^domains::[a-z_]+$")) and
      (.reason | type == "string" and (length > 40)) and
      any($rows[0][]; .path == $entry.source and
        (.owner == "shared::config" or .owner == "shared::utilities")))
  ' "$POLICY" >/dev/null || fail 'invalid shared domain dependency'
  jq -e '
    [.legacy_edges[], .store_callbacks[]] as $edges |
    all($edges[]; (.source | type == "string" and test("^src/([a-z_]+/)*[a-z_]+\\.rs$")) and
      (.target | type == "string" and test("^crate(::[A-Za-z_][A-Za-z_0-9]*)+$")) and
      (.class | IN("delivery", "store-callback")) and (.issue | IN("#166", "#167", "#169", "#170", "#177"))) and
    ($edges | group_by([.source,.target]) | all(length == 1)) and
    (.passive_store_models | group_by([.source,.target]) | all(length == 1)) and
    all(.passive_store_models[]; (.source | startswith("src/store/")) and
      (.target | test("^crate(::[A-Za-z_][A-Za-z_0-9]*)+$") and
        test("::[A-Z][A-Za-z_0-9]*$")))
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
  if [ "${INVENTORY_EDGE_CHECK:-true}" = true ]; then
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
    [.legacy_edges[],.store_callbacks[],.passive_store_models[],
     .setup_runtime_dependencies[],.shared_domain_dependencies[]] | .[] |
    . as $edge | if any($refs[0][]; .source == $edge.source and
      (.target == $edge.target or (.target|startswith($edge.target+"::")))) then empty
    else "absent policy edge: \(.source) -> \(.target)" end
  ' "$POLICY" >"$scratch/absent" || fail 'policy edge validation failed'
  [[ ! -s "$scratch/absent" ]] || fail_file "$scratch/absent"
  fi
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
  if [ "${INVENTORY_QUIET:-false}" != true ]; then
    echo "PASS: $(wc -l <"$scratch/files" | tr -d " ") production files, policy edges, assets and bridges"
  fi
  rm -rf "$scratch"
  trap - EXIT
)
