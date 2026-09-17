#!/usr/bin/env bash
# Real source-tree fixtures: weakening any boundary must fail a matching case.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
CHECK="$ROOT/scripts/architecture-check.sh"
TASK_TMP=$(mktemp -d)
trap 'rm -rf "$TASK_TMP"' EXIT
COUNT=0

new_tree() {
  TREE="$TASK_TMP/$1"
  mkdir -p "$TREE/src" "$TREE/scripts" "$TREE/docs/designs"
  jq '.legacy_edges=[] | .store_callbacks=[] | .passive_store_models=[] |
      .setup_runtime_dependencies=[]' "$ROOT/scripts/architecture-policy.json" >"$TREE/scripts/architecture-policy.json"
  cp "$ROOT/docs/designs/2026-09-17-domain-first-migration-inventory.md" "$TREE/docs/designs/2026-09-17-domain-first-migration-inventory.md"
  put src/lib.rs '//! Fixture crate.'
}
put() { mkdir -p "$TREE/$(dirname "$1")"; printf '%s\n' "$2" >"$TREE/$1"; }
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

new_tree allowed
put src/config.rs 'pub struct Config;'
both 0 '' src/config.rs

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

new_tree legacy_exception
put src/api/save.rs 'pub fn run() { crate::cli::embedding_input(); }'
policy '.legacy_edges=[{source:"src/api/save.rs",target:"crate::cli::embedding_input",class:"delivery",issue:"#166"}]'
both 0 '' src/api/save.rs
policy '.legacy_edges=[]'
both 1 'crate::cli::embedding_input' src/api/save.rs
new_tree stale_exception
put src/api/save.rs 'pub fn run() {}'
policy '.legacy_edges=[{source:"src/api/save.rs",target:"crate::cli::embedding_input",class:"delivery",issue:"#166"}]'
both 1 'stale policy edge' src/api/save.rs

for mutation in \
  '.legacy_edges += [.legacy_edges[0]]' \
  'del(.legacy_edges[0].target)' \
  '.legacy_edges[0].issue="#999"' \
  '.legacy_edges[0].target="crate::cli::*"' \
  '.legacy_edges[0].class="store-callback"' \
  '.legacy_modules += [.legacy_modules[0]]' \
  '.owner_dependencies += [.owner_dependencies[0]]' \
  '.legacy_modules[0].module="crate::api::save"' \
  '.legacy_modules[0].issue="#999"' \
  '.setup_runtime_dependencies=[{source:"src/api/setup.rs",target:"crate::api::save",owner:"bogus"}]' \
  '.setup_runtime_dependencies=null'; do
  new_tree invalid_policy
  put src/api/save.rs 'pub fn run() { crate::cli::embedding_input(); }'
  policy '.legacy_edges=[{source:"src/api/save.rs",target:"crate::cli::embedding_input",class:"delivery",issue:"#166"}]'
  policy "$mutation"
  both 3 'invalid policy' src/api/save.rs
done
new_tree malformed_json
put scripts/architecture-policy.json '{broken'
both 3 'invalid policy' src/lib.rs

new_tree store_callback
put src/store/rows.rs 'use crate::graph::derived as algorithm; pub fn write() { algorithm::refresh(); }'
both 1 'crate::graph::derived' src/store/rows.rs
policy '.store_callbacks=[{source:"src/store/rows.rs",target:"crate::graph::derived::refresh",class:"store-callback",issue:"#177"}]'
both 0 '' src/store/rows.rs
new_tree callback_constructor
put src/store/rows.rs 'use crate::source::lock::FileLock as Lock; pub fn write() { Lock::acquire(); }'
both 1 'crate::source::lock::FileLock' src/store/rows.rs
new_tree passive_model
put src/store/rows.rs 'use crate::memory::Ref; pub fn read() -> Ref { Ref::new("value") }'
policy '.passive_store_models=[{source:"src/store/rows.rs",target:"crate::memory::Ref"}]'
both 0 '' src/store/rows.rs
put src/store/rows.rs 'use crate::memory::Ref; pub fn write() { Ref::reindex(); }'
both 1 'crate::memory::Ref::reindex' src/store/rows.rs
new_tree passive_prefix_escape
put src/store/rows.rs 'use crate::memory::ReferencesService; pub fn write() {}'
policy '.passive_store_models=[{source:"src/store/rows.rs",target:"crate::memory::Ref"}]'
both 1 'crate::memory::ReferencesService' src/store/rows.rs

# Scope must select the actual binding, not the first same-name import in a file.
for body in \
  'use crate::config as model; fn write() { use crate::memory::Ref as model; model::reindex(); }' \
  'fn read() { use crate::config as model; model::read(); } fn write() { use crate::memory::Ref as model; model::reindex(); }' \
  'use crate::memory::Ref; fn write() { self::Ref::reindex(); }' \
  'use {crate::memory::Ref, Ref as model}; fn write() { model::reindex(); }' \
  'use crate::memory::Ref; mod inner { fn write() { super::Ref::reindex(); } }'; do
  new_tree lexical_store_alias
  put src/store/rows.rs "$body"
  policy '.passive_store_models=[{source:"src/store/rows.rs",target:"crate::memory::Ref"}]'
  both 1 'crate::memory::Ref::reindex' src/store/rows.rs
done
new_tree parent_file_alias
put src/store/rows.rs 'use crate::memory::Ref; pub mod child;'
put src/store/rows/child.rs 'fn write() { super::Ref::reindex(); }'
policy '.passive_store_models=[{source:"src/store/rows.rs",target:"crate::memory::Ref"}]'
both 1 'crate::memory::Ref::reindex' src/store/rows/child.rs
new_tree local_alias_shadows_passive
put src/store/rows.rs 'use crate::memory::Ref as model; fn read() { use crate::config as model; model::read(); }'
policy '.passive_store_models=[{source:"src/store/rows.rs",target:"crate::memory::Ref"}]'
both 0 '' src/store/rows.rs
new_tree commented_module
domain 'mod /* explanation */ inner { pub fn work() { super::super::super::cli::run(); } }'
both 1 'crate::cli::run' src/domains/memories.rs
new_tree passive_enum
put src/memory.rs 'pub enum Kind { Note, Tagged(String), Record { value: String } }'
put src/store/rows.rs 'use crate::memory::Kind; fn read() -> Kind { Kind::Note }'
policy '.passive_store_models=[{source:"src/store/rows.rs",target:"crate::memory::Kind"}]'
both 0 '' src/store/rows.rs
put src/store/rows.rs 'use crate::memory::Kind; fn read() -> Kind { Kind::Tagged(String::new()) }'
both 0 '' src/store/rows.rs
for method in reindex Reindex Unknown; do
  put src/store/rows.rs "use crate::memory::Kind; fn write() { Kind::$method(); }"
  both 1 "crate::memory::Kind::$method" src/store/rows.rs
done
new_tree reexported_passive_enum
put src/memory.rs 'pub mod frontmatter; pub use self::frontmatter::Kind;'
put src/memory/frontmatter.rs 'pub enum Kind { Note }'
put src/store/rows.rs 'use crate::memory::Kind; fn read() -> Kind { Kind::Note }'
policy '.passive_store_models=[{source:"src/store/rows.rs",target:"crate::memory::Kind"}]'
both 0 '' src/store/rows.rs

new_tree grouped_diagnostics
domain 'use crate::{serve::z, cli::{z, a}}; pub fn work() {}'
assert_status 1 'crate::cli::a'
test "$(grep -c '^src/domains/memories.rs:' "$TASK_TMP/result")" = 1
test "$(grep -n 'crate::cli::a' "$TASK_TMP/result" | cut -d: -f1)" -lt "$(grep -n 'crate::serve::z' "$TASK_TMP/result" | cut -d: -f1)"
new_tree scoped_selection
domain 'pub fn work() {}'
put src/api/save.rs 'pub fn run() { crate::cli::embedding_input(); }'
assert_status 0 '' --file src/domains/memories.rs
assert_status 1 'crate::cli::embedding_input' --file src/domains/memories.rs --file src/api/save.rs
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
assert_status 3 'missing input' --policy "$TASK_TMP/missing.json"
assert_status 3 'missing input' --inventory "$TASK_TMP/missing.md"
assert_status 0 '' --policy "$TREE/scripts/architecture-policy.json" --inventory "$TREE/docs/designs/2026-09-17-domain-first-migration-inventory.md"
# Keep the actual shell and dirname, but deliberately omit every checker tool.
mkdir "$TASK_TMP/tool-path"
ln -s "$(command -v bash)" "$TASK_TMP/tool-path/bash"
ln -s "$(command -v dirname)" "$TASK_TMP/tool-path/dirname"
ln -s "$(command -v grep)" "$TASK_TMP/tool-path/grep"
ln -s "$(command -v cat)" "$TASK_TMP/tool-path/cat"
PATH="$TASK_TMP/tool-path" assert_status 3 'missing tool'
printf 'PASS: %s architecture fixture assertions\n' "$COUNT"
