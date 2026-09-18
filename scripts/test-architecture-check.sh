#!/usr/bin/env bash
# Real source-tree fixtures: weakening any boundary must fail a matching case.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
CHECK="$ROOT/scripts/architecture-check.sh"
TASK_TMP=$(mktemp -d)
trap 'rm -rf "$TASK_TMP"' EXIT
COUNT=0
write_baseline() {
  jq -Rn '[inputs | select(startswith("| src/")) | split("|")[1:-1] |
  map(gsub("^ +| +$"; "")) |
  if length != 7 then error("malformed baseline inventory row") else
    {path:.[0],owner:.[4],target:.[5],issue:.[6]}
  end]' \
    "$1" >"$2"
}
write_baseline "$ROOT/docs/designs/2026-09-17-domain-first-migration-inventory.md" "$TASK_TMP/baseline"

put() { mkdir -p "$TREE/$(dirname "$1")"; printf '%s\n' "$2" >"$TREE/$1"; }
new_tree() {
  TREE="$TASK_TMP/$1"
  mkdir -p "$TREE/src" "$TREE/scripts" "$TREE/docs/designs"
  jq '.legacy_edges=[] | .store_callbacks=[] | .passive_store_models=[] |
      .setup_runtime_dependencies=[] | .shared_domain_dependencies=[]' \
    "$ROOT/scripts/architecture-policy.json" >"$TREE/scripts/architecture-policy.json"
  put src/lib.rs '//! Fixture crate.'
}
policy() {
  jq "$1" "$TREE/scripts/architecture-policy.json" >"$TASK_TMP/next.json"
  cp "$TASK_TMP/next.json" "$TREE/scripts/architecture-policy.json"
}
domain() {
  put src/domains.rs 'pub mod memories;'
  put src/domains/memories.rs "$1"
}
assert_status() {
  local want=$1 needle=$2 actual=0
  shift 2
  # Fixtures carry a complete contract too; ownership comes from the checked
  # inventory while public paths/assets remain absent in these private trees.
  if [ "${FIXTURE_INVENTORY_READY:-false}" != true ]; then
  (cd "$TREE" && find src -name '*.rs' ! -path '*/tests/*' | sort) |
    jq -Rn --slurpfile baseline "$TASK_TMP/baseline" --slurpfile policy "$ROOT/scripts/architecture-policy.json" '
      inputs as $path | ($baseline[0] | map(select(.path == $path)) | first) as $row |
      ($row.owner // (if ($path|startswith("src/domains/")) and
        (($policy[0].domains|index($path|split("/")[2]|rtrimstr(".rs"))) != null) then
        "domains::"+($path|split("/")[2]|rtrimstr(".rs"))
        elif ($path|startswith("src/store/")) then "infrastructure::store"
        elif ($path|startswith("src/config/")) then "shared::config"
        elif ($path|startswith("src/utilities/")) then "shared::utilities"
        else "shared::root" end)) as $owner |
      (if ($path|contains("/surprise/")) then ($path|sub("/surprise/";"/")) else $path end) as $target |
      "| "+([$path,"private","none","none",$owner,$target,($row.issue // "retain")]|join(" | "))+" |"
    ' -r >"$TREE/docs/designs/2026-09-17-domain-first-migration-inventory.md"
  fi
  bash "$CHECK" --root "$TREE" "$@" >"$TASK_TMP/result" 2>&1 || actual=$?
  if [ "$actual" != "$want" ] || { [ -n "$needle" ] && ! grep -Fq "$needle" "$TASK_TMP/result"; }; then
    printf 'FAIL: %s (%s), expected %s / %s; got %s\n' "${TREE##*/}" "$*" "$want" "$needle" "$actual" >&2
    cat "$TASK_TMP/result" >&2
    exit 1
  fi
  if [ "$want" = 0 ] && [ -s "$TASK_TMP/result" ]; then
    printf 'FAIL: valid tree should be silent\n' >&2
    cat "$TASK_TMP/result" >&2
    exit 1
  fi
  COUNT=$((COUNT + 1))
}
both() { assert_status "$1" "$2"; assert_status "$1" "$2" --file "$3"; }

# A removed gate lets the source tree report green without its architecture
# contract. This verifies the actual CI and staged-hook wiring, including the
# required check-all order, against disposable copies.
test_gate_wiring() {
  local check_all=$1 hook=$2 command
  awk '
    /^GATES=\(/ { collecting = 1; next }
    collecting && /^\)/ { exit }
    collecting && /^  guardrails-check$/ { guardrails = NR }
    collecting && /^  architecture-check$/ { architecture = NR }
    collecting && /^  store-chokepoint-check$/ { store = NR }
    END { exit(!(guardrails < architecture && architecture < store)) }
  ' "$check_all" || return 1
  command=$(awk '
    /^    architecture:$/ { command = 1; next }
    command && /^      run: / {
      sub(/^      run: /, "")
      print
      found = 1
      exit
    }
    command && /^    [^ ]/ { exit 1 }
    END { exit(found ? 0 : 1) }
  ' "$hook") || return 1
  # Execute the configured command with the multi-path expansion lefthook
  # supplies, including a root integration test matched by its Rust glob.
  command=${command/\{staged_files\}/'"$@"'}
  (cd "$ROOT" && bash -c "$command" architecture-hook src/lib.rs src/config.rs tests/cli__setup.rs)
}
assert_gate_wiring() {
  if ! test_gate_wiring "$@"; then
    printf 'FAIL: architecture checker is not wired into check-all and pre-commit\n' >&2
    exit 1
  fi
  COUNT=$((COUNT + 1))
}
assert_fails() {
  local test=$1
  shift
  if "$test" "$@"; then
    printf 'FAIL: gate wiring accepted %s\n' "$*" >&2
    exit 1
  fi
  COUNT=$((COUNT + 1))
}

# Error output is emitted as data from its file, never interpolated into a
# command-substitution argument. This keeps diagnostics intact when a parser
# reports shell-significant text.
assert_no_interpolated_error_files() {
  local command_substitution='$'"(cat "
  if rg -Fq "$command_substitution" "$CHECK" "$ROOT/scripts/lib/architecture-inventory.sh"; then
    printf 'FAIL: architecture diagnostics interpolate error files\n' >&2
    exit 1
  fi
  COUNT=$((COUNT + 1))
}
assert_no_interpolated_error_files

TEST_GATE_WIRING=test_gate_wiring
CHECK_ALL=$ROOT/scripts/check-all.sh
HOOK=$ROOT/lefthook.yml
CHECK_ALL_WITHOUT_GATE=$TASK_TMP/check-all-without-gate.sh
HOOK_WITHOUT_SCOPED_CHECK=$TASK_TMP/lefthook-without-scoped-check.yml
sed '/^  architecture-check$/d' "$CHECK_ALL" >"$CHECK_ALL_WITHOUT_GATE"
sed '/^    architecture:$/,/^    [^ ]/d' "$HOOK" >"$HOOK_WITHOUT_SCOPED_CHECK"
assert_gate_wiring "$CHECK_ALL" "$HOOK"
assert_fails "$TEST_GATE_WIRING" "$CHECK_ALL_WITHOUT_GATE" "$HOOK"
assert_fails "$TEST_GATE_WIRING" "$CHECK_ALL" "$HOOK_WITHOUT_SCOPED_CHECK"

assert_malformed_baseline_is_rejected() {
  local malformed=$TASK_TMP/malformed-baseline.md
  printf '%s\n' '| src/lib.rs | private | none | none | shared::root | src/lib.rs |' >"$malformed"
  if write_baseline "$malformed" "$TASK_TMP/malformed-baseline" >/dev/null 2>&1; then
    printf 'FAIL: baseline parser accepted a malformed inventory row\n' >&2
    exit 1
  fi
  COUNT=$((COUNT + 1))
}
assert_malformed_baseline_is_rejected

new_tree allowed
put src/config.rs 'pub struct Config;'
both 0 '' src/config.rs
put tests/cli__setup.rs 'use crate::cli::setup;'
assert_status 0 '' --file src/lib.rs src/config.rs tests/cli__setup.rs
assert_status 0 '' --file tests/cli__setup.rs

new_tree root_module
put src/business.rs 'pub fn run() {}'
both 1 'unapproved root module' src/business.rs
new_tree root_directory
put src/business/work.rs 'pub fn run() {}'
both 1 'unapproved root directory' src/business/work.rs

new_tree real_domain
domain 'pub fn run() {}'
both 0 '' src/domains/memories.rs
new_tree unknown_domain
put src/domains.rs 'pub mod unexpected;'
put src/domains/unexpected.rs 'pub fn run() {}'
both 1 'unapproved domain' src/domains/unexpected.rs
new_tree declared_unknown_domain
put src/domains.rs 'pub mod unexpected;'
both 1 'unapproved domain' src/domains.rs
new_tree declaration_scaffold
domain 'pub mod save;'
both 1 'empty domain scaffold' src/domains/memories.rs
new_tree readme_scaffold
put src/domains/memories/README.md '# Memories'
both 1 'empty domain scaffold' src/lib.rs
new_tree sibling_shape
put src/domains/memories/save.rs 'pub fn run() {}'
both 1 'missing sibling module' src/domains/memories/save.rs
new_tree bad_nested
domain 'pub fn run() {}'
put src/domains/memories/surprise/nested.rs 'pub fn run() {}'
both 1 'unapproved domain directory' src/domains/memories/surprise/nested.rs
new_tree allowed_nested
put src/domains.rs 'pub mod code;'
put src/domains/code.rs 'pub mod ast;'
put src/domains/code/ast.rs 'pub mod chunk;'
put src/domains/code/ast/chunk.rs 'pub fn run() {}'
both 0 '' src/domains/code/ast/chunk.rs
new_tree missing_namespace
put src/domains/memories.rs 'pub fn work() {}'
both 1 'missing sibling module' src/domains/memories.rs
new_tree empty_namespace
put src/domains/README.md '# Domains'
both 1 'empty domain scaffold' src/lib.rs
new_tree missing_nested_sibling
put src/domains.rs 'pub mod code;'
put src/domains/code.rs 'pub fn run() {}'
put src/domains/code/ast/chunk.rs 'pub fn run() {}'
both 1 'missing sibling module' src/domains/code/ast/chunk.rs

for form in \
  'use crate::cli::run; pub fn work() {}' \
  'use crate::{cli::{run, other}, config::Config}; pub fn work() {}' \
  $'use crate::\n cli::{\n run,\n other\n}; pub fn work() {}' \
  'use super::super::cli::run; pub fn work() {}' \
  'use crate::cli as transport; pub fn work() { transport::run(); }' \
  'pub use crate::cli::run as exported; pub fn work() {}' \
  'pub fn work() { crate::cli::run(); }' \
  'pub fn work() { crate /* ignored */ :: cli :: run(); }' \
  'pub fn work() { crate::r#cli::run(); }' \
  'use ::crate::cli::run; pub fn work() {}' \
  'use crate::{cli as transport}; pub fn work() { transport::run(); }'; do
  new_tree forbidden_import
  domain "$form"
  both 1 'crate::cli' src/domains/memories.rs
done
new_tree mixed_comment_import
domain 'use crate::{/* // ignored */ cli::run}; pub fn work() {}'
both 1 'crate::cli::run' src/domains/memories.rs
new_tree legacy_core
domain 'use crate::api::save; pub fn work() {}'
both 1 'crate::api::save' src/domains/memories.rs
new_tree undeclared_owner_edge
domain 'use crate::domains::sync::run; pub fn work() {}'
both 1 'crate::domains::sync::run' src/domains/memories.rs
new_tree allowed_owner_edge
domain 'use crate::domains::code::run; pub fn work() {}'
both 0 '' src/domains/memories.rs
new_tree ignored_text
domain $'// use crate::cli::run;\npub fn work() { let _ = "crate::serve::run()"; let _ = r#"use crate::cli;"#; }'
put src/domains/memories/tests/ignored.rs 'use crate::cli::run;'
both 0 '' src/domains/memories.rs
new_tree import_comments
domain $'use crate::{config::Config /* cli::run /* nested */ */}; pub fn work() {}'
both 0 '' src/domains/memories.rs

# The fixture file needs an owner that stays put. `assert_status` synthesizes
# each fixture row from the real ledger, so a path whose row has moved falls
# through to `shared::root` and the case stops asserting what it was written to
# assert. These trees named `src/api/search.rs` until #171 took it under
# `domains/retrieval/`, then `src/api/prune.rs` until #176 took it under
# `domains/maintenance/`, then `src/api/setup.rs` until #175 took the
# integrations cores and left `src/api/` with no inventory row at all.
# `src/domains/memories.rs` ends that sequence because it is armed twice over:
# it has a real ledger row owned by `domains::memories`, and the synthesizer's
# own fallback above derives that same owner from any
# `src/domains/<declared-capability>/` path even when no row exists — so no
# later slice can move it out from under these fixtures.
new_tree legacy_exception
domain 'pub fn run() { crate::cli::embedding_input(); }'
policy '.legacy_edges=[{source:"src/domains/memories.rs",target:"crate::cli::embedding_input",class:"delivery",issue:"#166"}]'
both 0 '' src/domains/memories.rs
policy '.legacy_edges=[]'
both 1 'crate::cli::embedding_input' src/domains/memories.rs
new_tree stale_exception
domain 'pub fn run() {}'
policy '.legacy_edges=[{source:"src/domains/memories.rs",target:"crate::cli::embedding_input",class:"delivery",issue:"#166"}]'
both 1 'stale policy edge' src/domains/memories.rs

# The `legacy_modules` mutations are NOT in this loop: with that array empty
# they auto-vivify a half-formed entry and fail the shape check instead of the
# rule under test. They live in `invalid_legacy_module` above, behind a seed.
for mutation in \
  '.legacy_edges += [.legacy_edges[0]]' \
  'del(.legacy_edges[0].target)' \
  '.legacy_edges[0].issue="#999"' \
  '.legacy_edges[0].target="crate::cli::*"' \
  '.legacy_edges[0].class="store-callback"' \
  '.owner_dependencies += [.owner_dependencies[0]]' \
  '.setup_runtime_dependencies=[{source:"src/domains/memories.rs",target:"crate::domains::memories::save",owner:"bogus"}]' \
  '.shared_domain_dependencies=[{source:"src/utilities/when.rs",target:"crate::domains::memories::Ref",owner:"domains::memories",reason:"too short"}]' \
  '.shared_domain_dependencies=[{source:"src/domains/memories.rs",target:"crate::domains::memories::Ref",owner:"domains::memories",reason:"a source outside config/ or utilities/ is not a shared-layer escape at all."}]' \
  '.setup_runtime_dependencies=null'; do
  new_tree invalid_policy
  domain 'pub fn run() { crate::cli::embedding_input(); }'
  policy '.legacy_edges=[{source:"src/domains/memories.rs",target:"crate::cli::embedding_input",class:"delivery",issue:"#166"}]'
  policy "$mutation"
  both 3 'invalid policy' src/domains/memories.rs
done
new_tree malformed_json
put scripts/architecture-policy.json '{broken'
both 3 'invalid policy' src/lib.rs

new_tree store_callback
put src/store/rows.rs 'use crate::domains::graph::derived as algorithm; pub fn write() { algorithm::refresh(); }'
both 1 'crate::domains::graph::derived' src/store/rows.rs
policy '.store_callbacks=[{source:"src/store/rows.rs",target:"crate::domains::graph::derived::refresh",class:"store-callback",issue:"#177"}]'
both 0 '' src/store/rows.rs
new_tree callback_constructor
put src/store/rows.rs 'use crate::domains::documents::source::registry::Registry as Lock; pub fn write() { Lock::acquire(); }'
both 1 'crate::domains::documents::source::registry::Registry' src/store/rows.rs
new_tree passive_model
put src/store/rows.rs 'use crate::domains::memories::Ref; pub fn read() -> Ref { Ref::new("value") }'
policy '.passive_store_models=[{source:"src/store/rows.rs",target:"crate::domains::memories::Ref"}]'
both 0 '' src/store/rows.rs
put src/store/rows.rs 'use crate::domains::memories::Ref; pub fn write() { Ref::reindex(); }'
both 1 'crate::domains::memories::Ref::reindex' src/store/rows.rs
put src/store/rows.rs 'use crate::domains::memories::Ref; pub fn write(model: Ref) { model.reindex(); }'
both 1 'crate::domains::memories::Ref::reindex' src/store/rows.rs
put src/store/rows.rs 'use crate::domains::memories::Ref as Model; fn write(model: &Model) { model.reindex(); }'
both 1 'crate::domains::memories::Ref::reindex' src/store/rows.rs
put src/store/rows.rs "use crate::domains::memories::Ref; fn write<'a>(model: &'a Ref) { model.reindex(); }"
both 1 'crate::domains::memories::Ref::reindex' src/store/rows.rs
put src/store/rows.rs 'use crate::domains::memories::Ref; fn write(model: Ref) { let model = model.reindex(); }'
both 1 'crate::domains::memories::Ref::reindex' src/store/rows.rs
put src/store/rows.rs 'use crate::domains::memories::Ref; fn write(model: Ref) { let model = 7; model.to_string(); }'
both 0 '' src/store/rows.rs
put src/store/rows.rs 'use crate::domains::memories::Ref; fn write(model: Ref) { let model = 7;model.to_string(); }'
both 0 '' src/store/rows.rs
put src/store/rows.rs 'use crate::domains::memories::Ref; fn write() { let model: Ref = Ref::new("value"); model.reindex(); }'
both 1 'crate::domains::memories::Ref::reindex' src/store/rows.rs
put src/store/rows.rs 'use crate::domains::memories::Ref; fn read(model: Ref) {} fn other(model: crate::config::Config) { model.reindex(); }'
both 0 '' src/store/rows.rs
new_tree passive_prefix_escape
put src/store/rows.rs 'use crate::domains::memories::ReferencesService; pub fn write() {}'
policy '.passive_store_models=[{source:"src/store/rows.rs",target:"crate::domains::memories::Ref"}]'
both 1 'crate::domains::memories::ReferencesService' src/store/rows.rs

# Scope must select the actual binding, not the first same-name import in a file.
for body in \
  'use crate::config as model; fn write() { use crate::domains::memories::Ref as model; model::reindex(); }' \
  'fn read() { use crate::config as model; model::read(); } fn write() { use crate::domains::memories::Ref as model; model::reindex(); }' \
  'use crate::domains::memories::Ref; fn write() { self::Ref::reindex(); }' \
  'use {crate::domains::memories::Ref, Ref as model}; fn write() { model::reindex(); }' \
  'use crate::domains::memories::Ref; mod inner { fn write() { super::Ref::reindex(); } }'; do
  new_tree lexical_store_alias
  put src/store/rows.rs "$body"
  policy '.passive_store_models=[{source:"src/store/rows.rs",target:"crate::domains::memories::Ref"}]'
  both 1 'crate::domains::memories::Ref::reindex' src/store/rows.rs
done
new_tree parent_file_alias
put src/store/rows.rs 'use crate::domains::memories::Ref; pub mod child;'
put src/store/rows/child.rs 'fn write() { super::Ref::reindex(); }'
policy '.passive_store_models=[{source:"src/store/rows.rs",target:"crate::domains::memories::Ref"}]'
both 1 'crate::domains::memories::Ref::reindex' src/store/rows/child.rs
new_tree local_alias_shadows_passive
put src/store/rows.rs 'use crate::domains::memories::Ref as model; fn read() { use crate::config as model; model::read(); }'
policy '.passive_store_models=[{source:"src/store/rows.rs",target:"crate::domains::memories::Ref"}]'
both 0 '' src/store/rows.rs
new_tree commented_module
domain 'mod /* explanation */ inner { pub fn work() { super::super::super::cli::run(); } }'
both 1 'crate::cli::run' src/domains/memories.rs
new_tree passive_enum
domain 'pub enum Kind { Note, Tagged(String), Record { value: String } }'
put src/store/rows.rs 'use crate::domains::memories::Kind; fn read() -> Kind { Kind::Note }'
policy '.passive_store_models=[{source:"src/store/rows.rs",target:"crate::domains::memories::Kind"}]'
both 0 '' src/store/rows.rs
put src/store/rows.rs 'use crate::domains::memories::Kind; fn read() -> Kind { Kind::Tagged(String::new()) }'
both 0 '' src/store/rows.rs
for method in reindex Reindex Unknown; do
  put src/store/rows.rs "use crate::domains::memories::Kind; fn write() { Kind::$method(); }"
  both 1 "crate::domains::memories::Kind::$method" src/store/rows.rs
done
new_tree reexported_passive_enum
domain 'pub mod frontmatter; pub use self::frontmatter::Kind;'
put src/domains/memories/frontmatter.rs 'pub enum Kind { Note }'
put src/store/rows.rs 'use crate::domains::memories::Kind; fn read() -> Kind { Kind::Note }'
policy '.passive_store_models=[{source:"src/store/rows.rs",target:"crate::domains::memories::Kind"}]'
both 0 '' src/store/rows.rs

# The shared layer. `config` and `utilities` sit under every capability, so an
# edge back into one inverts the layering — and until #178 nothing watched for
# it, which is how four of them accumulated. `shared::root` stays exempt: the
# crate-root facade's job IS re-exporting domain paths.
new_tree shared_layer
put src/utilities.rs 'pub mod helper;'
put src/utilities/helper.rs 'use crate::domains::memories::Ref; pub fn read() -> Ref { Ref::new() }'
both 1 'shared layer dependency' src/utilities/helper.rs
policy '.shared_domain_dependencies=[{source:"src/utilities/helper.rs",
  target:"crate::domains::memories::Ref",owner:"domains::memories",
  reason:"fixture: a declared shared-to-domain edge is exempt, and only the declared one is."}]'
both 0 '' src/utilities/helper.rs
# The declaration is exact, not a blanket pass for the file.
put src/utilities/helper.rs 'use crate::domains::memories::References; pub fn read() -> References { References::default() }'
both 1 'crate::domains::memories::References' src/utilities/helper.rs
# `src/lib.rs` re-exporting a domain path is the facade doing its job.
new_tree shared_root_facade
put src/lib.rs 'pub use crate::domains::memories::Ref;'
both 0 '' src/lib.rs

# `legacy_modules` emptied out with #178: `crate::api` went with the shell and
# `crate::output` with the move under `cli/`. On an empty array jq auto-vivifies
# `.legacy_modules[0]`, so each mutation below produces a half-formed entry that
# fails the SHAPE check rather than the rule it was written to exercise — the
# duplicate check, the module-path regex and the issue regex all stop being
# reached. Each case therefore seeds one well-formed entry first, exactly as
# `SEED_EDGE` does for `legacy_edges` in test-architecture-policy.sh.
SEED_MODULE='.legacy_modules=[{module:"crate::legacy",owner:"delivery::cli",issue:"#166"}]'
new_tree legacy_module_row
domain 'pub fn run() {}'
policy "$SEED_MODULE"
# The seed alone is schema-valid, so it reaches the NEXT check and fails there:
# every legacy_modules entry needs a ledger row agreeing on owner and issue.
# That is what proves the seed is not passing the mutations below vacuously.
both 1 'invalid ownership policy' src/domains/memories.rs
for mutation in \
  '.legacy_modules += [.legacy_modules[0]]' \
  '.legacy_modules[0].module="crate::api::save"' \
  '.legacy_modules[0].issue="#999"' \
  '.legacy_modules[0].owner="bogus"'; do
  new_tree invalid_legacy_module
  domain 'pub fn run() {}'
  policy "$SEED_MODULE"
  policy "$mutation"
  both 3 'invalid policy' src/domains/memories.rs
done

new_tree grouped_diagnostics
domain 'use crate::{serve::z, cli::{z, a}}; pub fn work() {}'
assert_status 1 'crate::cli::a'
test "$(grep -c '^src/domains/memories.rs:' "$TASK_TMP/result")" = 1
test "$(grep -n 'crate::cli::a' "$TASK_TMP/result" | cut -d: -f1)" -lt "$(grep -n 'crate::serve::z' "$TASK_TMP/result" | cut -d: -f1)"
# Scoped selection reports only the files actually selected. #178 retired the
# checker's `src/api/` source branch with the shell it guarded, so this case now
# carries its violation on a real capability file: the clean parent stays silent
# when selected alone, and naming both files reports the child. (Selecting a
# CHILD also selects its parent module file — that direction is `scoped_parent`
# below; this one proves the parent does not drag in its children.)
new_tree scoped_selection
domain 'pub fn work() {}'
put src/domains/memories/save.rs 'pub fn run() { crate::cli::embedding_input(); }'
put tests/cli__setup.rs 'use crate::cli::setup;'
assert_status 0 '' --file tests/cli__setup.rs
assert_status 0 '' --file src/domains/memories.rs
assert_status 1 'crate::cli::embedding_input' --file src/domains/memories.rs --file src/domains/memories/save.rs
# A crate-root re-export must be written `pub use crate::…`. rustc treats the
# uniform path as the same export, but the checker's resolver rewrites an
# unqualified binding to a relative path and then drops the edge — which is how
# a domain reaching delivery THROUGH an alias stayed invisible for twelve
# slices. Both halves are asserted: the rule fires on the unqualified form, and
# the qualified form actually makes the aliased edge visible.
new_tree unqualified_reexport
domain 'pub fn work() {}'
put src/lib.rs 'pub use domains::memories;'
both 1 'unqualified root re-export' src/lib.rs
put src/lib.rs 'pub use crate::domains::memories;'
both 0 '' src/lib.rs
# `pub(crate) use` loses the binding the same way, so it counts too.
put src/lib.rs 'pub(crate) use domains::memories;'
both 1 'unqualified root re-export' src/lib.rs
# Re-exporting a DEPENDENCY is not the same thing: there is no in-crate path
# for the resolver to lose, so the rule must stay quiet.
put src/lib.rs 'pub use serde::Serialize;'
both 0 '' src/lib.rs
new_tree aliased_delivery
put src/lib.rs 'pub use crate::cli::output;'
domain 'pub fn work() { crate::output::json::write(); }'
both 1 'crate::cli::output::json::write' src/domains/memories.rs
# The same tree with the unqualified alias: the delivery edge disappears
# entirely, and only the new rule reports anything at all.
put src/lib.rs 'pub use cli::output;'
assert_status 1 'unqualified root re-export'
test "$(grep -c 'crate::cli::output' "$TASK_TMP/result")" = 0

new_tree scoped_parent
domain 'use crate::cli; pub mod save;'
put src/domains/memories/save.rs 'pub fn work() {}'
assert_status 1 'crate::cli' --file src/domains/memories/save.rs
new_tree namespace_import
domain 'pub fn work() {}'
put src/domains.rs 'pub mod memories; pub use crate::cli as transport;'
both 1 'crate::cli' src/domains/memories.rs
new_tree arguments
assert_status 3 'argument' --unknown
assert_status 3 'argument' --file
assert_status 3 'missing input' --file src/missing.rs
assert_status 3 'invalid file argument' --file src/../tests/cli__setup.rs
assert_status 3 'missing input' --policy "$TASK_TMP/missing.json"
assert_status 3 'missing input' --inventory "$TASK_TMP/missing.md"
assert_status 0 '' --policy "$TREE/scripts/architecture-policy.json" --inventory "$TREE/docs/designs/2026-09-17-domain-first-migration-inventory.md"
# GitHub's review runner does not install ripgrep. The architecture gate must
# retain its complete validation with the portable tools it already requires.
mkdir "$TASK_TMP/no-rg-path"
for tool in bash basename cat diff dirname find jq mktemp rm sort ast-grep; do
  ln -s "$(command -v "$tool")" "$TASK_TMP/no-rg-path/$tool"
done
PATH="$TASK_TMP/no-rg-path" FIXTURE_INVENTORY_READY=true assert_status 0 ''
# Keep the actual shell and dirname, but deliberately omit every checker tool.
mkdir "$TASK_TMP/tool-path"
ln -s "$(command -v bash)" "$TASK_TMP/tool-path/bash"
ln -s "$(command -v dirname)" "$TASK_TMP/tool-path/dirname"
ln -s "$(command -v grep)" "$TASK_TMP/tool-path/grep"
ln -s "$(command -v cat)" "$TASK_TMP/tool-path/cat"
PATH="$TASK_TMP/tool-path" FIXTURE_INVENTORY_READY=true assert_status 3 'missing tool'
printf 'PASS: %s architecture fixture assertions\n' "$COUNT"
