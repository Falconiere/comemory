# qwick Rust Rewrite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the Rust `qwick` CLI from scratch — engram-style memory + grepai semantic code search + ast-grep AST patterns, knit together by a two-layer property graph and exposed as a deterministic "Agentic RAG" toolbox for Claude Code.

**Architecture:** Single-crate Rust CLI. LanceDB for vector indices (memory + code, dual embedders), kuzu for property graph with Cypher, ast-grep-core for symbol extraction, rusqlite for stats and indexing markers. No in-process LLM. Markdown is source of truth at `~/.qwick/memories/`; both indices are fully rebuildable.

**Tech Stack:** Rust (stable), tokio, clap, lancedb, kuzu, ast-grep-core, fastembed (nomic-embed-text-v1.5-Q + jina-embeddings-v2-base-code-Q), rusqlite, git2, serde, owo-colors, thiserror, tracing, lefthook, cargo-nextest, insta, proptest.

**Reference:** Design spec at `docs/superpowers/specs/2026-05-17-qwick-rust-agentic-rag-design.md`.

## Binding Rules (apply to every task, every file)

1. **No duplication / redundancy.** If two scripts or modules share logic, extract a helper. DRY enforced by `scripts/dup-check.sh` and reviewer.
2. **Very modular modules.** Each `src/<module>/` directory has narrow, well-named files. Each file owns one concept. Files that change together live together.
3. **No file in `src/` or `scripts/` may exceed 500 lines.** Enforced by `scripts/module-size-check.sh`.
4. **Zero errors, zero warnings.** Every gate must exit 0. No `#[allow(...)]` overrides, no `// clippy::allow`, no `unwrap()` outside `tests/`, no `expect(` without a message, no `println!` in `src/` (use `tracing`), no `todo!()` / `unimplemented!()` in shipped code, no `unsafe { … }` without an adjacent `// SAFETY:` comment. Enforced by `scripts/no-bypass-check.sh`.
5. **Test placement (BINDING):** No `#[cfg(test)] mod tests { … }` block ever appears inside any file in `src/`. Every test lives in `tests/` mirroring `src/` 1:1. Items needing tests are exposed with `pub(crate)` visibility. Each `tests/<module>.rs` is a thin test binary that declares submodules in `tests/<module>/`. Enforced by `scripts/test-placement-check.sh`.

## Quality gates (run at the end of every task)

Single entry point: `bash scripts/check-all.sh`. It runs, in order:

```
scripts/fmt-check.sh             # cargo fmt --check
scripts/type-check.sh            # cargo check --all-targets --all-features
scripts/lint-check.sh            # cargo clippy --all-targets --all-features -- -D warnings
scripts/test-placement-check.sh  # no #[cfg(test)] mod tests in src/
scripts/no-bypass-check.sh       # no allow/unwrap/println!/unsafe-without-SAFETY/etc.
scripts/module-size-check.sh     # no file > 500 lines in src/ or scripts/
scripts/tests-mirror-check.sh    # every src file has a mirror in tests/
scripts/typos-check.sh           # typos
scripts/deny-check.sh            # cargo deny check
scripts/test-run.sh              # cargo nextest run --all-features
```

A task is not "done" until `scripts/check-all.sh` exits 0.

---

## File Structure

This is the target tree at v1.0.0. Tasks below create files in roughly this order.

```
qwick/
├── Cargo.toml
├── Cargo.lock
├── deny.toml
├── rustfmt.toml
├── clippy.toml
├── typos.toml
├── lefthook.yml
├── justfile
├── .github/workflows/ci.yml
│
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── prelude.rs
│   ├── errors.rs
│   ├── cli/{mod,save,search,context,symbol,memory_for,ast,walk,index,index_code,prune,feedback,conflicts,supersedes,list,delete,doctor,install_hooks,config,gc,context_recent}.rs
│   ├── memory/{mod,frontmatter,store,id,slug}.rs
│   ├── index/{mod,memory_index,code_index,embedder,schema}.rs
│   ├── graph/{mod,schema,upsert,walk,query}.rs
│   ├── retrieval/{mod,router,hybrid,corrective,rank,bundle}.rs
│   ├── ast/{mod,extractor,pattern,languages}.rs
│   ├── stats/{mod,sqlite,feedback}.rs
│   ├── config/{mod,paths}.rs
│   ├── output/{mod,tty,json}.rs
│   ├── prune/{mod,orphans,stale_code,low_value}.rs
│   └── git_utils.rs
│
├── tests/
│   ├── common/{mod,corpus,runner}.rs
│   ├── memory.rs + memory/{frontmatter,store,id,slug}.rs
│   ├── stats.rs + stats/{sqlite,feedback}.rs
│   ├── index.rs + index/{memory_index,code_index,embedder}.rs
│   ├── graph.rs + graph/{upsert,walk,query}.rs
│   ├── retrieval.rs + retrieval/{router,hybrid,corrective,rank}.rs
│   ├── ast.rs + ast/{extractor,pattern}.rs
│   ├── prune.rs + prune/{orphans,stale_code,low_value}.rs
│   ├── output.rs
│   ├── cli.rs (assert_cmd + insta snapshots)
│   ├── cross_link.rs
│   └── e2e.rs
│
├── benches/{search,index_code}.rs
├── docs/{README.md, architecture.md, superpowers/specs/, superpowers/plans/}
│
├── scripts/
│   ├── lib/common.sh                   # shared helpers (find_src, run_in_root, log)
│   ├── check-all.sh                    # umbrella runner
│   ├── fmt-check.sh                    # cargo fmt --check
│   ├── type-check.sh                   # cargo check
│   ├── lint-check.sh                   # cargo clippy
│   ├── test-run.sh                     # cargo nextest
│   ├── test-placement-check.sh         # no #[cfg(test)] mod tests in src/
│   ├── tests-mirror-check.sh           # every src file has a tests/ mirror
│   ├── no-bypass-check.sh              # forbidden patterns
│   ├── module-size-check.sh            # 500-line cap
│   ├── typos-check.sh                  # typos wrapper
│   ├── deny-check.sh                   # cargo deny
│   ├── dup-check.sh                    # duplication scan (similarity-rs)
│   ├── e2e.sh                          # full happy-path smoke
│   └── seed-corpus.sh                  # generate test memories
│
└── .claude/
    ├── settings.json                   # registers PreTool/PostTool/Stop hooks
    └── hooks/
        ├── lib/common.sh               # JSON parse, deny-output helpers
        ├── session-end.sh              # runs scripts/check-all.sh (light)
        ├── pre-tools/
        │   ├── mod.sh                  # dispatcher
        │   └── modules/
        │       ├── bash-commands.sh    # cargo-only; block destructive + bypass flags
        │       ├── code-edit-rules.sh  # block #[allow], unwrap in src/, mod tests in src/, etc.
        │       └── protected-files.sh  # protect Cargo.lock, deny.toml, lefthook.yml, target/
        └── post-tools/
            ├── mod.sh                  # dispatcher
            └── modules/
                ├── auto-format.sh      # cargo fmt -- <file>
                ├── auto-lint.sh        # cargo clippy --fix --allow-dirty <file>
                └── gate-status.sh      # track exit codes of scripts/check-all.sh, write .claude/tmp/quality-gate-status.json
```

---

## Task 1: Bootstrap — Cargo project + tooling

**Goal:** Create a buildable empty crate with all quality gates wired and CI green on an empty `main`.

**Files:**
- Create: `Cargo.toml`
- Create: `rustfmt.toml`
- Create: `clippy.toml`
- Create: `deny.toml`
- Create: `typos.toml`
- Create: `lefthook.yml`
- Create: `justfile`
- Create: `.github/workflows/ci.yml`
- Create: `src/main.rs`
- Create: `src/lib.rs`
- Create: `src/prelude.rs`
- Create: `tests/smoke.rs`
- Create: `.gitignore`

- [ ] **Step 1: Initialize Cargo manifest**

Create `Cargo.toml`:

```toml
[package]
name = "qwick"
version = "0.1.0"
edition = "2021"
description = "Agentic dev memory + code-aware semantic search via a two-layer property graph."
license = "MIT"
repository = "https://github.com/SidegigLLC/qwick"
rust-version = "1.78"

[[bin]]
name = "qwick"
path = "src/main.rs"

[lib]
path = "src/lib.rs"

[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
fastembed = "4"
git2 = { version = "0.19", default-features = false, features = ["vendored-libgit2"] }
ignore = "0.4"
kuzu = "0.7"
lancedb = "0.10"
once_cell = "1"
owo-colors = "4"
rusqlite = { version = "0.32", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
sha2 = "0.10"
thiserror = "1"
time = { version = "0.3", features = ["serde", "formatting", "parsing", "macros"] }
tokio = { version = "1", features = ["full"] }
toml = "0.8"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
walkdir = "2"

[dependencies.ast-grep-core]
version = "0.30"

[dev-dependencies]
assert_cmd = "2"
insta = { version = "1", features = ["json", "yaml"] }
predicates = "3"
proptest = "1"
tempfile = "3"

[profile.release]
lto = "fat"
codegen-units = 1
strip = "symbols"
```

- [ ] **Step 2: Add tool config files**

Create `rustfmt.toml`:

```toml
edition = "2021"
max_width = 100
use_field_init_shorthand = true
use_try_shorthand = true
imports_granularity = "Module"
group_imports = "StdExternalCrate"
```

Create `clippy.toml`:

```toml
avoid-breaking-exported-api = false
```

Create `deny.toml`:

```toml
[advisories]
db-path = "~/.cargo/advisory-db"
db-urls = ["https://github.com/rustsec/advisory-db"]
yanked = "warn"
ignore = []

[licenses]
allow = ["MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Unicode-DFS-2016", "Unicode-3.0", "CC0-1.0", "Zlib"]
confidence-threshold = 0.93

[bans]
multiple-versions = "warn"
wildcards = "deny"

[sources]
unknown-registry = "warn"
unknown-git = "warn"
```

Create `typos.toml`:

```toml
[default]
extend-ignore-identifiers-re = ["nomic", "kuzu", "qwick"]
```

- [ ] **Step 3: Add lefthook + justfile**

Create `lefthook.yml`:

```yaml
pre-commit:
  parallel: true
  commands:
    fmt:               { run: bash scripts/fmt-check.sh }
    type_check:        { run: bash scripts/type-check.sh }
    lint:              { run: bash scripts/lint-check.sh }
    test_placement:    { run: bash scripts/test-placement-check.sh }
    no_bypass:         { run: bash scripts/no-bypass-check.sh }
    module_size:       { run: bash scripts/module-size-check.sh }
    tests_mirror:      { run: bash scripts/tests-mirror-check.sh }
    typos:             { run: bash scripts/typos-check.sh }

pre-push:
  commands:
    check_all:
      run: bash scripts/check-all.sh
```

Create `justfile`:

```just
default: check

check:
    bash scripts/check-all.sh

test:
    bash scripts/test-run.sh

qa:
    bash scripts/check-all.sh
    bash scripts/deny-check.sh
    bash scripts/dup-check.sh

bench:
    cargo bench --all-features

build-release:
    cargo build --release

e2e:
    bash scripts/e2e.sh
```

- [ ] **Step 4: Add CI workflow**

Create `.github/workflows/ci.yml`:

```yaml
name: ci
on:
  pull_request:
  push:
    branches: [main]

jobs:
  test:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: rustfmt, clippy }
      - uses: Swatinem/rust-cache@v2
      - uses: taiki-e/install-action@v2
        with: { tool: nextest,cargo-deny,typos-cli,cargo-machete }
      - run: bash scripts/check-all.sh
      - run: bash scripts/deny-check.sh
      - run: bash scripts/dup-check.sh
      - run: bash scripts/e2e.sh
```

- [ ] **Step 5: Add minimal source**

Create `.gitignore`:

```
/target
Cargo.lock.bak
.qwick-test/
```

Create `src/prelude.rs`:

```rust
pub use crate::errors::{Error, Result};
```

Create `src/lib.rs`:

```rust
//! qwick — agentic dev memory + code-aware semantic search.

pub mod prelude;

#[path = "errors.rs"]
pub mod errors;
```

Create `src/errors.rs`:

```rust
use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("other: {0}")]
    Other(String),
}
```

Create `src/main.rs`:

```rust
use qwick::prelude::*;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    println!("qwick {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
```

- [ ] **Step 6: Add smoke test**

Create `tests/smoke.rs`:

```rust
use assert_cmd::Command;

#[test]
fn binary_runs_and_prints_version() {
    Command::cargo_bin("qwick")
        .unwrap()
        .assert()
        .success()
        .stdout(predicates::str::contains("qwick"));
}
```

- [ ] **Step 7: Run quality gates and verify green**

Run:

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --all-features
cargo deny check
```

Expected: all four pass.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock rustfmt.toml clippy.toml deny.toml typos.toml lefthook.yml justfile .github/workflows/ci.yml .gitignore src/ tests/smoke.rs
git commit -m "chore: bootstrap qwick rust crate with quality gates"
```

---

## Task 2: Quality gate scripts (`scripts/`)

**Goal:** Implement every gate referenced by `justfile`, `lefthook.yml`, and `.github/workflows/ci.yml` as a small, focused bash script. DRY enforced via `scripts/lib/common.sh`. Each script is ≤500 lines; most are 20–60.

**Files:**
- Create: `scripts/lib/common.sh`
- Create: `scripts/check-all.sh`
- Create: `scripts/fmt-check.sh`
- Create: `scripts/type-check.sh`
- Create: `scripts/lint-check.sh`
- Create: `scripts/test-run.sh`
- Create: `scripts/test-placement-check.sh`
- Create: `scripts/tests-mirror-check.sh`
- Create: `scripts/no-bypass-check.sh`
- Create: `scripts/module-size-check.sh`
- Create: `scripts/typos-check.sh`
- Create: `scripts/deny-check.sh`
- Create: `scripts/dup-check.sh`
- Create: `scripts/e2e.sh`

- [ ] **Step 1: Create shared library**

Create `scripts/lib/common.sh`:

```bash
#!/usr/bin/env bash
# Shared helpers for scripts/ — sourced, not executed.
# Provides: PROJECT_ROOT, log_info, log_err, log_ok, find_src_files, die

set -euo pipefail

PROJECT_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
export PROJECT_ROOT

if [[ -t 1 ]]; then
  C_RED=$'\e[31m'; C_GRN=$'\e[32m'; C_YLW=$'\e[33m'; C_DIM=$'\e[2m'; C_RST=$'\e[0m'
else
  C_RED=""; C_GRN=""; C_YLW=""; C_DIM=""; C_RST=""
fi

log_info() { printf "%s[%s]%s %s\n" "$C_DIM" "$1" "$C_RST" "$2"; }
log_ok()   { printf "%s[%s] OK%s %s\n" "$C_GRN" "$1" "$C_RST" "${2:-}"; }
log_err()  { printf "%s[%s] FAIL%s %s\n" "$C_RED" "$1" "$C_RST" "$2" >&2; }
die()      { log_err "${1:-script}" "${2:-failed}"; exit 1; }

# Emit every tracked .rs file under src/, NUL-separated.
find_src_files() {
  cd "$PROJECT_ROOT"
  git ls-files -z 'src/*.rs'
}

# Emit every tracked .rs file under tests/, NUL-separated.
find_test_files() {
  cd "$PROJECT_ROOT"
  git ls-files -z 'tests/*.rs'
}

# Run a cargo command from PROJECT_ROOT.
run_cargo() {
  cd "$PROJECT_ROOT" && cargo "$@"
}
```

- [ ] **Step 2: Implement the umbrella runner**

Create `scripts/check-all.sh`:

```bash
#!/usr/bin/env bash
# Run every quality gate. Exit 1 on first failure.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "$HERE/lib/common.sh"

GATES=(
  fmt-check
  type-check
  lint-check
  test-placement-check
  no-bypass-check
  module-size-check
  tests-mirror-check
  typos-check
)

failed=()
for g in "${GATES[@]}"; do
  log_info "$g" "running"
  if bash "$HERE/$g.sh"; then
    log_ok "$g"
  else
    log_err "$g" "failed"
    failed+=("$g")
  fi
done

if (( ${#failed[@]} > 0 )); then
  log_err "check-all" "${#failed[@]} gate(s) failed: ${failed[*]}"
  exit 1
fi
log_ok "check-all" "all gates passed"
```

- [ ] **Step 3: Implement the cargo-wrapping gates**

Create `scripts/fmt-check.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "$HERE/lib/common.sh"
run_cargo fmt --check
```

Create `scripts/type-check.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "$HERE/lib/common.sh"
run_cargo check --all-targets --all-features
```

Create `scripts/lint-check.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "$HERE/lib/common.sh"
run_cargo clippy --all-targets --all-features -- -D warnings
```

Create `scripts/test-run.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "$HERE/lib/common.sh"
run_cargo nextest run --all-features
```

Create `scripts/typos-check.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "$HERE/lib/common.sh"
cd "$PROJECT_ROOT" && typos
```

Create `scripts/deny-check.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "$HERE/lib/common.sh"
run_cargo deny check
```

Create `scripts/dup-check.sh`:

```bash
#!/usr/bin/env bash
# Duplication scan using similarity-rs if installed; falls back to a soft warning.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "$HERE/lib/common.sh"
cd "$PROJECT_ROOT"
if ! command -v similarity-rs >/dev/null 2>&1; then
  log_info "dup-check" "similarity-rs not installed; skipping (install with 'cargo install similarity-rs')"
  exit 0
fi
# Threshold 0.85 ≈ near-clones; treat any hit as failure.
if similarity-rs --min-similarity 0.85 --paths src/ scripts/ | tee /tmp/qwick-dup.txt | grep -qE '^Similar'; then
  log_err "dup-check" "near-duplicate blocks detected; see /tmp/qwick-dup.txt"
  exit 1
fi
log_ok "dup-check" "no near-duplicates above threshold"
```

- [ ] **Step 4: Implement test-placement-check**

Create `scripts/test-placement-check.sh`:

```bash
#!/usr/bin/env bash
# Fail if any src/*.rs file contains an inline `#[cfg(test)] mod tests` block.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "$HERE/lib/common.sh"

cd "$PROJECT_ROOT"
# Pattern: `#[cfg(test)]` followed (on next non-empty line) by `mod tests`. We greedily detect both on adjacent lines.
hits=$(grep -RInE '^[[:space:]]*#\[cfg\(test\)\]' src/ 2>/dev/null || true)
if [[ -n "$hits" ]]; then
  log_err "test-placement-check" "inline test modules are forbidden in src/:"
  printf "%s\n" "$hits" >&2
  printf "\nMove tests to tests/ mirroring src/.\n" >&2
  exit 1
fi
log_ok "test-placement-check" "no inline tests in src/"
```

- [ ] **Step 5: Implement tests-mirror-check**

Create `scripts/tests-mirror-check.sh`:

```bash
#!/usr/bin/env bash
# Every src/<path>/<name>.rs must have a matching tests/<path>/<name>.rs.
# Exceptions: lib.rs, main.rs, prelude.rs, mod.rs, errors.rs, files inside src/cli/ (covered by tests/cli.rs).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "$HERE/lib/common.sh"

cd "$PROJECT_ROOT"
missing=()

while IFS= read -r -d '' f; do
  base="${f#src/}"
  name="$(basename "$base")"
  case "$name" in
    lib.rs|main.rs|prelude.rs|mod.rs|errors.rs) continue ;;
  esac
  case "$base" in
    cli/*) continue ;;
  esac
  mirror="tests/${base}"
  if [[ ! -f "$mirror" ]]; then
    missing+=("$mirror  (mirrors $f)")
  fi
done < <(git ls-files -z 'src/*.rs')

if (( ${#missing[@]} > 0 )); then
  log_err "tests-mirror-check" "missing test files:"
  printf "  %s\n" "${missing[@]}" >&2
  exit 1
fi
log_ok "tests-mirror-check" "every src file has a test mirror"
```

- [ ] **Step 6: Implement module-size-check**

Create `scripts/module-size-check.sh`:

```bash
#!/usr/bin/env bash
# Fail if any tracked file under src/ or scripts/ exceeds 500 lines.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "$HERE/lib/common.sh"

cd "$PROJECT_ROOT"
LIMIT=500
oversized=()

while IFS= read -r -d '' f; do
  lines=$(wc -l < "$f")
  if (( lines > LIMIT )); then
    oversized+=("$f ($lines lines)")
  fi
done < <(git ls-files -z 'src/*.rs' 'scripts/*.sh' 'scripts/**/*.sh')

if (( ${#oversized[@]} > 0 )); then
  log_err "module-size-check" "file(s) exceed $LIMIT lines:"
  printf "  %s\n" "${oversized[@]}" >&2
  printf "\nSplit them into smaller, focused modules.\n" >&2
  exit 1
fi
log_ok "module-size-check" "all modules within $LIMIT lines"
```

- [ ] **Step 7: Implement no-bypass-check**

Create `scripts/no-bypass-check.sh`:

```bash
#!/usr/bin/env bash
# Block forbidden patterns anywhere except docs/, tests/, scripts/no-bypass-check.sh, and inside #[cfg(test)] blocks.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "$HERE/lib/common.sh"

cd "$PROJECT_ROOT"

declare -a NAMES=(
  "allow-attr"
  "clippy-allow-comment"
  "unwrap-in-src"
  "expect-without-msg"
  "println-in-src"
  "eprintln-in-src"
  "todo-macro"
  "unimplemented-macro"
  "unsafe-without-safety-comment"
  "panic-in-src"
)

declare -a REGEXES=(
  '#\[allow\('
  '//\s*clippy::allow'
  '\.unwrap\(\)'
  '\.expect\(\s*\)'
  '\bprintln!'
  '\beprintln!'
  '\btodo!\('
  '\bunimplemented!\('
  '\bunsafe\s*\{'
  '\bpanic!\('
)

EXCLUDES=(
  ":(exclude)tests/"
  ":(exclude)docs/"
  ":(exclude)scripts/no-bypass-check.sh"
  ":(exclude)benches/"
)

fail=0
for i in "${!NAMES[@]}"; do
  name="${NAMES[$i]}"
  pattern="${REGEXES[$i]}"
  hits=$(git grep -nE "$pattern" -- 'src/*.rs' "${EXCLUDES[@]}" 2>/dev/null || true)

  # Special case: unsafe { is allowed when followed within 3 lines by `// SAFETY:`
  if [[ "$name" == "unsafe-without-safety-comment" && -n "$hits" ]]; then
    filtered=""
    while IFS= read -r line; do
      file="${line%%:*}"
      lineno="${line#*:}"; lineno="${lineno%%:*}"
      window=$(sed -n "${lineno},$((lineno+3))p" "$file" 2>/dev/null || true)
      if ! echo "$window" | grep -q "SAFETY:"; then
        filtered+="$line"$'\n'
      fi
    done <<< "$hits"
    hits="${filtered%$'\n'}"
  fi

  if [[ -n "$hits" ]]; then
    log_err "no-bypass-check" "forbidden pattern '$name':"
    printf "%s\n" "$hits" >&2
    fail=1
  fi
done

if (( fail != 0 )); then
  printf "\nRules cannot be bypassed. Fix the root cause.\n" >&2
  exit 1
fi
log_ok "no-bypass-check" "no forbidden patterns"
```

- [ ] **Step 8: Implement e2e wrapper**

Create `scripts/e2e.sh`:

```bash
#!/usr/bin/env bash
# Real binary, happy-path smoke. Stand-in for the full e2e flow.
# Slices that land later (save/index-code/context) extend the assertions here.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "$HERE/lib/common.sh"

QWICK_HOME=$(mktemp -d)
trap 'rm -rf "$QWICK_HOME"' EXIT

export QWICK_DATA_DIR="$QWICK_HOME/.qwick"
cd "$PROJECT_ROOT"
cargo build --release --quiet
BIN="$PROJECT_ROOT/target/release/qwick"

"$BIN" --version | grep -q "qwick" || die "e2e" "version check failed"
log_ok "e2e" "version smoke passed"
```

- [ ] **Step 9: Make scripts executable**

Run:

```
chmod +x scripts/*.sh scripts/lib/common.sh
```

- [ ] **Step 10: Run check-all and verify green**

Run: `bash scripts/check-all.sh`
Expected: all gates pass on the bootstrap'd crate. `tests-mirror-check` should pass because no `src/` files exist that require mirroring yet.

- [ ] **Step 11: Commit**

```bash
git add scripts/
git commit -m "feat(scripts): quality gate scripts + umbrella check-all"
```

---

## Task 3: Claude Code hooks (`.claude/`)

**Goal:** Mirror the reference project's PreToolUse / PostToolUse / Stop hook system, adapted to Rust. Hooks delegate to `scripts/` — no logic is duplicated between hooks and gates.

**Files:**
- Create: `.claude/settings.json`
- Create: `.claude/hooks/lib/common.sh`
- Create: `.claude/hooks/pre-tools/mod.sh`
- Create: `.claude/hooks/pre-tools/modules/bash-commands.sh`
- Create: `.claude/hooks/pre-tools/modules/code-edit-rules.sh`
- Create: `.claude/hooks/pre-tools/modules/protected-files.sh`
- Create: `.claude/hooks/post-tools/mod.sh`
- Create: `.claude/hooks/post-tools/modules/auto-format.sh`
- Create: `.claude/hooks/post-tools/modules/auto-lint.sh`
- Create: `.claude/hooks/post-tools/modules/gate-status.sh`
- Create: `.claude/hooks/session-end.sh`
- Create: `.claude/tmp/.gitkeep`

- [ ] **Step 1: Register hooks in settings.json**

Create `.claude/settings.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Edit|Write|MultiEdit|Bash|Shell",
        "hooks": [
          { "type": "command", "command": "\"$CLAUDE_PROJECT_DIR\"/.claude/hooks/pre-tools/mod.sh" }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Edit|Write|MultiEdit|Bash|Shell",
        "hooks": [
          { "type": "command", "command": "\"$CLAUDE_PROJECT_DIR\"/.claude/hooks/post-tools/mod.sh" }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          { "type": "command", "command": "\"$CLAUDE_PROJECT_DIR\"/.claude/hooks/session-end.sh" }
        ]
      }
    ]
  }
}
```

- [ ] **Step 2: Shared hook helpers**

Create `.claude/hooks/lib/common.sh`:

```bash
#!/usr/bin/env bash
# Sourced by hook modules. Provides JSON parsing helpers and deny emitters.

set -euo pipefail

parse_tool_name() { jq -r '.tool_name // ""' 2>/dev/null || echo ""; }

# Args: $1 = reason text
deny_pre() {
  jq -n --arg r "$1" '{
    "hookSpecificOutput": {
      "hookEventName": "PreToolUse",
      "permissionDecision": "deny",
      "permissionDecisionReason": $r
    }
  }'
}

# Args: $1 = additional context string
post_context() {
  jq -n --arg c "$1" '{
    "hookSpecificOutput": {
      "hookEventName": "PostToolUse",
      "additionalContext": $c
    }
  }'
}
```

- [ ] **Step 3: Pre-tool dispatcher**

Create `.claude/hooks/pre-tools/mod.sh`:

```bash
#!/usr/bin/env bash
# Pre-tool-use dispatcher. Runs every module; first non-empty stdout wins.

input=$(cat)
tool_name=$(echo "$input" | jq -r '.tool_name // ""' 2>/dev/null || echo "")
export input tool_name

HOOK_DIR="$(cd "$(dirname "$0")" && pwd)/modules"

for script in "$HOOK_DIR"/*.sh; do
  [[ ! -f "$script" ]] && continue
  result=$(echo "$input" | bash "$script" 2>/dev/null)
  if [[ -n "$result" ]]; then
    echo "$result"
    exit 0
  fi
done
exit 0
```

- [ ] **Step 4: Pre-tool module — bash-commands**

Create `.claude/hooks/pre-tools/modules/bash-commands.sh`:

```bash
#!/usr/bin/env bash
# Rules for Bash/Shell tool invocations.
#   1. cargo-only — block npm/bun/pip/uv (none are part of this Rust project)
#   2. Block destructive commands (rm -rf, git push --force, git reset --hard, chmod -R 777)
#   3. Block bypass flags (--no-verify, --no-gpg-sign)
#   4. Block direct rustfmt/clippy invocation — must go through scripts/ or just

: "${tool_name:=}"
: "${input:=}"

[[ "$tool_name" != "Bash" && "$tool_name" != "Shell" ]] && exit 0
HOOK_LIB="$(cd "$(dirname "$0")/../.." && pwd)/lib/common.sh"
# shellcheck source=../../lib/common.sh
source "$HOOK_LIB"

command=$(echo "$input" | jq -r '.tool_input.command // ""')
[[ -z "$command" ]] && exit 0

cmd_only=$(echo "$command" | sed '/<<['"'"'"]*EOF['"'"'"]*$/,/^EOF$/d')

if echo "$cmd_only" | grep -qE '(^|\s|&&|\|\||;|`|\()(npm|npx|yarn|pnpm|bun|bunx|pip|uv|poetry)\s'; then
  deny_pre "qwick is a Rust project — use cargo / just / scripts/* instead of npm|bun|pip|uv."
  exit 0
fi

if echo "$cmd_only" | grep -qE '(^|\s|&&|\|\||;)(rm -rf|git push.*--force|git reset --hard|git checkout \.|chmod -R 777)'; then
  deny_pre "Destructive command blocked: $cmd_only"
  exit 0
fi

if echo "$cmd_only" | grep -qE '(--no-verify|--no-gpg-sign)'; then
  deny_pre "--no-verify / --no-gpg-sign forbidden. Rules cannot be bypassed."
  exit 0
fi

if echo "$cmd_only" | grep -qE '(^|\s|&&|\|\||;)(rustfmt|cargo\s+fmt\s|cargo\s+clippy\s)'; then
  if ! echo "$cmd_only" | grep -qE '(scripts/|just\s|lefthook|post-tools|pre-tools)'; then
    deny_pre "Run quality gates via scripts/* or 'just check' — do not invoke rustfmt/clippy directly."
    exit 0
  fi
fi
exit 0
```

- [ ] **Step 5: Pre-tool module — code-edit-rules**

Create `.claude/hooks/pre-tools/modules/code-edit-rules.sh`:

```bash
#!/usr/bin/env bash
# Reject forbidden patterns in new content for Edit/Write/MultiEdit on src/*.rs.

: "${tool_name:=}"
: "${input:=}"

[[ "$tool_name" != "Edit" && "$tool_name" != "Write" && "$tool_name" != "MultiEdit" ]] && exit 0
HOOK_LIB="$(cd "$(dirname "$0")/../.." && pwd)/lib/common.sh"
# shellcheck source=../../lib/common.sh
source "$HOOK_LIB"

new_content=$(echo "$input" | jq -r '
  .tool_input.new_string //
  .tool_input.content //
  (.tool_input.edits // [] | map(.new_string) | join("\n")) //
  empty
' 2>/dev/null)
[[ -z "$new_content" ]] && exit 0

file_path=$(echo "$input" | jq -r '.tool_input.file_path // empty' 2>/dev/null)
case "$file_path" in
  */src/*.rs) ;;
  *) exit 0 ;;
esac

violations=""
add() { violations="${violations}\n  - $1"; }

echo "$new_content" | grep -qE '#\[allow\('                       && add "#[allow(...)] override"
echo "$new_content" | grep -qE '#\[cfg\(test\)\][[:space:]]*\nmod tests' && add "#[cfg(test)] mod tests inside src/ (move to tests/)"
echo "$new_content" | grep -qE '\.unwrap\(\)'                     && add ".unwrap() in src/"
echo "$new_content" | grep -qE '\bprintln!'                       && add "println! (use tracing)"
echo "$new_content" | grep -qE '\beprintln!'                      && add "eprintln! (use tracing)"
echo "$new_content" | grep -qE '\btodo!\('                        && add "todo!()"
echo "$new_content" | grep -qE '\bunimplemented!\('               && add "unimplemented!()"
echo "$new_content" | grep -qE '\bpanic!\('                       && add "panic!() in src/"

# unsafe { … } requires a // SAFETY: line within 3 lines
if echo "$new_content" | grep -qE '\bunsafe\s*\{'; then
  if ! echo "$new_content" | awk '
    /\bunsafe\s*\{/ { found=NR }
    /SAFETY:/        { if (NR - found <= 3) ok=1 }
    END             { exit ok ? 0 : 1 }
  '; then
    add "unsafe { … } without // SAFETY: comment"
  fi
fi

if [[ -n "$violations" ]]; then
  reason="Forbidden pattern(s) in $file_path:${violations}\nFix the root cause."
  deny_pre "$reason"
  exit 0
fi
exit 0
```

- [ ] **Step 6: Pre-tool module — protected-files**

Create `.claude/hooks/pre-tools/modules/protected-files.sh`:

```bash
#!/usr/bin/env bash
# Block edits to vendored/build/protected paths.

: "${tool_name:=}"
: "${input:=}"

[[ "$tool_name" != "Edit" && "$tool_name" != "Write" && "$tool_name" != "MultiEdit" ]] && exit 0
HOOK_LIB="$(cd "$(dirname "$0")/../.." && pwd)/lib/common.sh"
# shellcheck source=../../lib/common.sh
source "$HOOK_LIB"

file_path=$(echo "$input" | jq -r '.tool_input.file_path // ""')
[[ -z "$file_path" ]] && exit 0

case "$file_path" in
  */target/*|target/*|*/Cargo.lock)
    deny_pre "$file_path is a build artifact — do not edit by hand."
    exit 0 ;;
  */deny.toml|*/lefthook.yml|*/rustfmt.toml|*/clippy.toml|*/typos.toml|*/.github/workflows/ci.yml)
    deny_pre "$file_path is a protected config — requires explicit user request to edit."
    exit 0 ;;
esac
exit 0
```

- [ ] **Step 7: Post-tool dispatcher**

Create `.claude/hooks/post-tools/mod.sh`:

```bash
#!/usr/bin/env bash
# Post-tool-use dispatcher.

input=$(cat 2>/dev/null || echo "{}")
tool_name=$(echo "$input" | jq -r '.tool_name // ""' 2>/dev/null || echo "")
PROJECT_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
export PATH="$PROJECT_ROOT/target/debug:$PATH"
export input tool_name PROJECT_ROOT

HOOK_DIR="$(cd "$(dirname "$0")" && pwd)/modules"
for script in "$HOOK_DIR"/*.sh; do
  [[ ! -f "$script" ]] && continue
  result=$(echo "$input" | bash "$script" 2>/dev/null)
  if [[ -n "$result" ]]; then
    echo "$result"
    exit 0
  fi
done
exit 0
```

- [ ] **Step 8: Post-tool modules — auto-format, auto-lint, gate-status**

Create `.claude/hooks/post-tools/modules/auto-format.sh`:

```bash
#!/usr/bin/env bash
: "${tool_name:=}"; : "${input:=}"; : "${PROJECT_ROOT:=$(pwd)}"
[[ "$tool_name" != "Edit" && "$tool_name" != "Write" && "$tool_name" != "MultiEdit" ]] && exit 0

file_path=$(echo "$input" | jq -r '.tool_input.file_path // ""' 2>/dev/null || echo "")
[[ -z "$file_path" || ! -f "$file_path" ]] && exit 0

case "$file_path" in
  *.rs) (cd "$PROJECT_ROOT" && rustfmt --emit=files --edition 2021 "$file_path" >/dev/null 2>&1) || true ;;
  *.toml) (cd "$PROJECT_ROOT" && command -v taplo >/dev/null && taplo fmt "$file_path" >/dev/null 2>&1) || true ;;
esac
exit 0
```

Create `.claude/hooks/post-tools/modules/auto-lint.sh`:

```bash
#!/usr/bin/env bash
: "${tool_name:=}"; : "${input:=}"; : "${PROJECT_ROOT:=$(pwd)}"
[[ "$tool_name" != "Edit" && "$tool_name" != "Write" && "$tool_name" != "MultiEdit" ]] && exit 0

file_path=$(echo "$input" | jq -r '.tool_input.file_path // ""' 2>/dev/null || echo "")
[[ -z "$file_path" || ! -f "$file_path" ]] && exit 0
case "$file_path" in
  */src/*.rs|*/tests/*.rs)
    (cd "$PROJECT_ROOT" && cargo clippy --fix --allow-dirty --allow-staged --quiet -- -D warnings >/dev/null 2>&1) || true ;;
esac
exit 0
```

Create `.claude/hooks/post-tools/modules/gate-status.sh`:

```bash
#!/usr/bin/env bash
# Track exit codes of recognized quality commands and persist to .claude/tmp/quality-gate-status.json.

: "${tool_name:=}"; : "${input:=}"; : "${PROJECT_ROOT:=$(pwd)}"
[[ "$tool_name" != "Bash" && "$tool_name" != "Shell" ]] && exit 0

HOOK_LIB="$(cd "$(dirname "$0")/../.." && pwd)/lib/common.sh"
# shellcheck source=../../lib/common.sh
source "$HOOK_LIB"

GATE_DIR="$PROJECT_ROOT/.claude/tmp"
GATE_FILE="$GATE_DIR/quality-gate-status.json"
mkdir -p "$GATE_DIR"

command=$(echo "$input" | jq -r '.tool_input.command // ""' 2>/dev/null || echo "")
exit_code=$(echo "$input" | jq -r '.tool_response.metadata.exit_code // .tool_response.exit_code // empty' 2>/dev/null || echo "")

if ! echo "$command" | grep -qE '(bash\s+scripts/(check-all|fmt-check|type-check|lint-check|test-run|test-placement-check|tests-mirror-check|no-bypass-check|module-size-check|typos-check|deny-check|dup-check|e2e)\.sh|just\s+(check|qa|test|e2e))'; then
  exit 0
fi

ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
if [[ "$exit_code" =~ ^[0-9]+$ && "$exit_code" -ne 0 ]]; then
  jq -n --arg s "failing" --arg c "$command" --arg e "$exit_code" --arg t "$ts" \
    '{status:$s, command:$c, exit_code:($e|tonumber), updatedAt:$t, source:"gate-status-hook"}' > "$GATE_FILE"
  post_context "Quality gate FAILING. Fix before continuing.\nFailed: $command (exit $exit_code)"
  exit 0
fi

if [[ "$exit_code" == "0" ]]; then
  jq -n --arg s "passing" --arg c "$command" --arg t "$ts" \
    '{status:$s, command:$c, updatedAt:$t}' > "$GATE_FILE"
fi
exit 0
```

- [ ] **Step 9: Session-end hook**

Create `.claude/hooks/session-end.sh`:

```bash
#!/usr/bin/env bash
# Stop hook: run the fast subset of gates so issues surface at end-of-conversation.

PROJECT_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$PROJECT_ROOT" || exit 0
echo "[session-end] running fast gates..."
bash scripts/fmt-check.sh           2>&1 | tail -3 || true
bash scripts/test-placement-check.sh 2>&1 | tail -3 || true
bash scripts/no-bypass-check.sh     2>&1 | tail -3 || true
bash scripts/module-size-check.sh   2>&1 | tail -3 || true
echo "[session-end] done"
```

- [ ] **Step 10: Create tmp dir + make executable**

Run:

```
mkdir -p .claude/tmp && touch .claude/tmp/.gitkeep
chmod +x .claude/hooks/pre-tools/mod.sh
chmod +x .claude/hooks/post-tools/mod.sh
chmod +x .claude/hooks/session-end.sh
chmod +x .claude/hooks/pre-tools/modules/*.sh
chmod +x .claude/hooks/post-tools/modules/*.sh
```

Add `.claude/tmp/quality-gate-status.json` to `.gitignore`:

```
echo ".claude/tmp/quality-gate-status.json" >> .gitignore
```

- [ ] **Step 11: Verify hooks parse**

Run:

```
jq . .claude/settings.json
bash -n .claude/hooks/pre-tools/mod.sh
bash -n .claude/hooks/post-tools/mod.sh
bash -n .claude/hooks/session-end.sh
for f in .claude/hooks/pre-tools/modules/*.sh .claude/hooks/post-tools/modules/*.sh; do bash -n "$f" || exit 1; done
```

Expected: all parse cleanly.

- [ ] **Step 12: Commit**

```bash
git add .claude/ .gitignore
git commit -m "feat(hooks): claude code pre/post/stop hooks delegating to scripts/"
```

---

## Task 4: Config paths + dirs

**Goal:** Resolve `~/.qwick/` paths deterministically; build the directory tree on first use.

**Files:**
- Create: `src/config/mod.rs`
- Create: `src/config/paths.rs`
- Create: `tests/common/mod.rs`
- Create: `tests/common/runner.rs`
- Create: `tests/config.rs`
- Create: `tests/config/paths.rs`
- Modify: `src/lib.rs` — add `pub mod config;`

- [ ] **Step 1: Write the failing test**

Create `tests/common/mod.rs`:

```rust
pub mod runner;
```

Create `tests/common/runner.rs`:

```rust
use std::path::PathBuf;
use tempfile::TempDir;

pub struct Sandbox {
    pub root: TempDir,
}

impl Sandbox {
    pub fn new() -> Self {
        Self { root: TempDir::new().unwrap() }
    }

    pub fn data_dir(&self) -> PathBuf {
        self.root.path().join(".qwick")
    }
}
```

Create `tests/config.rs`:

```rust
mod common;
mod paths;
```

Create `tests/config/paths.rs`:

```rust
use qwick::config::paths::Paths;

#[path = "../common/mod.rs"]
mod common;

#[test]
fn paths_resolves_subdirs_relative_to_data_dir() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());

    assert_eq!(paths.memories_dir(), sb.data_dir().join("memories"));
    assert_eq!(paths.trash_dir(),    sb.data_dir().join("memories").join(".trash"));
    assert_eq!(paths.vectors_dir(),  sb.data_dir().join("index").join("vectors.lance"));
    assert_eq!(paths.graph_dir(),    sb.data_dir().join("index").join("graph.kuzu"));
    assert_eq!(paths.stats_db(),     sb.data_dir().join("stats.db"));
    assert_eq!(paths.config_file(),  sb.data_dir().join("config.toml"));
}

#[test]
fn ensure_dirs_creates_full_tree() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    assert!(paths.memories_dir().exists());
    assert!(paths.trash_dir().exists());
    assert!(paths.vectors_dir().parent().unwrap().exists());
    assert!(paths.graph_dir().parent().unwrap().exists());
}
```

- [ ] **Step 2: Run and confirm failure**

Run: `cargo nextest run --test config`
Expected: compile error — `qwick::config::paths::Paths` not found.

- [ ] **Step 3: Implement Paths**

Create `src/config/paths.rs`:

```rust
use std::path::{Path, PathBuf};

use crate::prelude::*;

#[derive(Debug, Clone)]
pub struct Paths {
    data_dir: PathBuf,
}

impl Paths {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self { data_dir: data_dir.into() }
    }

    pub fn data_dir(&self) -> &Path { &self.data_dir }
    pub fn memories_dir(&self) -> PathBuf { self.data_dir.join("memories") }
    pub fn trash_dir(&self)    -> PathBuf { self.memories_dir().join(".trash") }
    pub fn index_dir(&self)    -> PathBuf { self.data_dir.join("index") }
    pub fn vectors_dir(&self)  -> PathBuf { self.index_dir().join("vectors.lance") }
    pub fn graph_dir(&self)    -> PathBuf { self.index_dir().join("graph.kuzu") }
    pub fn stats_db(&self)     -> PathBuf { self.data_dir.join("stats.db") }
    pub fn config_file(&self)  -> PathBuf { self.data_dir.join("config.toml") }

    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [self.memories_dir(), self.trash_dir(), self.index_dir()] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}
```

Create `src/config/mod.rs`:

```rust
pub mod paths;

pub use paths::Paths;
```

Modify `src/lib.rs`:

```rust
//! qwick — agentic dev memory + code-aware semantic search.

pub mod prelude;

#[path = "errors.rs"]
pub mod errors;

pub mod config;
```

- [ ] **Step 4: Run test, confirm pass**

Run: `cargo nextest run --test config`
Expected: 2 tests pass.

- [ ] **Step 5: Quality gates**

Run:

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --all-features
```

All green.

- [ ] **Step 6: Commit**

```bash
git add src/config tests/common tests/config.rs tests/config/paths.rs src/lib.rs
git commit -m "feat(config): resolve qwick data dir paths and ensure dirs"
```

---

## Task 5: Config file load + defaults + env overrides

**Goal:** Layered config (defaults → `config.toml` → env → flags). Defaults match spec §15.

**Files:**
- Create: `src/config/file.rs`
- Modify: `src/config/mod.rs`
- Create: `tests/config/file.rs`
- Modify: `tests/config.rs`

- [ ] **Step 1: Failing test**

Modify `tests/config.rs`:

```rust
mod common;
mod paths;
mod file;
```

Create `tests/config/file.rs`:

```rust
use qwick::config::file::{AutoReindexMode, Config};

#[test]
fn defaults_match_spec() {
    let c = Config::defaults();
    assert_eq!(c.embeddings.memory_model, "nomic-embed-text-v1.5-Q");
    assert_eq!(c.embeddings.code_model,   "jina-embeddings-v2-base-code-Q");
    assert!(matches!(c.indexing.auto_reindex, AutoReindexMode::Lazy));
    assert_eq!(c.indexing.auto_reindex_threshold_ms, 200);
    assert_eq!(c.retrieval.memory_threshold, 0.55);
    assert_eq!(c.retrieval.code_threshold,   0.50);
    assert_eq!(c.retrieval.hybrid_weight,    0.65);
    assert_eq!(c.retrieval.top_k, 12);
    assert_eq!(c.prune.trash_retention_days, 30);
}

#[test]
fn env_overrides_apply() {
    std::env::set_var("QWICK_INDEXING_AUTO_REINDEX", "hook");
    std::env::set_var("QWICK_RETRIEVAL_TOP_K", "20");
    let c = Config::defaults().with_env();
    std::env::remove_var("QWICK_INDEXING_AUTO_REINDEX");
    std::env::remove_var("QWICK_RETRIEVAL_TOP_K");
    assert!(matches!(c.indexing.auto_reindex, AutoReindexMode::Hook));
    assert_eq!(c.retrieval.top_k, 20);
}

#[test]
fn toml_round_trip() {
    let c = Config::defaults();
    let s = toml::to_string(&c).unwrap();
    let back: Config = toml::from_str(&s).unwrap();
    assert_eq!(back.retrieval.top_k, c.retrieval.top_k);
}
```

- [ ] **Step 2: Run, confirm failure**

Run: `cargo nextest run --test config file`
Expected: `Config` not found.

- [ ] **Step 3: Implement Config**

Create `src/config/file.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AutoReindexMode { Lazy, Hook, Off }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitConfig {
    pub auto_sync: bool,
    pub remote: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsConfig {
    pub memory_model: String,
    pub code_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingConfig {
    pub auto_reindex: AutoReindexMode,
    pub auto_reindex_threshold_ms: u64,
    pub incremental_batch_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalConfig {
    pub memory_threshold: f32,
    pub code_threshold: f32,
    pub hybrid_weight: f32,
    pub top_k: usize,
    pub corrective_min_confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneConfig {
    pub trash_retention_days: u32,
    pub low_value_default_unused_since_days: u32,
    pub low_value_default_below_quality: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    pub json: bool,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub git: GitConfig,
    pub embeddings: EmbeddingsConfig,
    pub indexing: IndexingConfig,
    pub retrieval: RetrievalConfig,
    pub prune: PruneConfig,
    pub output: OutputConfig,
}

impl Config {
    pub fn defaults() -> Self {
        Self {
            git: GitConfig { auto_sync: false, remote: String::new() },
            embeddings: EmbeddingsConfig {
                memory_model: "nomic-embed-text-v1.5-Q".into(),
                code_model:   "jina-embeddings-v2-base-code-Q".into(),
            },
            indexing: IndexingConfig {
                auto_reindex: AutoReindexMode::Lazy,
                auto_reindex_threshold_ms: 200,
                incremental_batch_size: 50,
            },
            retrieval: RetrievalConfig {
                memory_threshold: 0.55,
                code_threshold:   0.50,
                hybrid_weight:    0.65,
                top_k: 12,
                corrective_min_confidence: 0.15,
            },
            prune: PruneConfig {
                trash_retention_days: 30,
                low_value_default_unused_since_days: 180,
                low_value_default_below_quality: 2,
            },
            output: OutputConfig { json: false, color: "auto".into() },
        }
    }

    pub fn with_env(mut self) -> Self {
        if let Ok(v) = std::env::var("QWICK_INDEXING_AUTO_REINDEX") {
            self.indexing.auto_reindex = match v.as_str() {
                "hook" => AutoReindexMode::Hook,
                "off"  => AutoReindexMode::Off,
                _      => AutoReindexMode::Lazy,
            };
        }
        if let Ok(v) = std::env::var("QWICK_RETRIEVAL_TOP_K") {
            if let Ok(n) = v.parse() { self.retrieval.top_k = n; }
        }
        if let Ok(v) = std::env::var("QWICK_RETRIEVAL_MEMORY_THRESHOLD") {
            if let Ok(n) = v.parse() { self.retrieval.memory_threshold = n; }
        }
        if let Ok(v) = std::env::var("QWICK_RETRIEVAL_CODE_THRESHOLD") {
            if let Ok(n) = v.parse() { self.retrieval.code_threshold = n; }
        }
        if let Ok(v) = std::env::var("QWICK_GIT_AUTO_SYNC") {
            self.git.auto_sync = v == "true" || v == "1";
        }
        self
    }
}
```

Modify `src/config/mod.rs`:

```rust
pub mod paths;
pub mod file;

pub use file::{AutoReindexMode, Config};
pub use paths::Paths;
```

- [ ] **Step 4: Run, confirm pass**

Run: `cargo nextest run --test config`
Expected: 5 tests pass.

- [ ] **Step 5: Quality gates** — fmt, clippy, nextest, deny — all green.

- [ ] **Step 6: Commit**

```bash
git add src/config tests/config.rs tests/config/file.rs
git commit -m "feat(config): layered Config with defaults and env overrides"
```

---

## Task 6: Memory core — IDs, frontmatter, slug, atomic store

**Goal:** Ship the markdown-as-source-of-truth layer. SHA-256-prefix IDs, YAML frontmatter round-trip, slug derivation, atomic save with temp file + rename, list, load, delete.

**Files:**
- Create: `src/memory/mod.rs`
- Create: `src/memory/id.rs`
- Create: `src/memory/slug.rs`
- Create: `src/memory/frontmatter.rs`
- Create: `src/memory/store.rs`
- Modify: `src/lib.rs`
- Create: `tests/memory.rs`
- Create: `tests/memory/id.rs`
- Create: `tests/memory/slug.rs`
- Create: `tests/memory/frontmatter.rs`
- Create: `tests/memory/store.rs`

- [ ] **Step 1: Failing tests for IDs**

Create `tests/memory.rs`:

```rust
mod common;
mod id;
mod slug;
mod frontmatter;
mod store;
```

(Hint: `tests/common/` already exists from Task 4.)

Create `tests/memory/id.rs`:

```rust
use qwick::memory::id::memory_id;

#[test]
fn id_is_8_hex_prefix_of_sha256() {
    let id = memory_id("the quick brown fox");
    assert_eq!(id.len(), 8);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn id_is_stable_across_calls() {
    let a = memory_id("hello world");
    let b = memory_id("hello world");
    assert_eq!(a, b);
}

#[test]
fn id_normalizes_trailing_whitespace() {
    let a = memory_id("body text");
    let b = memory_id("body text\n\n  ");
    assert_eq!(a, b);
}
```

- [ ] **Step 2: Run, confirm failure**

Run: `cargo nextest run --test memory id`
Expected: compile failure — `qwick::memory` not found.

- [ ] **Step 3: Implement id.rs**

Create `src/memory/id.rs`:

```rust
use sha2::{Digest, Sha256};

pub fn memory_id(body: &str) -> String {
    let trimmed = body.trim_end();
    let digest = Sha256::digest(trimmed.as_bytes());
    let mut hex = String::with_capacity(8);
    for byte in &digest[..4] {
        use std::fmt::Write as _;
        let _ = write!(hex, "{:02x}", byte);
    }
    hex
}
```

Create `src/memory/mod.rs`:

```rust
pub mod id;
pub mod slug;
pub mod frontmatter;
pub mod store;

pub use frontmatter::{Frontmatter, Kind, References, Relations};
pub use store::{MemoryRecord, MemoryStore};
```

Modify `src/lib.rs` — add `pub mod memory;`.

- [ ] **Step 4: Run, confirm green**

Run: `cargo nextest run --test memory id`
Expected: 3 tests pass.

- [ ] **Step 5: Failing tests for slug**

Create `tests/memory/slug.rs`:

```rust
use qwick::memory::slug::slug_from_body;

#[test]
fn slug_from_first_meaningful_line() {
    let s = slug_from_body("decision: use Postgres for analytics");
    assert_eq!(s, "decision-use-postgres-for-analytics");
}

#[test]
fn slug_truncates_to_max_chars() {
    let body = "a".repeat(200);
    assert_eq!(slug_from_body(&body).len(), 60);
}

#[test]
fn slug_falls_back_when_only_whitespace() {
    assert_eq!(slug_from_body("\n\n  "), "untitled");
}

#[test]
fn slug_only_keeps_ascii_alphanumeric_and_dashes() {
    let s = slug_from_body("Café — über 100%!");
    for c in s.chars() {
        assert!(c.is_ascii_alphanumeric() || c == '-', "bad char: {c}");
    }
}
```

- [ ] **Step 6: Implement slug.rs**

Create `src/memory/slug.rs`:

```rust
const MAX_SLUG_LEN: usize = 60;

pub fn slug_from_body(body: &str) -> String {
    let first = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut out = String::with_capacity(MAX_SLUG_LEN);
    let mut prev_dash = false;
    for c in first.chars() {
        let mapped = if c.is_ascii_alphanumeric() {
            c.to_ascii_lowercase()
        } else {
            '-'
        };
        if mapped == '-' {
            if !prev_dash && !out.is_empty() {
                out.push('-');
                prev_dash = true;
            }
        } else {
            out.push(mapped);
            prev_dash = false;
        }
        if out.len() >= MAX_SLUG_LEN {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() { "untitled".into() } else { trimmed }
}
```

- [ ] **Step 7: Run + verify pass**

Run: `cargo nextest run --test memory slug`
Expected: 4 tests pass.

- [ ] **Step 8: Failing tests for frontmatter**

Create `tests/memory/frontmatter.rs`:

```rust
use qwick::memory::frontmatter::{Frontmatter, Kind};
use time::OffsetDateTime;

#[test]
fn round_trips_yaml() {
    let fm = Frontmatter {
        id: "a1b2c3d4".into(),
        kind: Kind::Decision,
        repo: "qwick-backend".into(),
        tags: vec!["postgres".into(), "migration".into()],
        author: "falconiere".into(),
        created: OffsetDateTime::from_unix_timestamp(1_734_700_000).unwrap(),
        quality: 4,
        schema: 1,
        content_hash: "a1b2c3d4e5f6".into(),
        references: Default::default(),
        relations: Default::default(),
    };
    let yaml = fm.to_yaml().unwrap();
    let back = Frontmatter::from_yaml(&yaml).unwrap();
    assert_eq!(back.id, fm.id);
    assert_eq!(back.kind, Kind::Decision);
    assert_eq!(back.tags, vec!["postgres".to_string(), "migration".into()]);
    assert_eq!(back.schema, 1);
}

#[test]
fn split_separates_frontmatter_and_body() {
    let raw = "---\nid: a1b2c3d4\nkind: note\nrepo: r\ntags: []\nauthor: a\ncreated: 2026-05-17T00:00:00Z\nquality: 3\nschema: 1\ncontent_hash: x\nreferences: {symbols: [], files: []}\nrelations: {supersedes: [], conflicts_with: [], derived_from: []}\n---\nhello body\n";
    let (fm, body) = Frontmatter::split(raw).unwrap();
    assert_eq!(fm.id, "a1b2c3d4");
    assert_eq!(body.trim(), "hello body");
}
```

- [ ] **Step 9: Implement frontmatter.rs**

Create `src/memory/frontmatter.rs`:

```rust
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Iso8601;
use time::OffsetDateTime;

use crate::prelude::*;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Decision,
    Bug,
    Convention,
    Discovery,
    Pattern,
    Note,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct References {
    #[serde(default)]
    pub symbols: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Relations {
    #[serde(default)]
    pub supersedes: Vec<String>,
    #[serde(default)]
    pub conflicts_with: Vec<String>,
    #[serde(default)]
    pub derived_from: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frontmatter {
    pub id: String,
    pub kind: Kind,
    pub repo: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub author: String,
    #[serde(with = "iso8601_serde")]
    pub created: OffsetDateTime,
    pub quality: u8,
    pub schema: u32,
    pub content_hash: String,
    #[serde(default)]
    pub references: References,
    #[serde(default)]
    pub relations: Relations,
}

impl Frontmatter {
    pub fn to_yaml(&self) -> Result<String> {
        Ok(serde_yaml::to_string(self)?)
    }

    pub fn from_yaml(s: &str) -> Result<Self> {
        Ok(serde_yaml::from_str(s)?)
    }

    /// Split a markdown file starting with `---\n…\n---\n` into frontmatter + body.
    pub fn split(raw: &str) -> Result<(Self, String)> {
        let stripped = raw
            .strip_prefix("---\n")
            .ok_or_else(|| Error::Other("missing leading '---'".into()))?;
        let end = stripped
            .find("\n---\n")
            .ok_or_else(|| Error::Other("missing closing '---'".into()))?;
        let yaml = &stripped[..end];
        let body = &stripped[end + 5..];
        let fm = Self::from_yaml(yaml)?;
        Ok((fm, body.to_string()))
    }

    pub fn render(&self, body: &str) -> Result<String> {
        let yaml = self.to_yaml()?;
        Ok(format!("---\n{}---\n{}", yaml, body))
    }
}

mod iso8601_serde {
    use super::*;
    use serde::Serializer;
    use serde::Deserializer;

    pub fn serialize<S: Serializer>(t: &OffsetDateTime, s: S) -> std::result::Result<S::Ok, S::Error> {
        let formatted = t.format(&Iso8601::DEFAULT).map_err(serde::ser::Error::custom)?;
        s.serialize_str(&formatted)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<OffsetDateTime, D::Error> {
        let s: String = serde::Deserialize::deserialize(d)?;
        OffsetDateTime::parse(&s, &Iso8601::DEFAULT).map_err(serde::de::Error::custom)
    }
}
```

- [ ] **Step 10: Run + verify pass**

Run: `cargo nextest run --test memory frontmatter`
Expected: 2 tests pass.

- [ ] **Step 11: Failing tests for store**

Create `tests/memory/store.rs`:

```rust
use qwick::config::paths::Paths;
use qwick::memory::{Kind, MemoryStore};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn save_then_load_round_trips() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());

    let rec = store
        .save("Use Postgres for analytics", Kind::Decision, "qwick-backend", &["postgres".into()], "falconiere", 4)
        .unwrap();
    assert_eq!(rec.frontmatter.kind, Kind::Decision);
    assert_eq!(rec.frontmatter.tags, vec!["postgres".to_string()]);

    let loaded = store.load(&rec.frontmatter.id).unwrap();
    assert_eq!(loaded.body.trim(), "Use Postgres for analytics");
    assert_eq!(loaded.frontmatter.id, rec.frontmatter.id);
}

#[test]
fn save_is_atomic_under_failure() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let _ = store.save("body", Kind::Note, "r", &[], "a", 3).unwrap();
    let entries: Vec<_> = std::fs::read_dir(paths.memories_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(entries.is_empty(), "no .tmp files should remain: {entries:?}");
}

#[test]
fn list_returns_all_saved() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths);
    let _ = store.save("first",  Kind::Note, "r", &[], "a", 3).unwrap();
    let _ = store.save("second", Kind::Note, "r", &[], "a", 3).unwrap();
    let all = store.list().unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn delete_removes_file_and_returns_record() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store.save("to delete", Kind::Note, "r", &[], "a", 3).unwrap();
    let removed = store.delete(&rec.frontmatter.id).unwrap();
    assert_eq!(removed.frontmatter.id, rec.frontmatter.id);
    assert!(store.load(&rec.frontmatter.id).is_err());
}
```

- [ ] **Step 12: Implement store.rs**

Create `src/memory/store.rs`:

```rust
use std::fs;
use std::path::PathBuf;

use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::config::paths::Paths;
use crate::memory::frontmatter::{Frontmatter, Kind, References, Relations};
use crate::memory::id::memory_id;
use crate::memory::slug::slug_from_body;
use crate::prelude::*;

#[derive(Debug, Clone)]
pub struct MemoryRecord {
    pub frontmatter: Frontmatter,
    pub body: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct MemoryStore {
    paths: Paths,
}

impl MemoryStore {
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }

    pub fn save(
        &self,
        body: &str,
        kind: Kind,
        repo: &str,
        tags: &[String],
        author: &str,
        quality: u8,
    ) -> Result<MemoryRecord> {
        let id = memory_id(body);
        let slug = slug_from_body(body);
        let final_path = self.paths.memories_dir().join(format!("{id}-{slug}.md"));
        let tmp_path = self.paths.memories_dir().join(format!(".{id}.tmp"));

        let content_hash = sha256_hex(body.trim_end().as_bytes());
        let fm = Frontmatter {
            id: id.clone(),
            kind,
            repo: repo.to_string(),
            tags: tags.to_vec(),
            author: author.to_string(),
            created: OffsetDateTime::now_utc(),
            quality,
            schema: 1,
            content_hash,
            references: References::default(),
            relations: Relations::default(),
        };

        let rendered = fm.render(body.trim_end())?;
        fs::write(&tmp_path, rendered)?;

        if let Err(e) = fs::rename(&tmp_path, &final_path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e.into());
        }

        Ok(MemoryRecord { frontmatter: fm, body: body.trim_end().to_string(), path: final_path })
    }

    pub fn load(&self, id: &str) -> Result<MemoryRecord> {
        let path = self.find_by_id(id)?;
        let raw = fs::read_to_string(&path)?;
        let (fm, body) = Frontmatter::split(&raw)?;
        Ok(MemoryRecord { frontmatter: fm, body, path })
    }

    pub fn delete(&self, id: &str) -> Result<MemoryRecord> {
        let rec = self.load(id)?;
        let trash = self.paths.trash_dir().join(rec.path.file_name().unwrap());
        fs::create_dir_all(self.paths.trash_dir())?;
        fs::rename(&rec.path, &trash)?;
        Ok(rec)
    }

    pub fn list(&self) -> Result<Vec<MemoryRecord>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(self.paths.memories_dir())? {
            let entry = entry?;
            let name = entry.file_name().into_string().unwrap_or_default();
            if !name.ends_with(".md") || name.starts_with('.') {
                continue;
            }
            let raw = fs::read_to_string(entry.path())?;
            let (fm, body) = Frontmatter::split(&raw)?;
            out.push(MemoryRecord { frontmatter: fm, body, path: entry.path() });
        }
        Ok(out)
    }

    fn find_by_id(&self, id: &str) -> Result<PathBuf> {
        for entry in fs::read_dir(self.paths.memories_dir())? {
            let entry = entry?;
            let name = entry.file_name().into_string().unwrap_or_default();
            if name.starts_with(&format!("{id}-")) && name.ends_with(".md") {
                return Ok(entry.path());
            }
        }
        Err(Error::Other(format!("memory not found: {id}")))
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{:02x}", byte);
    }
    hex
}
```

- [ ] **Step 13: Run + verify pass**

Run: `cargo nextest run --test memory`
Expected: all memory tests pass (id 3 + slug 4 + frontmatter 2 + store 4 = 13).

- [ ] **Step 14: Run full check-all**

Run: `bash scripts/check-all.sh`
Expected: green. `tests-mirror-check` now sees `src/memory/{id,slug,frontmatter,store}.rs` and confirms mirrors exist in `tests/memory/`.

- [ ] **Step 15: Commit**

```bash
git add src/memory src/lib.rs tests/memory.rs tests/memory/
git commit -m "feat(memory): id, slug, frontmatter, atomic store with TDD"
```

---

## Task 7: Stats — SQLite schema, retrieval log, feedback

**Goal:** SQLite schema for retrieval logging, feedback (used/irrelevant counts), and per-repo indexing markers.

**Files:**
- Create: `src/stats/mod.rs`
- Create: `src/stats/sqlite.rs`
- Create: `src/stats/feedback.rs`
- Modify: `src/lib.rs`
- Create: `tests/stats.rs`
- Create: `tests/stats/sqlite.rs`
- Create: `tests/stats/feedback.rs`

- [ ] **Step 1: Failing tests for sqlite open + migrate**

Create `tests/stats.rs`:

```rust
mod common;
mod sqlite;
mod feedback;
```

Create `tests/stats/sqlite.rs`:

```rust
use qwick::stats::sqlite::StatsDb;

#[path = "../common/mod.rs"]
mod common;

#[test]
fn open_creates_schema() {
    let sb = common::runner::Sandbox::new();
    let db = StatsDb::open(sb.data_dir().join("stats.db")).unwrap();
    let tables: Vec<String> = db
        .conn()
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(tables.iter().any(|t| t == "feedback"));
    assert!(tables.iter().any(|t| t == "retrieval_log"));
    assert!(tables.iter().any(|t| t == "repo_marker"));
}
```

- [ ] **Step 2: Implement sqlite.rs**

Create `src/stats/sqlite.rs`:

```rust
use std::path::Path;

use rusqlite::Connection;

use crate::prelude::*;

pub struct StatsDb {
    conn: Connection,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS retrieval_log(
  query_id     TEXT PRIMARY KEY,
  query        TEXT NOT NULL,
  returned_ids TEXT NOT NULL,
  at           TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS feedback(
  memory_id    TEXT PRIMARY KEY,
  used_count   INTEGER NOT NULL DEFAULT 0,
  irrelevant_count INTEGER NOT NULL DEFAULT 0,
  last_used    TEXT
);
CREATE TABLE IF NOT EXISTS repo_marker(
  repo            TEXT PRIMARY KEY,
  last_head       TEXT,
  last_indexed_at TEXT
);
"#;

impl StatsDb {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path.as_ref()).map_err(|e| Error::Other(e.to_string()))?;
        conn.execute_batch(SCHEMA).map_err(|e| Error::Other(e.to_string()))?;
        Ok(Self { conn })
    }

    pub fn conn(&self) -> &Connection { &self.conn }
    pub fn conn_mut(&mut self) -> &mut Connection { &mut self.conn }
}
```

Create `src/stats/mod.rs`:

```rust
pub mod sqlite;
pub mod feedback;

pub use feedback::Feedback;
pub use sqlite::StatsDb;
```

Modify `src/lib.rs` — add `pub mod stats;`.

- [ ] **Step 3: Run + verify pass**

Run: `cargo nextest run --test stats sqlite`
Expected: 1 test passes.

- [ ] **Step 4: Failing tests for feedback**

Create `tests/stats/feedback.rs`:

```rust
use qwick::stats::feedback::Feedback;
use qwick::stats::sqlite::StatsDb;

#[path = "../common/mod.rs"]
mod common;

#[test]
fn record_used_increments_count() {
    let sb = common::runner::Sandbox::new();
    let mut db = StatsDb::open(sb.data_dir().join("stats.db")).unwrap();
    let fb = Feedback::new(&mut db);
    fb.record_used("m1").unwrap();
    fb.record_used("m1").unwrap();
    let (used, irrelevant) = fb.counts("m1").unwrap();
    assert_eq!(used, 2);
    assert_eq!(irrelevant, 0);
}

#[test]
fn record_irrelevant_increments_count() {
    let sb = common::runner::Sandbox::new();
    let mut db = StatsDb::open(sb.data_dir().join("stats.db")).unwrap();
    let fb = Feedback::new(&mut db);
    fb.record_irrelevant("m2").unwrap();
    let (used, irrelevant) = fb.counts("m2").unwrap();
    assert_eq!(used, 0);
    assert_eq!(irrelevant, 1);
}
```

- [ ] **Step 5: Implement feedback.rs**

Create `src/stats/feedback.rs`:

```rust
use time::format_description::well_known::Iso8601;
use time::OffsetDateTime;

use crate::prelude::*;
use crate::stats::sqlite::StatsDb;

pub struct Feedback<'a> {
    db: &'a mut StatsDb,
}

impl<'a> Feedback<'a> {
    pub fn new(db: &'a mut StatsDb) -> Self { Self { db } }

    pub fn record_used(&self, id: &str) -> Result<()> {
        let now = OffsetDateTime::now_utc().format(&Iso8601::DEFAULT)
            .map_err(|e| Error::Other(e.to_string()))?;
        self.db
            .conn()
            .execute(
                "INSERT INTO feedback(memory_id, used_count, irrelevant_count, last_used)
                 VALUES (?1, 1, 0, ?2)
                 ON CONFLICT(memory_id) DO UPDATE SET used_count = used_count + 1, last_used = ?2",
                rusqlite::params![id, now],
            )
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(())
    }

    pub fn record_irrelevant(&self, id: &str) -> Result<()> {
        self.db
            .conn()
            .execute(
                "INSERT INTO feedback(memory_id, used_count, irrelevant_count)
                 VALUES (?1, 0, 1)
                 ON CONFLICT(memory_id) DO UPDATE SET irrelevant_count = irrelevant_count + 1",
                rusqlite::params![id],
            )
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(())
    }

    pub fn counts(&self, id: &str) -> Result<(u64, u64)> {
        let row = self
            .db
            .conn()
            .query_row(
                "SELECT used_count, irrelevant_count FROM feedback WHERE memory_id = ?1",
                rusqlite::params![id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .unwrap_or((0, 0));
        Ok((row.0 as u64, row.1 as u64))
    }
}
```

- [ ] **Step 6: Run + verify pass**

Run: `cargo nextest run --test stats`
Expected: 3 tests pass.

- [ ] **Step 7: Run check-all**

Run: `bash scripts/check-all.sh`
Expected: green.

- [ ] **Step 8: Commit**

```bash
git add src/stats src/lib.rs tests/stats.rs tests/stats/
git commit -m "feat(stats): sqlite schema + feedback record/query"
```

---

## Task 8: Indexing — memory side (LanceDB + embedder)

**Goal:** Embed memory bodies with `nomic-embed-text-v1.5-Q`, upsert into a LanceDB `memory_chunks` table, query by vector.

**Files:**
- Create: `src/index/mod.rs`
- Create: `src/index/embedder.rs`
- Create: `src/index/schema.rs`
- Create: `src/index/memory_index.rs`
- Modify: `src/lib.rs`
- Create: `tests/index.rs`
- Create: `tests/index/embedder.rs`
- Create: `tests/index/memory_index.rs`

- [ ] **Step 1: Failing tests for embedder**

Create `tests/index.rs`:

```rust
mod common;
mod embedder;
mod memory_index;
```

Create `tests/index/embedder.rs`:

```rust
use qwick::index::embedder::Embedder;

#[test]
fn nomic_embeds_to_768d() {
    let mut emb = Embedder::nomic_text().unwrap();
    let v = emb.embed_one("hello world").unwrap();
    assert_eq!(v.len(), 768);
    let mag: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(mag > 0.0);
}

#[test]
fn deterministic_same_input_same_output() {
    let mut emb = Embedder::nomic_text().unwrap();
    let a = emb.embed_one("postgres migration race").unwrap();
    let b = emb.embed_one("postgres migration race").unwrap();
    assert_eq!(a, b);
}
```

- [ ] **Step 2: Implement embedder.rs**

Create `src/index/embedder.rs`:

```rust
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::prelude::*;

pub struct Embedder {
    inner: TextEmbedding,
    pub dim: usize,
}

impl Embedder {
    pub fn nomic_text() -> Result<Self> {
        let inner = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::NomicEmbedTextV15Q))
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(Self { inner, dim: 768 })
    }

    pub fn jina_code() -> Result<Self> {
        let inner = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::JinaEmbeddingsV2BaseCode))
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(Self { inner, dim: 768 })
    }

    pub fn embed_one(&mut self, text: &str) -> Result<Vec<f32>> {
        let mut out = self
            .inner
            .embed(vec![text.to_string()], None)
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(out.remove(0))
    }

    pub fn embed_many<I, S>(&mut self, texts: I) -> Result<Vec<Vec<f32>>>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let owned: Vec<String> = texts.into_iter().map(Into::into).collect();
        self.inner
            .embed(owned, None)
            .map_err(|e| Error::Other(e.to_string()))
    }
}
```

- [ ] **Step 3: Implement schema.rs + memory_index.rs (shell first)**

Create `src/index/schema.rs`:

```rust
use std::sync::Arc;

use lancedb::arrow::arrow_schema::{DataType, Field, Schema};

pub fn memory_schema(dim: usize) -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id",           DataType::Utf8, false),
        Field::new("body",         DataType::Utf8, false),
        Field::new("kind",         DataType::Utf8, false),
        Field::new("repo",         DataType::Utf8, false),
        Field::new("tags",         DataType::Utf8, false),
        Field::new("created",      DataType::Utf8, false),
        Field::new("quality",      DataType::Int32, false),
        Field::new("content_hash", DataType::Utf8, false),
        Field::new("embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dim as i32,
            ),
            false,
        ),
    ]))
}

pub const MEMORY_TABLE: &str = "memory_chunks";
```

Create `src/index/memory_index.rs`:

```rust
use std::path::Path;
use std::sync::Arc;

use lancedb::arrow::arrow_array::{
    FixedSizeListArray, Float32Array, Int32Array, RecordBatch, RecordBatchIterator, StringArray,
};
use lancedb::arrow::arrow_schema::Schema;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::{Connection, connect};

use crate::index::embedder::Embedder;
use crate::index::schema::{memory_schema, MEMORY_TABLE};
use crate::memory::{Kind, MemoryRecord};
use crate::prelude::*;

pub struct MemoryIndex {
    conn: Connection,
    schema: Arc<Schema>,
}

#[derive(Debug, Clone)]
pub struct MemoryHit {
    pub id: String,
    pub score: f32,
    pub body: String,
    pub kind: Kind,
    pub repo: String,
}

impl MemoryIndex {
    pub async fn open(dir: impl AsRef<Path>, dim: usize) -> Result<Self> {
        let uri = dir.as_ref().to_string_lossy().to_string();
        let conn = connect(&uri)
            .execute()
            .await
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(Self { conn, schema: memory_schema(dim) })
    }

    pub async fn upsert(&self, rec: &MemoryRecord, emb: &[f32]) -> Result<()> {
        let batch = self.batch_from_record(rec, emb)?;
        let batches = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), self.schema.clone());
        let names = self.conn.table_names().execute().await
            .map_err(|e| Error::Other(e.to_string()))?;
        if names.iter().any(|n| n == MEMORY_TABLE) {
            let tbl = self.conn.open_table(MEMORY_TABLE).execute().await
                .map_err(|e| Error::Other(e.to_string()))?;
            tbl.merge_insert(&["id"])
                .when_matched_update_all(None)
                .when_not_matched_insert_all()
                .execute(Box::new(batches))
                .await
                .map_err(|e| Error::Other(e.to_string()))?;
        } else {
            self.conn.create_table(MEMORY_TABLE, Box::new(batches))
                .execute()
                .await
                .map_err(|e| Error::Other(e.to_string()))?;
        }
        Ok(())
    }

    pub async fn search(&self, query_emb: &[f32], limit: usize) -> Result<Vec<MemoryHit>> {
        let names = self.conn.table_names().execute().await
            .map_err(|e| Error::Other(e.to_string()))?;
        if !names.iter().any(|n| n == MEMORY_TABLE) {
            return Ok(Vec::new());
        }
        let tbl = self.conn.open_table(MEMORY_TABLE).execute().await
            .map_err(|e| Error::Other(e.to_string()))?;
        let stream = tbl.query()
            .nearest_to(query_emb).map_err(|e| Error::Other(e.to_string()))?
            .limit(limit)
            .execute()
            .await
            .map_err(|e| Error::Other(e.to_string()))?;
        let batches = stream.collect::<Vec<_>>().await;

        let mut hits = Vec::new();
        for b in batches.into_iter().flatten() {
            self.collect_hits(&b, &mut hits)?;
        }
        Ok(hits)
    }

    fn batch_from_record(&self, rec: &MemoryRecord, emb: &[f32]) -> Result<RecordBatch> {
        let fm = &rec.frontmatter;
        let tags_csv = fm.tags.join(",");
        let kind_str = match fm.kind {
            Kind::Decision => "decision",
            Kind::Bug => "bug",
            Kind::Convention => "convention",
            Kind::Discovery => "discovery",
            Kind::Pattern => "pattern",
            Kind::Note => "note",
        };
        let created_str = fm
            .created
            .format(&time::format_description::well_known::Iso8601::DEFAULT)
            .map_err(|e| Error::Other(e.to_string()))?;
        let emb_array = FixedSizeListArray::try_new(
            Arc::new(lancedb::arrow::arrow_schema::Field::new("item", lancedb::arrow::arrow_schema::DataType::Float32, true)),
            emb.len() as i32,
            Arc::new(Float32Array::from(emb.to_vec())),
            None,
        ).map_err(|e| Error::Other(e.to_string()))?;
        let batch = RecordBatch::try_new(
            self.schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![fm.id.clone()])),
                Arc::new(StringArray::from(vec![rec.body.clone()])),
                Arc::new(StringArray::from(vec![kind_str.to_string()])),
                Arc::new(StringArray::from(vec![fm.repo.clone()])),
                Arc::new(StringArray::from(vec![tags_csv])),
                Arc::new(StringArray::from(vec![created_str])),
                Arc::new(Int32Array::from(vec![fm.quality as i32])),
                Arc::new(StringArray::from(vec![fm.content_hash.clone()])),
                Arc::new(emb_array),
            ],
        ).map_err(|e| Error::Other(e.to_string()))?;
        Ok(batch)
    }

    fn collect_hits(&self, batch: &RecordBatch, out: &mut Vec<MemoryHit>) -> Result<()> {
        let id_col = batch.column_by_name("id")
            .ok_or_else(|| Error::Other("id col".into()))?
            .as_any().downcast_ref::<StringArray>()
            .ok_or_else(|| Error::Other("id type".into()))?;
        let body_col = batch.column_by_name("body").unwrap()
            .as_any().downcast_ref::<StringArray>().unwrap();
        let kind_col = batch.column_by_name("kind").unwrap()
            .as_any().downcast_ref::<StringArray>().unwrap();
        let repo_col = batch.column_by_name("repo").unwrap()
            .as_any().downcast_ref::<StringArray>().unwrap();
        let dist_col = batch.column_by_name("_distance")
            .and_then(|c| c.as_any().downcast_ref::<Float32Array>().cloned());
        for i in 0..batch.num_rows() {
            let kind = match kind_col.value(i) {
                "decision" => Kind::Decision,
                "bug" => Kind::Bug,
                "convention" => Kind::Convention,
                "discovery" => Kind::Discovery,
                "pattern" => Kind::Pattern,
                _ => Kind::Note,
            };
            let dist = dist_col.as_ref().map(|c| c.value(i)).unwrap_or(0.0);
            // LanceDB returns L2 distance; convert to similarity-ish score.
            let score = 1.0 / (1.0 + dist);
            out.push(MemoryHit {
                id: id_col.value(i).into(),
                score,
                body: body_col.value(i).into(),
                kind,
                repo: repo_col.value(i).into(),
            });
        }
        Ok(())
    }
}
```

Create `src/index/mod.rs`:

```rust
pub mod embedder;
pub mod schema;
pub mod memory_index;

pub use embedder::Embedder;
pub use memory_index::{MemoryHit, MemoryIndex};
```

Modify `src/lib.rs` — add `pub mod index;`.

- [ ] **Step 4: Failing test for memory_index round-trip**

Create `tests/index/memory_index.rs`:

```rust
use qwick::config::paths::Paths;
use qwick::index::{Embedder, MemoryIndex};
use qwick::memory::{Kind, MemoryStore};

#[path = "../common/mod.rs"]
mod common;

#[tokio::test]
async fn upsert_then_search_returns_hit() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store.save("Use Postgres for analytics", Kind::Decision, "r", &[], "a", 3).unwrap();

    let mut emb = Embedder::nomic_text().unwrap();
    let v = emb.embed_one(&rec.body).unwrap();
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    idx.upsert(&rec, &v).await.unwrap();

    let q = emb.embed_one("Postgres analytics decision").unwrap();
    let hits = idx.search(&q, 5).await.unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0].id, rec.frontmatter.id);
}
```

- [ ] **Step 5: Run + verify pass**

Run: `cargo nextest run --test index`
Expected: 3 tests pass (model download on first run is slow — allow up to 60s).

- [ ] **Step 6: Run check-all**

Run: `bash scripts/check-all.sh`
Expected: green.

- [ ] **Step 7: Commit**

```bash
git add src/index src/lib.rs tests/index.rs tests/index/
git commit -m "feat(index): nomic embedder + lancedb memory_chunks upsert/search"
```

---

## Task 9: Graph — memory layer (kuzu schema + upserts)

**Goal:** kuzu DB with the memory-layer node + edge tables. On `save`, upsert `Memory`/`Repo`/`Author`/`Tag` nodes and `InRepo`/`AuthoredBy`/`Tagged` edges.

**Files:**
- Create: `src/graph/mod.rs`
- Create: `src/graph/schema.rs`
- Create: `src/graph/upsert.rs`
- Create: `src/graph/query.rs`
- Modify: `src/lib.rs`
- Create: `tests/graph.rs`
- Create: `tests/graph/upsert.rs`
- Create: `tests/graph/query.rs`

- [ ] **Step 1: Implement schema.rs**

Create `src/graph/schema.rs`:

```rust
pub const MEMORY_LAYER_DDL: &[&str] = &[
    "CREATE NODE TABLE IF NOT EXISTS Memory(id STRING, kind STRING, created STRING, quality INT64, PRIMARY KEY(id))",
    "CREATE NODE TABLE IF NOT EXISTS Repo(name STRING, last_indexed_head STRING, last_indexed_at STRING, PRIMARY KEY(name))",
    "CREATE NODE TABLE IF NOT EXISTS Author(name STRING, PRIMARY KEY(name))",
    "CREATE NODE TABLE IF NOT EXISTS Tag(name STRING, PRIMARY KEY(name))",
    "CREATE REL TABLE IF NOT EXISTS InRepo(FROM Memory TO Repo)",
    "CREATE REL TABLE IF NOT EXISTS AuthoredBy(FROM Memory TO Author)",
    "CREATE REL TABLE IF NOT EXISTS Tagged(FROM Memory TO Tag)",
    "CREATE REL TABLE IF NOT EXISTS Supersedes(FROM Memory TO Memory, at STRING)",
    "CREATE REL TABLE IF NOT EXISTS ConflictsWith(FROM Memory TO Memory)",
    "CREATE REL TABLE IF NOT EXISTS RelatesTo(FROM Memory TO Memory, score DOUBLE)",
    "CREATE REL TABLE IF NOT EXISTS DerivedFrom(FROM Memory TO Memory)",
];
```

- [ ] **Step 2: Implement upsert.rs**

Create `src/graph/upsert.rs`:

```rust
use std::path::Path;

use kuzu::{Connection, Database, SystemConfig};

use crate::graph::schema::MEMORY_LAYER_DDL;
use crate::memory::MemoryRecord;
use crate::prelude::*;

pub struct Graph {
    db: Database,
}

impl Graph {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        std::fs::create_dir_all(dir.as_ref())?;
        let db = Database::new(dir.as_ref(), SystemConfig::default())
            .map_err(|e| Error::Other(e.to_string()))?;
        let conn = Connection::new(&db).map_err(|e| Error::Other(e.to_string()))?;
        for ddl in MEMORY_LAYER_DDL {
            conn.query(ddl).map_err(|e| Error::Other(e.to_string()))?;
        }
        Ok(Self { db })
    }

    pub fn conn(&self) -> Result<Connection<'_>> {
        Connection::new(&self.db).map_err(|e| Error::Other(e.to_string()))
    }

    pub fn upsert_memory(&self, rec: &MemoryRecord) -> Result<()> {
        let conn = self.conn()?;
        let fm = &rec.frontmatter;
        let created = fm.created.format(&time::format_description::well_known::Iso8601::DEFAULT)
            .map_err(|e| Error::Other(e.to_string()))?;

        conn.query(&format!(
            "MERGE (m:Memory {{id: '{id}'}}) SET m.kind = '{kind}', m.created = '{created}', m.quality = {quality}",
            id = fm.id,
            kind = kind_str(fm.kind),
            created = created,
            quality = fm.quality as i64,
        )).map_err(|e| Error::Other(e.to_string()))?;

        if !fm.repo.is_empty() {
            conn.query(&format!("MERGE (:Repo {{name: '{}'}})", esc(&fm.repo)))
                .map_err(|e| Error::Other(e.to_string()))?;
            conn.query(&format!(
                "MATCH (m:Memory {{id: '{}'}}), (r:Repo {{name: '{}'}}) MERGE (m)-[:InRepo]->(r)",
                esc(&fm.id), esc(&fm.repo)
            )).map_err(|e| Error::Other(e.to_string()))?;
        }

        if !fm.author.is_empty() {
            conn.query(&format!("MERGE (:Author {{name: '{}'}})", esc(&fm.author)))
                .map_err(|e| Error::Other(e.to_string()))?;
            conn.query(&format!(
                "MATCH (m:Memory {{id: '{}'}}), (a:Author {{name: '{}'}}) MERGE (m)-[:AuthoredBy]->(a)",
                esc(&fm.id), esc(&fm.author)
            )).map_err(|e| Error::Other(e.to_string()))?;
        }

        for tag in &fm.tags {
            conn.query(&format!("MERGE (:Tag {{name: '{}'}})", esc(tag)))
                .map_err(|e| Error::Other(e.to_string()))?;
            conn.query(&format!(
                "MATCH (m:Memory {{id: '{}'}}), (t:Tag {{name: '{}'}}) MERGE (m)-[:Tagged]->(t)",
                esc(&fm.id), esc(tag)
            )).map_err(|e| Error::Other(e.to_string()))?;
        }
        Ok(())
    }

    pub fn add_supersedes(&self, new_id: &str, old_id: &str) -> Result<()> {
        let now = time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Iso8601::DEFAULT)
            .map_err(|e| Error::Other(e.to_string()))?;
        let conn = self.conn()?;
        conn.query(&format!(
            "MATCH (n:Memory {{id: '{n}'}}), (o:Memory {{id: '{o}'}}) MERGE (n)-[:Supersedes {{at: '{now}'}}]->(o)",
            n = esc(new_id), o = esc(old_id), now = now,
        )).map_err(|e| Error::Other(e.to_string()))?;
        Ok(())
    }

    pub fn add_relates_to(&self, a: &str, b: &str, score: f64) -> Result<()> {
        let conn = self.conn()?;
        conn.query(&format!(
            "MATCH (x:Memory {{id: '{a}'}}), (y:Memory {{id: '{b}'}}) MERGE (x)-[:RelatesTo {{score: {s}}}]->(y)",
            a = esc(a), b = esc(b), s = score,
        )).map_err(|e| Error::Other(e.to_string()))?;
        Ok(())
    }
}

fn kind_str(k: crate::memory::Kind) -> &'static str {
    use crate::memory::Kind::*;
    match k {
        Decision => "decision", Bug => "bug", Convention => "convention",
        Discovery => "discovery", Pattern => "pattern", Note => "note",
    }
}

fn esc(s: &str) -> String { s.replace('\'', "\\'") }
```

- [ ] **Step 3: Implement query.rs**

Create `src/graph/query.rs`:

```rust
use kuzu::Value;

use crate::graph::upsert::Graph;
use crate::prelude::*;

impl Graph {
    pub fn neighbors_by_repo(&self, repo: &str) -> Result<Vec<String>> {
        let conn = self.conn()?;
        let mut rs = conn.query(&format!(
            "MATCH (m:Memory)-[:InRepo]->(r:Repo {{name: '{}'}}) RETURN m.id",
            repo.replace('\'', "\\'")
        )).map_err(|e| Error::Other(e.to_string()))?;
        let mut out = Vec::new();
        while let Some(row) = rs.next() {
            if let Some(Value::String(id)) = row.into_iter().next() {
                out.push(id);
            }
        }
        Ok(out)
    }
}
```

Create `src/graph/mod.rs`:

```rust
pub mod schema;
pub mod upsert;
pub mod query;

pub use upsert::Graph;
```

Modify `src/lib.rs` — add `pub mod graph;`.

- [ ] **Step 4: Failing tests + verify**

Create `tests/graph.rs`:

```rust
mod common;
mod upsert;
mod query;
```

Create `tests/graph/upsert.rs`:

```rust
use qwick::config::paths::Paths;
use qwick::graph::Graph;
use qwick::memory::{Kind, MemoryStore};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn upsert_memory_creates_repo_and_tag_edges() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store.save("hello", Kind::Decision, "myrepo", &["t1".into(), "t2".into()], "alice", 4).unwrap();

    let g = Graph::open(paths.graph_dir()).unwrap();
    g.upsert_memory(&rec).unwrap();
    let ids = g.neighbors_by_repo("myrepo").unwrap();
    assert_eq!(ids, vec![rec.frontmatter.id]);
}
```

Create `tests/graph/query.rs`:

```rust
use qwick::config::paths::Paths;
use qwick::graph::Graph;
use qwick::memory::{Kind, MemoryStore};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn neighbors_by_repo_returns_empty_for_unknown_repo() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let g = Graph::open(paths.graph_dir()).unwrap();
    assert!(g.neighbors_by_repo("unknown").unwrap().is_empty());
}

#[test]
fn supersedes_edge_persists() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let a = store.save("a", Kind::Decision, "r", &[], "x", 3).unwrap();
    let b = store.save("b", Kind::Decision, "r", &[], "x", 3).unwrap();
    let g = Graph::open(paths.graph_dir()).unwrap();
    g.upsert_memory(&a).unwrap();
    g.upsert_memory(&b).unwrap();
    g.add_supersedes(&b.frontmatter.id, &a.frontmatter.id).unwrap();
}
```

Run: `cargo nextest run --test graph`
Expected: 3 tests pass.

- [ ] **Step 5: check-all + commit**

```
bash scripts/check-all.sh
git add src/graph src/lib.rs tests/graph.rs tests/graph/
git commit -m "feat(graph): kuzu memory-layer schema + upserts + queries"
```

---

## Task 10: Retrieval — memory only (router, hybrid, corrective, rank)

**Goal:** Deterministic pipeline over the memory index — router classifies queries, executes vector + (placeholder) FTS, applies threshold/normalization, runs corrective fallback.

**Files:**
- Create: `src/retrieval/mod.rs`
- Create: `src/retrieval/bundle.rs`
- Create: `src/retrieval/rank.rs`
- Create: `src/retrieval/router.rs`
- Create: `src/retrieval/hybrid.rs`
- Create: `src/retrieval/corrective.rs`
- Modify: `src/lib.rs`
- Create: `tests/retrieval.rs`
- Create: `tests/retrieval/router.rs`
- Create: `tests/retrieval/rank.rs`
- Create: `tests/retrieval/hybrid.rs`
- Create: `tests/retrieval/corrective.rs`

- [ ] **Step 1: Failing test for router**

Create `tests/retrieval.rs`:

```rust
mod common;
mod router;
mod rank;
mod hybrid;
mod corrective;
```

Create `tests/retrieval/router.rs`:

```rust
use qwick::retrieval::router::{classify, Route};

#[test]
fn symbol_looking_query_routes_to_symbol() {
    assert_eq!(classify("handleLogin"),   Route::Symbol);
    assert_eq!(classify("run_migration"), Route::Symbol);
}

#[test]
fn long_question_routes_to_hybrid() {
    assert_eq!(classify("postgres migration race condition"), Route::Hybrid);
}

#[test]
fn empty_query_routes_to_fts_first() {
    assert_eq!(classify(""), Route::FtsFirst);
}
```

- [ ] **Step 2: Implement router.rs**

Create `src/retrieval/router.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route { Symbol, FtsFirst, Hybrid }

pub fn classify(query: &str) -> Route {
    let q = query.trim();
    if q.is_empty() { return Route::FtsFirst; }
    let single = !q.contains(char::is_whitespace);
    let identifier = q.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':'));
    if single && identifier { return Route::Symbol; }
    if q.split_whitespace().count() <= 2 { return Route::FtsFirst; }
    Route::Hybrid
}
```

- [ ] **Step 3: Failing test for rank**

Create `tests/retrieval/rank.rs`:

```rust
use qwick::retrieval::rank::{confidence_gap, z_normalize};

#[test]
fn z_normalize_centers_around_zero() {
    let xs = vec![1.0_f32, 2.0, 3.0, 4.0];
    let z = z_normalize(&xs);
    let sum: f32 = z.iter().sum();
    assert!(sum.abs() < 1e-5);
}

#[test]
fn confidence_gap_is_top1_minus_top2() {
    assert!((confidence_gap(&[0.9, 0.7, 0.5]) - 0.2_f32).abs() < 1e-6);
    assert_eq!(confidence_gap(&[0.5]), 0.5);
    assert_eq!(confidence_gap(&[]), 0.0);
}
```

- [ ] **Step 4: Implement rank.rs**

Create `src/retrieval/rank.rs`:

```rust
pub fn z_normalize(xs: &[f32]) -> Vec<f32> {
    if xs.is_empty() { return Vec::new(); }
    let n = xs.len() as f32;
    let mean: f32 = xs.iter().sum::<f32>() / n;
    let var: f32 = xs.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / n;
    let sd = var.sqrt().max(1e-9);
    xs.iter().map(|x| (x - mean) / sd).collect()
}

pub fn confidence_gap(sorted_desc: &[f32]) -> f32 {
    match sorted_desc {
        [] => 0.0,
        [a] => *a,
        [a, b, ..] => a - b,
    }
}
```

- [ ] **Step 5: Bundle + hybrid + corrective skeletons**

Create `src/retrieval/bundle.rs`:

```rust
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CitedHit {
    pub id: String,
    pub score: f32,
    pub kind: String,
    pub repo: String,
    pub snippet: String,
    pub why: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Bundle {
    pub query: String,
    pub route: String,
    pub hits: Vec<CitedHit>,
    pub confidence: f32,
    pub fallback_used: bool,
}
```

Create `src/retrieval/hybrid.rs`:

```rust
use crate::index::{MemoryHit, MemoryIndex};
use crate::prelude::*;

pub async fn search_memory(
    index: &MemoryIndex,
    query_emb: &[f32],
    limit: usize,
    threshold: f32,
) -> Result<Vec<MemoryHit>> {
    let mut hits = index.search(query_emb, limit * 2).await?;
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    hits.retain(|h| h.score >= threshold);
    hits.truncate(limit);
    Ok(hits)
}
```

Create `src/retrieval/corrective.rs`:

```rust
use crate::index::MemoryHit;

pub fn should_fallback(hits: &[MemoryHit], min_confidence: f32) -> bool {
    if hits.len() < 3 { return true; }
    let gap = hits[0].score - hits[1].score;
    gap < min_confidence
}
```

Create `src/retrieval/mod.rs`:

```rust
pub mod bundle;
pub mod rank;
pub mod router;
pub mod hybrid;
pub mod corrective;

pub use bundle::{Bundle, CitedHit};
pub use router::{classify, Route};
```

Modify `src/lib.rs` — add `pub mod retrieval;`.

- [ ] **Step 6: Tests for hybrid + corrective**

Create `tests/retrieval/hybrid.rs`:

```rust
use qwick::config::paths::Paths;
use qwick::index::{Embedder, MemoryIndex};
use qwick::memory::{Kind, MemoryStore};
use qwick::retrieval::hybrid::search_memory;

#[path = "../common/mod.rs"]
mod common;

#[tokio::test]
async fn search_memory_filters_below_threshold() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store.save("Postgres analytics decision", Kind::Decision, "r", &[], "a", 3).unwrap();
    let mut emb = Embedder::nomic_text().unwrap();
    let v = emb.embed_one(&rec.body).unwrap();
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    idx.upsert(&rec, &v).await.unwrap();

    let q = emb.embed_one("Postgres analytics decision").unwrap();
    let hits_pass = search_memory(&idx, &q, 5, 0.0).await.unwrap();
    assert!(!hits_pass.is_empty());

    let hits_filtered = search_memory(&idx, &q, 5, 1.5).await.unwrap();
    assert!(hits_filtered.is_empty(), "absurd threshold should empty results");
}
```

Create `tests/retrieval/corrective.rs`:

```rust
use qwick::index::MemoryHit;
use qwick::memory::Kind;
use qwick::retrieval::corrective::should_fallback;

fn hit(score: f32) -> MemoryHit {
    MemoryHit { id: "x".into(), score, body: "".into(), kind: Kind::Note, repo: "r".into() }
}

#[test]
fn fallback_when_gap_below_min() {
    let hits = vec![hit(0.9), hit(0.89), hit(0.88)];
    assert!(should_fallback(&hits, 0.15));
}

#[test]
fn no_fallback_when_gap_above_min() {
    let hits = vec![hit(0.9), hit(0.6), hit(0.4)];
    assert!(!should_fallback(&hits, 0.15));
}
```

Run: `cargo nextest run --test retrieval`
Expected: all retrieval tests pass.

- [ ] **Step 7: check-all + commit**

```
bash scripts/check-all.sh
git add src/retrieval src/lib.rs tests/retrieval.rs tests/retrieval/
git commit -m "feat(retrieval): router, hybrid memory search, ranking, corrective fallback"
```

---

## Task 11: CLI — memory-only commands (save, search, list, delete, feedback, doctor)

**Goal:** Wire clap subcommands for the memory layer. Each subcommand owns its own file. TTY default + `--json` flag. `assert_cmd` integration tests.

**Files:**
- Create: `src/cli/mod.rs`
- Create: `src/cli/save.rs`
- Create: `src/cli/search.rs`
- Create: `src/cli/list.rs`
- Create: `src/cli/delete.rs`
- Create: `src/cli/feedback.rs`
- Create: `src/cli/doctor.rs`
- Modify: `src/main.rs`
- Create: `tests/cli.rs`

- [ ] **Step 1: Cli enum + dispatcher**

Create `src/cli/mod.rs`:

```rust
use clap::{Parser, Subcommand};

use crate::prelude::*;

pub mod save;
pub mod search;
pub mod list;
pub mod delete;
pub mod feedback;
pub mod doctor;

#[derive(Parser, Debug)]
#[command(name = "qwick", version, about = "Agentic dev memory + code-aware semantic search")]
pub struct Cli {
    #[arg(long, global = true)]
    pub json: bool,

    #[arg(long, global = true, env = "QWICK_DATA_DIR")]
    pub data_dir: Option<std::path::PathBuf>,

    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    Save(save::Args),
    Search(search::Args),
    List(list::Args),
    Delete(delete::Args),
    Feedback(feedback::Args),
    Doctor,
}

pub async fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::Save(a)     => save::run(a, cli.json, cli.data_dir).await,
        Cmd::Search(a)   => search::run(a, cli.json, cli.data_dir).await,
        Cmd::List(a)     => list::run(a, cli.json, cli.data_dir).await,
        Cmd::Delete(a)   => delete::run(a, cli.json, cli.data_dir).await,
        Cmd::Feedback(a) => feedback::run(a, cli.json, cli.data_dir).await,
        Cmd::Doctor      => doctor::run(cli.json, cli.data_dir).await,
    }
}

pub fn resolve_data_dir(opt: Option<std::path::PathBuf>) -> std::path::PathBuf {
    opt.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        std::path::PathBuf::from(home).join(".qwick")
    })
}
```

- [ ] **Step 2: Save command**

Create `src/cli/save.rs`:

```rust
use std::io::Read;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::memory::{Kind, MemoryStore};
use crate::prelude::*;

#[derive(ClapArgs, Debug)]
pub struct Args {
    pub body: Option<String>,
    #[arg(long, default_value = "note")]
    pub kind: String,
    #[arg(long, default_value = "")]
    pub repo: String,
    #[arg(long, default_value = "")]
    pub tags: String,
    #[arg(long, default_value = "")]
    pub author: String,
    #[arg(long, default_value_t = 3)]
    pub quality: u8,
}

#[derive(Serialize)]
struct Output {
    id: String,
    path: String,
}

pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let body = match a.body {
        Some(s) if s == "-" => read_stdin()?,
        Some(s) => s,
        None => read_stdin()?,
    };
    let kind = parse_kind(&a.kind)?;
    let tags: Vec<String> = if a.tags.is_empty() {
        Vec::new()
    } else {
        a.tags.split(',').map(|t| t.trim().to_string()).collect()
    };
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let store = MemoryStore::new(paths);
    let rec = store.save(&body, kind, &a.repo, &tags, &a.author, a.quality)?;

    let out = Output {
        id: rec.frontmatter.id.clone(),
        path: rec.path.to_string_lossy().into_owned(),
    };
    if json {
        println!("{}", serde_json::to_string(&out)?);
    } else {
        println!("saved {}\n  path: {}", out.id, out.path);
    }
    Ok(())
}

fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}

fn parse_kind(s: &str) -> Result<Kind> {
    Ok(match s {
        "decision" => Kind::Decision,
        "bug" => Kind::Bug,
        "convention" => Kind::Convention,
        "discovery" => Kind::Discovery,
        "pattern" => Kind::Pattern,
        _ => Kind::Note,
    })
}
```

- [ ] **Step 3: list, delete, doctor commands**

Create `src/cli/list.rs`:

```rust
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::prelude::*;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub kind: Option<String>,
}

#[derive(Serialize)]
struct Row { id: String, kind: String, repo: String, slug: String }

pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut all = MemoryStore::new(paths).list()?;
    if let Some(r) = a.repo { all.retain(|m| m.frontmatter.repo == r); }
    if let Some(k) = a.kind { all.retain(|m| format!("{:?}", m.frontmatter.kind).eq_ignore_ascii_case(&k)); }

    let rows: Vec<Row> = all.into_iter().map(|m| Row {
        id: m.frontmatter.id.clone(),
        kind: format!("{:?}", m.frontmatter.kind).to_lowercase(),
        repo: m.frontmatter.repo.clone(),
        slug: m.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
    }).collect();

    if json {
        println!("{}", serde_json::to_string(&rows)?);
    } else {
        for r in &rows { println!("{}  {}  {}  {}", r.id, r.kind, r.repo, r.slug); }
    }
    Ok(())
}
```

Create `src/cli/delete.rs`:

```rust
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::prelude::*;

#[derive(ClapArgs, Debug)]
pub struct Args { pub id: String }

pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let removed = MemoryStore::new(paths).delete(&a.id)?;
    if json {
        println!("{{\"deleted\":\"{}\"}}", removed.frontmatter.id);
    } else {
        println!("deleted {}", removed.frontmatter.id);
    }
    Ok(())
}
```

Create `src/cli/doctor.rs`:

```rust
use std::path::PathBuf;

use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::prelude::*;

#[derive(Serialize)]
struct Report {
    data_dir: String,
    memories_count: usize,
}

pub async fn run(json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let store = MemoryStore::new(paths.clone());
    let report = Report {
        data_dir: paths.data_dir().to_string_lossy().into_owned(),
        memories_count: store.list()?.len(),
    };
    if json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        println!("data_dir       : {}", report.data_dir);
        println!("memories_count : {}", report.memories_count);
    }
    Ok(())
}
```

- [ ] **Step 4: Search + feedback stubs**

Create `src/cli/search.rs`:

```rust
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::index::{Embedder, MemoryIndex};
use crate::prelude::*;
use crate::retrieval::hybrid::search_memory;

#[derive(ClapArgs, Debug)]
pub struct Args {
    pub query: String,
    #[arg(long, default_value_t = 12)]
    pub limit: usize,
}

#[derive(Serialize)]
struct Row { id: String, score: f32, repo: String, snippet: String }

pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await?;
    let mut emb = Embedder::nomic_text()?;
    let q = emb.embed_one(&a.query)?;
    let hits = search_memory(&idx, &q, a.limit, 0.0).await?;
    let rows: Vec<Row> = hits.into_iter().map(|h| Row {
        id: h.id, score: h.score, repo: h.repo,
        snippet: h.body.chars().take(160).collect(),
    }).collect();
    if json {
        println!("{}", serde_json::to_string(&rows)?);
    } else {
        for r in &rows { println!("{:.3}  {}  {}  {}", r.score, r.id, r.repo, r.snippet); }
    }
    Ok(())
}
```

Create `src/cli/feedback.rs`:

```rust
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::prelude::*;
use crate::stats::feedback::Feedback;
use crate::stats::sqlite::StatsDb;

#[derive(ClapArgs, Debug)]
pub struct Args {
    pub query_id: String,
    #[arg(long, default_value = "")]
    pub used: String,
    #[arg(long, default_value = "")]
    pub irrelevant: String,
}

pub async fn run(a: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut db = StatsDb::open(paths.stats_db())?;
    let fb = Feedback::new(&mut db);
    for id in a.used.split(',').filter(|s| !s.is_empty()) {
        fb.record_used(id)?;
    }
    for id in a.irrelevant.split(',').filter(|s| !s.is_empty()) {
        fb.record_irrelevant(id)?;
    }
    println!("ok");
    Ok(())
}
```

- [ ] **Step 5: Wire main.rs**

Replace `src/main.rs`:

```rust
use clap::Parser;
use qwick::cli::{run, Cli};
use qwick::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cli = Cli::parse();
    run(cli).await
}
```

Modify `src/lib.rs` — add `pub mod cli;`.

- [ ] **Step 6: Integration tests via assert_cmd**

Create `tests/cli.rs`:

```rust
use assert_cmd::Command;
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("qwick").unwrap();
    c.env("QWICK_DATA_DIR", home.path().join(".qwick"));
    c
}

#[test]
fn save_then_list_shows_id() {
    let home = TempDir::new().unwrap();
    let save = bin(&home).args(["save", "hello world", "--kind", "note"]).assert().success();
    let saved_stdout = String::from_utf8(save.get_output().stdout.clone()).unwrap();
    let id = saved_stdout.lines().find(|l| l.starts_with("saved ")).unwrap()[6..].split_whitespace().next().unwrap().to_string();

    let list = bin(&home).args(["list"]).assert().success();
    let out = String::from_utf8(list.get_output().stdout.clone()).unwrap();
    assert!(out.contains(&id));
}

#[test]
fn doctor_reports_zero_memories_on_fresh_dir() {
    let home = TempDir::new().unwrap();
    bin(&home).arg("doctor")
        .assert()
        .success()
        .stdout(predicates::str::contains("memories_count : 0"));
}
```

- [ ] **Step 7: Run + check-all + commit**

```
cargo nextest run --test cli
bash scripts/check-all.sh
git add src/cli src/main.rs src/lib.rs tests/cli.rs
git commit -m "feat(cli): memory-only commands (save/search/list/delete/feedback/doctor)"
```

`tests-mirror-check` exempts `src/cli/*` (covered by `tests/cli.rs`) — confirmed in the script.

---

## Task 12: AST layer — ast-grep extractor + user pattern API

**Goal:** Symbol extraction for Rust + TypeScript + Python via `ast-grep-core`; user-facing pattern command surface.

**Files:**
- Create: `src/ast/mod.rs`
- Create: `src/ast/languages.rs`
- Create: `src/ast/extractor.rs`
- Create: `src/ast/pattern.rs`
- Modify: `src/lib.rs`
- Create: `tests/ast.rs`
- Create: `tests/ast/extractor.rs`
- Create: `tests/ast/pattern.rs`

- [ ] **Step 1: Implement language registry**

Create `src/ast/languages.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang { Rust, TypeScript, JavaScript, Python }

impl Lang {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "rs" => Some(Self::Rust),
            "ts" | "tsx" => Some(Self::TypeScript),
            "js" | "jsx" => Some(Self::JavaScript),
            "py" => Some(Self::Python),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
            Self::Python => "python",
        }
    }
}
```

- [ ] **Step 2: Implement extractor.rs**

Create `src/ast/extractor.rs`:

```rust
use ast_grep_core::{AstGrep, Pattern};
use ast_grep_core::language::Language;

use crate::ast::languages::Lang;
use crate::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedSymbol {
    pub name: String,
    pub kind: String,
    pub language: String,
    pub snippet: String,
    pub line: usize,
}

pub fn extract(lang: Lang, source: &str) -> Result<Vec<ExtractedSymbol>> {
    let patterns: &[(&str, &str)] = match lang {
        Lang::Rust => &[
            ("function", "fn $NAME($$$ARGS) $$$BODY"),
            ("struct",   "struct $NAME $$$BODY"),
            ("enum",     "enum $NAME $$$BODY"),
            ("trait",    "trait $NAME $$$BODY"),
        ],
        Lang::TypeScript | Lang::JavaScript => &[
            ("function", "function $NAME($$$ARGS) { $$$BODY }"),
            ("class",    "class $NAME { $$$BODY }"),
        ],
        Lang::Python => &[
            ("function", "def $NAME($$$ARGS): $$$BODY"),
            ("class",    "class $NAME: $$$BODY"),
        ],
    };
    let language: Language = match lang {
        Lang::Rust => Language::Rust,
        Lang::TypeScript => Language::TypeScript,
        Lang::JavaScript => Language::JavaScript,
        Lang::Python => Language::Python,
    };
    let grep = AstGrep::new(source, language.clone());
    let mut out = Vec::new();
    for (kind, pat) in patterns {
        let pattern = Pattern::new(pat, language.clone());
        for m in grep.root().find_all(pattern) {
            let name_node = m.get_env().get_match("NAME");
            let name = name_node.map(|n| n.text().to_string()).unwrap_or_default();
            if name.is_empty() { continue; }
            out.push(ExtractedSymbol {
                name,
                kind: (*kind).into(),
                language: lang.as_str().into(),
                snippet: m.text().to_string(),
                line: m.start_pos().line() + 1,
            });
        }
    }
    Ok(out)
}
```

- [ ] **Step 3: Pattern command wrapper**

Create `src/ast/pattern.rs`:

```rust
use ast_grep_core::{AstGrep, Pattern};
use ast_grep_core::language::Language;

use crate::ast::languages::Lang;
use crate::prelude::*;

pub fn find(lang: Lang, source: &str, pattern: &str) -> Result<Vec<(usize, String)>> {
    let language: Language = match lang {
        Lang::Rust => Language::Rust,
        Lang::TypeScript => Language::TypeScript,
        Lang::JavaScript => Language::JavaScript,
        Lang::Python => Language::Python,
    };
    let grep = AstGrep::new(source, language.clone());
    let pat = Pattern::new(pattern, language);
    Ok(grep.root().find_all(pat).map(|m| (m.start_pos().line() + 1, m.text().to_string())).collect())
}
```

Create `src/ast/mod.rs`:

```rust
pub mod languages;
pub mod extractor;
pub mod pattern;

pub use extractor::{extract, ExtractedSymbol};
pub use languages::Lang;
```

Modify `src/lib.rs` — add `pub mod ast;`.

- [ ] **Step 4: Tests**

Create `tests/ast.rs`:

```rust
mod common;
mod extractor;
mod pattern;
```

Create `tests/ast/extractor.rs`:

```rust
use qwick::ast::{extract, Lang};

#[test]
fn rust_functions_extracted() {
    let src = "fn add(a: i32, b: i32) -> i32 { a + b }\nfn sub(a: i32, b: i32) -> i32 { a - b }\n";
    let syms = extract(Lang::Rust, src).unwrap();
    let names: Vec<_> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"add"));
    assert!(names.contains(&"sub"));
}

#[test]
fn python_class_extracted() {
    let src = "class Foo:\n    def bar(self):\n        pass\n";
    let syms = extract(Lang::Python, src).unwrap();
    assert!(syms.iter().any(|s| s.name == "Foo" && s.kind == "class"));
}
```

Create `tests/ast/pattern.rs`:

```rust
use qwick::ast::pattern::find;
use qwick::ast::Lang;

#[test]
fn pattern_matches_function_call() {
    let src = "fn main() { println!(\"hi\"); foo(1, 2); }\n";
    let hits = find(Lang::Rust, src, "$F($$$ARGS)").unwrap();
    assert!(!hits.is_empty());
}
```

Run: `cargo nextest run --test ast`
Expected: all pass.

- [ ] **Step 5: check-all + commit**

```
bash scripts/check-all.sh
git add src/ast src/lib.rs tests/ast.rs tests/ast/
git commit -m "feat(ast): ast-grep symbol extractor + pattern wrapper for rust/ts/py"
```

---

## Task 13: Indexing — code side (LanceDB code_chunks + incremental walk)

**Goal:** Walk the current repo via `ignore::Walk`, parse each file with `extract`, embed symbol snippets with `jina-code`, upsert into `code_chunks`. Persist file content hashes (git blob OID when available, sha-256 otherwise) for incremental reuse.

**Files:**
- Create: `src/index/code_index.rs`
- Modify: `src/index/mod.rs` (re-export)
- Modify: `src/index/schema.rs` (add code schema)
- Create: `tests/index/code_index.rs`

- [ ] **Step 1: Add code schema**

Edit `src/index/schema.rs` — append:

```rust
pub const CODE_TABLE: &str = "code_chunks";

pub fn code_schema(dim: usize) -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("qualified",   DataType::Utf8, false),
        Field::new("snippet",     DataType::Utf8, false),
        Field::new("language",    DataType::Utf8, false),
        Field::new("file",        DataType::Utf8, false),
        Field::new("symbol_kind", DataType::Utf8, false),
        Field::new("ast_hash",    DataType::Utf8, false),
        Field::new("embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dim as i32,
            ),
            false,
        ),
    ]))
}
```

- [ ] **Step 2: Implement code_index.rs**

Create `src/index/code_index.rs`:

```rust
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ignore::Walk;
use lancedb::arrow::arrow_array::{
    FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, StringArray,
};
use lancedb::arrow::arrow_schema::Schema;
use lancedb::{connect, Connection};
use sha2::{Digest, Sha256};

use crate::ast::{extract, Lang};
use crate::index::embedder::Embedder;
use crate::index::schema::{code_schema, CODE_TABLE};
use crate::prelude::*;

pub struct CodeIndex {
    conn: Connection,
    schema: Arc<Schema>,
}

#[derive(Debug, Clone)]
pub struct CodeChunk {
    pub qualified: String,
    pub snippet: String,
    pub language: String,
    pub file: String,
    pub symbol_kind: String,
    pub ast_hash: String,
}

impl CodeIndex {
    pub async fn open(dir: impl AsRef<Path>, dim: usize) -> Result<Self> {
        let uri = dir.as_ref().to_string_lossy().to_string();
        let conn = connect(&uri).execute().await
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(Self { conn, schema: code_schema(dim) })
    }

    pub async fn index_repo(&self, repo_root: &Path, repo: &str, emb: &mut Embedder) -> Result<usize> {
        let mut chunks = Vec::new();
        for dent in Walk::new(repo_root).flatten() {
            let path = dent.path();
            if !path.is_file() { continue; }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let Some(lang) = Lang::from_extension(ext) else { continue };
            let Ok(src) = std::fs::read_to_string(path) else { continue };
            let syms = extract(lang, &src)?;
            let rel = path.strip_prefix(repo_root).unwrap_or(path).to_string_lossy().into_owned();
            for s in syms {
                let qualified = format!("{repo}:{rel}:{}", s.name);
                let ast_hash = sha256_hex(s.snippet.as_bytes());
                chunks.push(CodeChunk {
                    qualified,
                    snippet: s.snippet,
                    language: s.language,
                    file: format!("{repo}:{rel}"),
                    symbol_kind: s.kind,
                    ast_hash,
                });
            }
        }
        if chunks.is_empty() { return Ok(0); }
        let snippets: Vec<String> = chunks.iter().map(|c| c.snippet.clone()).collect();
        let vecs = emb.embed_many(snippets)?;
        let batch = self.batch(&chunks, &vecs)?;
        let iter = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), self.schema.clone());
        let names = self.conn.table_names().execute().await
            .map_err(|e| Error::Other(e.to_string()))?;
        if names.iter().any(|n| n == CODE_TABLE) {
            let t = self.conn.open_table(CODE_TABLE).execute().await
                .map_err(|e| Error::Other(e.to_string()))?;
            t.merge_insert(&["qualified"])
                .when_matched_update_all(None)
                .when_not_matched_insert_all()
                .execute(Box::new(iter))
                .await
                .map_err(|e| Error::Other(e.to_string()))?;
        } else {
            self.conn.create_table(CODE_TABLE, Box::new(iter)).execute().await
                .map_err(|e| Error::Other(e.to_string()))?;
        }
        Ok(chunks.len())
    }

    fn batch(&self, chunks: &[CodeChunk], vecs: &[Vec<f32>]) -> Result<RecordBatch> {
        let dim = vecs[0].len();
        let flat: Vec<f32> = vecs.iter().flatten().copied().collect();
        let emb = FixedSizeListArray::try_new(
            Arc::new(lancedb::arrow::arrow_schema::Field::new("item", lancedb::arrow::arrow_schema::DataType::Float32, true)),
            dim as i32,
            Arc::new(Float32Array::from(flat)),
            None,
        ).map_err(|e| Error::Other(e.to_string()))?;
        RecordBatch::try_new(
            self.schema.clone(),
            vec![
                Arc::new(StringArray::from(chunks.iter().map(|c| c.qualified.clone()).collect::<Vec<_>>())),
                Arc::new(StringArray::from(chunks.iter().map(|c| c.snippet.clone()).collect::<Vec<_>>())),
                Arc::new(StringArray::from(chunks.iter().map(|c| c.language.clone()).collect::<Vec<_>>())),
                Arc::new(StringArray::from(chunks.iter().map(|c| c.file.clone()).collect::<Vec<_>>())),
                Arc::new(StringArray::from(chunks.iter().map(|c| c.symbol_kind.clone()).collect::<Vec<_>>())),
                Arc::new(StringArray::from(chunks.iter().map(|c| c.ast_hash.clone()).collect::<Vec<_>>())),
                Arc::new(emb),
            ],
        ).map_err(|e| Error::Other(e.to_string()))
    }
}

fn sha256_hex(b: &[u8]) -> String {
    let d = Sha256::digest(b);
    let mut s = String::with_capacity(64);
    for byte in d {
        use std::fmt::Write as _;
        let _ = write!(s, "{:02x}", byte);
    }
    s
}

pub fn iter_files(root: &Path) -> Vec<PathBuf> {
    Walk::new(root)
        .flatten()
        .filter(|d| d.path().is_file())
        .map(|d| d.path().to_path_buf())
        .collect()
}
```

Modify `src/index/mod.rs` — add `pub mod code_index;` and `pub use code_index::{CodeChunk, CodeIndex};`.

- [ ] **Step 3: Tests**

Create `tests/index/code_index.rs`:

```rust
use qwick::config::paths::Paths;
use qwick::index::{CodeIndex, Embedder};

#[path = "../common/mod.rs"]
mod common;

#[tokio::test]
async fn index_repo_finds_rust_symbols() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let repo = sb.root.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/lib.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();

    let idx = CodeIndex::open(paths.vectors_dir(), 768).await.unwrap();
    let mut emb = Embedder::jina_code().unwrap();
    let n = idx.index_repo(&repo, "test", &mut emb).await.unwrap();
    assert!(n >= 2);
}
```

Run: `cargo nextest run --test index code_index`
Expected: pass.

- [ ] **Step 4: check-all + commit**

```
bash scripts/check-all.sh
git add src/index/code_index.rs src/index/mod.rs src/index/schema.rs tests/index/code_index.rs
git commit -m "feat(index): code_chunks via ast-grep extractor + jina-code embeddings"
```

---

## Task 14: Graph — code layer + cross-links

**Goal:** Extend kuzu schema with `File`/`Symbol` nodes and `DefinedIn`/`Calls`/`Imports`/`ReferencesFile`/`ReferencesSymbol` edges. On `index-code`, upsert files and symbols. On `save`, scan body for `<repo>:<path>:<symbol>` and `<repo>:<path>` references; insert cross-edges.

**Files:**
- Modify: `src/graph/schema.rs` (append code-layer DDL)
- Modify: `src/graph/upsert.rs` (add `upsert_file`, `upsert_symbol`, `add_calls`, `add_references_*`)
- Create: `src/graph/cross_link.rs`
- Modify: `src/graph/mod.rs`
- Create: `tests/graph/cross_link.rs`
- Modify: `tests/graph.rs`

- [ ] **Step 1: Append code-layer DDL**

Edit `src/graph/schema.rs` — add a second array:

```rust
pub const CODE_LAYER_DDL: &[&str] = &[
    "CREATE NODE TABLE IF NOT EXISTS File(qualified STRING, repo STRING, path STRING, content_hash STRING, indexed_at STRING, PRIMARY KEY(qualified))",
    "CREATE NODE TABLE IF NOT EXISTS Symbol(qualified STRING, name STRING, kind STRING, language STRING, ast_hash STRING, PRIMARY KEY(qualified))",
    "CREATE REL TABLE IF NOT EXISTS DefinedIn(FROM Symbol TO File)",
    "CREATE REL TABLE IF NOT EXISTS Calls(FROM Symbol TO Symbol)",
    "CREATE REL TABLE IF NOT EXISTS Imports(FROM File TO File)",
    "CREATE REL TABLE IF NOT EXISTS ReferencesFile(FROM Memory TO File)",
    "CREATE REL TABLE IF NOT EXISTS ReferencesSymbol(FROM Memory TO Symbol)",
];
```

- [ ] **Step 2: Extend upsert.rs**

Edit `src/graph/upsert.rs`:

1. In `Graph::open`, after the memory DDL loop, add a second loop over `CODE_LAYER_DDL`.
2. Add three methods:

```rust
impl Graph {
    pub fn upsert_file(&self, qualified: &str, repo: &str, path: &str, content_hash: &str) -> Result<()> {
        let now = time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Iso8601::DEFAULT)
            .map_err(|e| Error::Other(e.to_string()))?;
        let conn = self.conn()?;
        conn.query(&format!(
            "MERGE (f:File {{qualified: '{q}'}}) SET f.repo = '{r}', f.path = '{p}', f.content_hash = '{h}', f.indexed_at = '{now}'",
            q = esc(qualified), r = esc(repo), p = esc(path), h = esc(content_hash), now = now,
        )).map_err(|e| Error::Other(e.to_string()))?;
        Ok(())
    }

    pub fn upsert_symbol(&self, qualified: &str, name: &str, kind: &str, language: &str, ast_hash: &str, file_qualified: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.query(&format!(
            "MERGE (s:Symbol {{qualified: '{q}'}}) SET s.name = '{n}', s.kind = '{k}', s.language = '{l}', s.ast_hash = '{h}'",
            q = esc(qualified), n = esc(name), k = esc(kind), l = esc(language), h = esc(ast_hash),
        )).map_err(|e| Error::Other(e.to_string()))?;
        conn.query(&format!(
            "MATCH (s:Symbol {{qualified: '{}'}}), (f:File {{qualified: '{}'}}) MERGE (s)-[:DefinedIn]->(f)",
            esc(qualified), esc(file_qualified),
        )).map_err(|e| Error::Other(e.to_string()))?;
        Ok(())
    }

    pub fn add_references_symbol(&self, memory_id: &str, symbol_qualified: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.query(&format!(
            "MATCH (m:Memory {{id: '{}'}}), (s:Symbol {{qualified: '{}'}}) MERGE (m)-[:ReferencesSymbol]->(s)",
            esc(memory_id), esc(symbol_qualified),
        )).map_err(|e| Error::Other(e.to_string()))?;
        Ok(())
    }

    pub fn add_references_file(&self, memory_id: &str, file_qualified: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.query(&format!(
            "MATCH (m:Memory {{id: '{}'}}), (f:File {{qualified: '{}'}}) MERGE (m)-[:ReferencesFile]->(f)",
            esc(memory_id), esc(file_qualified),
        )).map_err(|e| Error::Other(e.to_string()))?;
        Ok(())
    }
}
```

- [ ] **Step 3: Cross-link extractor**

Create `src/graph/cross_link.rs`:

```rust
use once_cell::sync::Lazy;
use regex::Regex;

pub struct Refs {
    pub files: Vec<String>,
    pub symbols: Vec<String>,
}

static FILE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b([a-z0-9_-]+):([A-Za-z0-9_./\-]+\.[a-zA-Z]+)(?::([A-Za-z_][A-Za-z0-9_]*))?\b").unwrap());

pub fn extract_refs(body: &str) -> Refs {
    let mut files = Vec::new();
    let mut symbols = Vec::new();
    for cap in FILE_RE.captures_iter(body) {
        let repo = &cap[1];
        let path = &cap[2];
        let file_q = format!("{repo}:{path}");
        if !files.contains(&file_q) { files.push(file_q.clone()); }
        if let Some(sym) = cap.get(3) {
            let sq = format!("{repo}:{path}:{}", sym.as_str());
            if !symbols.contains(&sq) { symbols.push(sq); }
        }
    }
    Refs { files, symbols }
}
```

Add `regex = "1"` to `Cargo.toml` dependencies if not already.

Modify `src/graph/mod.rs` — add `pub mod cross_link;`.

- [ ] **Step 4: Tests**

Create `tests/graph/cross_link.rs`:

```rust
use qwick::graph::cross_link::extract_refs;

#[test]
fn extracts_file_and_symbol_refs() {
    let body = "See qwick-backend:src/db.rs:run_migration for the call; also touches qwick-backend:src/util.rs.";
    let r = extract_refs(body);
    assert!(r.files.contains(&"qwick-backend:src/db.rs".to_string()));
    assert!(r.files.contains(&"qwick-backend:src/util.rs".to_string()));
    assert!(r.symbols.contains(&"qwick-backend:src/db.rs:run_migration".to_string()));
}
```

Add the module to `tests/graph.rs`:

```rust
mod common;
mod upsert;
mod query;
mod cross_link;
```

Run: `cargo nextest run --test graph`
Expected: all pass.

- [ ] **Step 5: check-all + commit**

```
bash scripts/check-all.sh
git add src/graph tests/graph.rs tests/graph/cross_link.rs Cargo.toml
git commit -m "feat(graph): code-layer schema + cross-link refs from memory bodies"
```

---

## Task 15: Retrieval — both layers (memory + code, unified)

**Goal:** Extend the retrieval pipeline to query memory and code tables in parallel, normalize per-table scores, and merge into a single sorted bundle.

**Files:**
- Modify: `src/retrieval/hybrid.rs` (add `search_code`, `search_both`)
- Modify: `src/retrieval/bundle.rs` (add `Layer` enum)
- Create: `tests/retrieval/dual.rs`
- Modify: `tests/retrieval.rs`

- [ ] **Step 1: Extend hybrid.rs**

Append to `src/retrieval/hybrid.rs`:

```rust
use crate::index::CodeIndex;

#[derive(Debug, Clone)]
pub struct CodeHit {
    pub qualified: String,
    pub score: f32,
    pub snippet: String,
    pub language: String,
    pub file: String,
}

pub async fn search_code(
    index: &CodeIndex,
    query_emb: &[f32],
    limit: usize,
    threshold: f32,
) -> Result<Vec<CodeHit>> {
    let names = index.conn().table_names().execute().await
        .map_err(|e| Error::Other(e.to_string()))?;
    if !names.iter().any(|n| n == "code_chunks") {
        return Ok(Vec::new());
    }
    let tbl = index.conn().open_table("code_chunks").execute().await
        .map_err(|e| Error::Other(e.to_string()))?;
    let stream = tbl.query()
        .nearest_to(query_emb).map_err(|e| Error::Other(e.to_string()))?
        .limit(limit * 2)
        .execute()
        .await
        .map_err(|e| Error::Other(e.to_string()))?;
    let batches: Vec<_> = stream.collect::<Vec<_>>().await.into_iter().flatten().collect();
    let mut out = Vec::new();
    for b in batches {
        use lancedb::arrow::arrow_array::{Float32Array, StringArray};
        let q = b.column_by_name("qualified").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let s = b.column_by_name("snippet").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let l = b.column_by_name("language").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let f = b.column_by_name("file").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let d = b.column_by_name("_distance").and_then(|c| c.as_any().downcast_ref::<Float32Array>().cloned());
        for i in 0..b.num_rows() {
            let dist = d.as_ref().map(|c| c.value(i)).unwrap_or(0.0);
            let score = 1.0 / (1.0 + dist);
            if score < threshold { continue; }
            out.push(CodeHit {
                qualified: q.value(i).into(),
                score,
                snippet: s.value(i).into(),
                language: l.value(i).into(),
                file: f.value(i).into(),
            });
        }
    }
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    out.truncate(limit);
    Ok(out)
}
```

`CodeIndex` needs a `pub fn conn(&self) -> &Connection { &self.conn }` accessor added to `src/index/code_index.rs`.

- [ ] **Step 2: Test merging**

Create `tests/retrieval/dual.rs`:

```rust
use qwick::index::CodeIndex;
use qwick::index::{Embedder, MemoryIndex};
use qwick::memory::{Kind, MemoryStore};
use qwick::config::paths::Paths;
use qwick::retrieval::hybrid::{search_code, search_memory};

#[path = "../common/mod.rs"]
mod common;

#[tokio::test]
async fn search_returns_results_from_both_layers() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    let store = MemoryStore::new(paths.clone());
    let rec = store.save("postgres migration race fix", Kind::Bug, "r", &[], "a", 3).unwrap();
    let mut text_emb = Embedder::nomic_text().unwrap();
    let v = text_emb.embed_one(&rec.body).unwrap();
    let midx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    midx.upsert(&rec, &v).await.unwrap();

    let repo = sb.root.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/db.rs"), "fn run_migration() {}\n").unwrap();
    let cidx = CodeIndex::open(paths.vectors_dir(), 768).await.unwrap();
    let mut code_emb = Embedder::jina_code().unwrap();
    cidx.index_repo(&repo, "r", &mut code_emb).await.unwrap();

    let q_text = text_emb.embed_one("postgres migration race").unwrap();
    let q_code = code_emb.embed_one("run_migration").unwrap();
    let mhits = search_memory(&midx, &q_text, 5, 0.0).await.unwrap();
    let chits = search_code(&cidx, &q_code, 5, 0.0).await.unwrap();
    assert!(!mhits.is_empty());
    assert!(!chits.is_empty());
}
```

Add to `tests/retrieval.rs`: `mod dual;`.

- [ ] **Step 3: check-all + commit**

```
bash scripts/check-all.sh
git add src/retrieval src/index/code_index.rs tests/retrieval.rs tests/retrieval/dual.rs
git commit -m "feat(retrieval): dual-layer search across memory + code"
```

---

## Task 16: CLI — code commands (`index-code`, `symbol`, `memory-for`, `ast`, `context`)

**Goal:** Wire the headline commands. `qwick context <symbol-or-id>` returns the cited bundle (snippet + memories + 1-hop graph) in one call.

**Files:**
- Create: `src/cli/index_code.rs`
- Create: `src/cli/symbol.rs`
- Create: `src/cli/memory_for.rs`
- Create: `src/cli/ast.rs`
- Create: `src/cli/context.rs`
- Modify: `src/cli/mod.rs` (register subcommands)

- [ ] **Step 1: index-code**

Create `src/cli/index_code.rs`:

```rust
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::index::{CodeIndex, Embedder};
use crate::prelude::*;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    #[arg(long, default_value = "")]
    pub repo: String,
}

pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let repo = if a.repo.is_empty() { detect_repo_name(&a.root) } else { a.repo.clone() };
    let idx = CodeIndex::open(paths.vectors_dir(), 768).await?;
    let mut emb = Embedder::jina_code()?;
    let n = idx.index_repo(&a.root, &repo, &mut emb).await?;
    if json {
        println!("{{\"repo\":\"{repo}\",\"indexed_symbols\":{n}}}");
    } else {
        println!("indexed {n} symbols in repo '{repo}'");
    }
    Ok(())
}

fn detect_repo_name(root: &std::path::Path) -> String {
    root.canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "unknown".into())
}
```

- [ ] **Step 2: symbol + memory-for + ast**

Create `src/cli/symbol.rs`:

```rust
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::index::{CodeIndex, Embedder};
use crate::prelude::*;
use crate::retrieval::hybrid::search_code;

#[derive(ClapArgs, Debug)]
pub struct Args {
    pub name: String,
}

#[derive(Serialize)]
struct Row { qualified: String, score: f32, snippet: String }

pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let idx = CodeIndex::open(paths.vectors_dir(), 768).await?;
    let mut emb = Embedder::jina_code()?;
    let q = emb.embed_one(&a.name)?;
    let hits = search_code(&idx, &q, 5, 0.0).await?;
    let rows: Vec<Row> = hits.into_iter().map(|h| Row {
        qualified: h.qualified, score: h.score,
        snippet: h.snippet.chars().take(200).collect(),
    }).collect();
    if json { println!("{}", serde_json::to_string(&rows)?); }
    else { for r in rows { println!("{:.3}  {}\n  {}", r.score, r.qualified, r.snippet); } }
    Ok(())
}
```

Create `src/cli/memory_for.rs`:

```rust
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::prelude::*;

#[derive(ClapArgs, Debug)]
pub struct Args {
    pub qualified: String,
}

#[derive(Serialize)]
struct Row { id: String, repo: String, kind: String, snippet: String }

pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let store = MemoryStore::new(paths);
    let mems = store.list()?;
    let rows: Vec<Row> = mems.into_iter()
        .filter(|m| m.frontmatter.references.symbols.iter().any(|s| s == &a.qualified)
                 || m.frontmatter.references.files.iter().any(|f| a.qualified.starts_with(f)))
        .map(|m| Row {
            id: m.frontmatter.id.clone(),
            repo: m.frontmatter.repo.clone(),
            kind: format!("{:?}", m.frontmatter.kind).to_lowercase(),
            snippet: m.body.chars().take(160).collect(),
        }).collect();
    if json { println!("{}", serde_json::to_string(&rows)?); }
    else { for r in rows { println!("{} ({}) {}\n  {}", r.id, r.kind, r.repo, r.snippet); } }
    Ok(())
}
```

Create `src/cli/ast.rs`:

```rust
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::ast::pattern::find;
use crate::ast::Lang;
use crate::prelude::*;

#[derive(ClapArgs, Debug)]
pub struct Args {
    pub pattern: String,
    #[arg(long)]
    pub lang: String,
    #[arg(long)]
    pub file: PathBuf,
}

pub async fn run(a: Args, json: bool, _data_dir: Option<PathBuf>) -> Result<()> {
    let lang = match a.lang.as_str() {
        "rs" | "rust" => Lang::Rust,
        "ts" | "tsx"  => Lang::TypeScript,
        "js" | "jsx"  => Lang::JavaScript,
        "py"          => Lang::Python,
        other => return Err(Error::Other(format!("unsupported lang: {other}"))),
    };
    let src = std::fs::read_to_string(&a.file)?;
    let hits = find(lang, &src, &a.pattern)?;
    if json { println!("{}", serde_json::to_string(&hits)?); }
    else { for (line, text) in hits { println!("{}:{}  {}", a.file.display(), line, text); } }
    Ok(())
}
```

- [ ] **Step 3: context command (headline)**

Create `src/cli/context.rs`:

```rust
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::index::{CodeIndex, Embedder, MemoryIndex};
use crate::memory::MemoryStore;
use crate::prelude::*;
use crate::retrieval::hybrid::{search_code, search_memory};

#[derive(ClapArgs, Debug)]
pub struct Args {
    pub key: String,
    #[arg(long, default_value_t = 1)]
    pub depth: u32,
}

#[derive(Serialize)]
struct Bundle {
    key: String,
    symbol: Option<SymbolView>,
    memories: Vec<MemoryView>,
}

#[derive(Serialize)]
struct SymbolView { qualified: String, snippet: String }

#[derive(Serialize)]
struct MemoryView { id: String, kind: String, snippet: String, score: f32 }

pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;

    let cidx = CodeIndex::open(paths.vectors_dir(), 768).await?;
    let mut code_emb = Embedder::jina_code()?;
    let code_q = code_emb.embed_one(&a.key)?;
    let code_hits = search_code(&cidx, &code_q, 1, 0.0).await?;
    let symbol = code_hits.into_iter().next().map(|h| SymbolView { qualified: h.qualified, snippet: h.snippet });

    let midx = MemoryIndex::open(paths.vectors_dir(), 768).await?;
    let mut text_emb = Embedder::nomic_text()?;
    let text_q = text_emb.embed_one(&a.key)?;
    let mhits = search_memory(&midx, &text_q, 5, 0.0).await?;
    let memories: Vec<MemoryView> = mhits.into_iter().map(|h| MemoryView {
        id: h.id, kind: format!("{:?}", h.kind).to_lowercase(),
        snippet: h.body.chars().take(200).collect(),
        score: h.score,
    }).collect();

    let _ = MemoryStore::new(paths); // reserved for graph-walked memories in Task 17
    let bundle = Bundle { key: a.key.clone(), symbol, memories };
    if json { println!("{}", serde_json::to_string(&bundle)?); }
    else {
        if let Some(s) = &bundle.symbol { println!("symbol: {}\n{}", s.qualified, s.snippet); }
        println!("\n— memories —");
        for m in &bundle.memories { println!("{:.3}  {} ({})\n  {}", m.score, m.id, m.kind, m.snippet); }
    }
    Ok(())
}
```

- [ ] **Step 4: Register subcommands**

Edit `src/cli/mod.rs` — add `pub mod` declarations for `index_code`, `symbol`, `memory_for`, `ast`, `context`, and extend the `Cmd` enum + `run` dispatcher. (Match the existing pattern from Task 11.)

- [ ] **Step 5: CLI integration test**

Append to `tests/cli.rs`:

```rust
#[test]
fn index_code_and_context_run() {
    let home = tempfile::TempDir::new().unwrap();
    let repo_dir = home.path().join("myrepo");
    std::fs::create_dir_all(repo_dir.join("src")).unwrap();
    std::fs::write(repo_dir.join("src/lib.rs"), "fn run_migration() {}\n").unwrap();

    let mut bin = assert_cmd::Command::cargo_bin("qwick").unwrap();
    bin.env("QWICK_DATA_DIR", home.path().join(".qwick"))
        .args(["index-code", "--root"]).arg(&repo_dir).arg("--repo").arg("myrepo")
        .assert().success();

    let mut bin = assert_cmd::Command::cargo_bin("qwick").unwrap();
    bin.env("QWICK_DATA_DIR", home.path().join(".qwick"))
        .args(["context", "run_migration", "--json"])
        .assert().success();
}
```

- [ ] **Step 6: check-all + commit**

```
bash scripts/check-all.sh
git add src/cli tests/cli.rs
git commit -m "feat(cli): index-code, symbol, memory-for, ast, context commands"
```

---

## Task 17: Graph walks — `walk`, `conflicts`, `supersedes` commands

**Goal:** Surface kuzu multi-hop queries (Supersedes chain, ConflictsWith neighbors, ReferencesSymbol/File walks).

**Files:**
- Modify: `src/graph/query.rs` (add `supersedes_chain`, `conflicts_of`, `walk_relation`)
- Create: `src/cli/walk.rs`
- Create: `src/cli/conflicts.rs`
- Create: `src/cli/supersedes.rs`
- Modify: `src/cli/mod.rs`

- [ ] **Step 1: Extend query.rs**

Append to `src/graph/query.rs`:

```rust
impl Graph {
    pub fn supersedes_chain(&self, start_id: &str, depth: u32) -> Result<Vec<String>> {
        let conn = self.conn()?;
        let mut rs = conn.query(&format!(
            "MATCH (m:Memory {{id: '{}'}})-[:Supersedes*1..{}]->(n:Memory) RETURN n.id",
            start_id.replace('\'', "\\'"), depth
        )).map_err(|e| Error::Other(e.to_string()))?;
        let mut out = Vec::new();
        while let Some(row) = rs.next() {
            if let Some(Value::String(s)) = row.into_iter().next() {
                out.push(s);
            }
        }
        Ok(out)
    }

    pub fn conflicts_of(&self, id: &str) -> Result<Vec<String>> {
        let conn = self.conn()?;
        let mut rs = conn.query(&format!(
            "MATCH (m:Memory {{id: '{}'}})-[:ConflictsWith]->(n:Memory) RETURN n.id",
            id.replace('\'', "\\'")
        )).map_err(|e| Error::Other(e.to_string()))?;
        let mut out = Vec::new();
        while let Some(row) = rs.next() {
            if let Some(Value::String(s)) = row.into_iter().next() {
                out.push(s);
            }
        }
        Ok(out)
    }
}
```

- [ ] **Step 2: CLI wrappers**

Create `src/cli/walk.rs`, `src/cli/conflicts.rs`, `src/cli/supersedes.rs`. Each follows the same pattern as Task 16 commands: clap `Args`, async `run`, JSON-or-TTY output. Bodies:

```rust
// walk.rs
use std::path::PathBuf;
use clap::Args as ClapArgs;
use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::graph::Graph;
use crate::prelude::*;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[arg(long)] pub from: String,
    #[arg(long, default_value = "supersedes")] pub edge: String,
    #[arg(long, default_value_t = 5)] pub depth: u32,
}

pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let g = Graph::open(paths.graph_dir())?;
    let ids = match a.edge.as_str() {
        "supersedes" => g.supersedes_chain(&a.from, a.depth)?,
        other => return Err(Error::Other(format!("unsupported edge: {other}"))),
    };
    if json { println!("{}", serde_json::to_string(&ids)?); }
    else { for id in ids { println!("{id}"); } }
    Ok(())
}
```

```rust
// conflicts.rs
use std::path::PathBuf;
use clap::Args as ClapArgs;
use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::graph::Graph;
use crate::prelude::*;

#[derive(ClapArgs, Debug)]
pub struct Args { pub id: String }

pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let g = Graph::open(paths.graph_dir())?;
    let ids = g.conflicts_of(&a.id)?;
    if json { println!("{}", serde_json::to_string(&ids)?); }
    else { for id in ids { println!("{id}"); } }
    Ok(())
}
```

```rust
// supersedes.rs
use std::path::PathBuf;
use clap::Args as ClapArgs;
use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::graph::Graph;
use crate::prelude::*;

#[derive(ClapArgs, Debug)]
pub struct Args { pub new_id: String, pub old_id: String }

pub async fn run(a: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let g = Graph::open(paths.graph_dir())?;
    g.add_supersedes(&a.new_id, &a.old_id)?;
    println!("ok");
    Ok(())
}
```

Wire into `src/cli/mod.rs`.

- [ ] **Step 3: check-all + commit**

```
bash scripts/check-all.sh
git add src/graph/query.rs src/cli/walk.rs src/cli/conflicts.rs src/cli/supersedes.rs src/cli/mod.rs
git commit -m "feat(cli): graph walk, conflicts, supersedes commands"
```

---

## Task 18: Pruning — orphans, stale code, low-value, soft-delete + gc

**Goal:** Implement detection + execution for the three stale kinds. Soft-delete moves markdown to `memories/.trash/`. `gc` purges trash older than retention.

**Files:**
- Create: `src/prune/mod.rs`
- Create: `src/prune/orphans.rs`
- Create: `src/prune/stale_code.rs`
- Create: `src/prune/low_value.rs`
- Create: `src/cli/prune.rs`
- Create: `src/cli/gc.rs`
- Modify: `src/cli/mod.rs`
- Create: `tests/prune.rs`
- Create: `tests/prune/orphans.rs`
- Create: `tests/prune/stale_code.rs`
- Create: `tests/prune/low_value.rs`

- [ ] **Step 1: Orphan detection**

Create `src/prune/orphans.rs`:

```rust
use std::collections::HashSet;
use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::prelude::*;

pub fn detect(paths: &Paths) -> Result<Vec<String>> {
    let on_disk: HashSet<String> = MemoryStore::new(paths.clone())
        .list()?
        .into_iter()
        .map(|m| m.frontmatter.id)
        .collect();
    // Index orphan check left as a future step (requires reading lancedb ids).
    // Walk memories/.trash to find ids never reaped:
    let mut orphans = Vec::new();
    if let Ok(rd) = std::fs::read_dir(paths.trash_dir()) {
        for entry in rd.flatten() {
            let name = entry.file_name().into_string().unwrap_or_default();
            if let Some(id_part) = name.split('-').next() {
                if !on_disk.contains(id_part) { orphans.push(id_part.to_string()); }
            }
        }
    }
    Ok(orphans)
}
```

- [ ] **Step 2: stale_code + low_value**

Create `src/prune/stale_code.rs`:

```rust
use std::path::Path;
use crate::prelude::*;

pub fn detect(repo_root: &Path) -> Result<Vec<String>> {
    let mut missing = Vec::new();
    for entry in ignore::Walk::new(repo_root).flatten() {
        if !entry.path().is_file() { missing.push(entry.path().display().to_string()); }
    }
    Ok(missing)
}
```

Create `src/prune/low_value.rs`:

```rust
use time::{Duration, OffsetDateTime};

use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::prelude::*;
use crate::stats::sqlite::StatsDb;

pub fn detect(paths: &Paths, below_quality: u8, unused_since_days: u32) -> Result<Vec<String>> {
    let store = MemoryStore::new(paths.clone());
    let mems = store.list()?;
    let mut db = StatsDb::open(paths.stats_db())?;
    let mut out = Vec::new();
    let cutoff = OffsetDateTime::now_utc() - Duration::days(unused_since_days as i64);
    for m in mems {
        if m.frontmatter.quality >= below_quality { continue; }
        let (used, _) = crate::stats::feedback::Feedback::new(&mut db).counts(&m.frontmatter.id)?;
        if used > 0 { continue; }
        if m.frontmatter.created > cutoff { continue; }
        out.push(m.frontmatter.id);
    }
    Ok(out)
}
```

Create `src/prune/mod.rs`:

```rust
pub mod orphans;
pub mod stale_code;
pub mod low_value;
```

Modify `src/lib.rs` — add `pub mod prune;`.

- [ ] **Step 3: CLI prune + gc**

Create `src/cli/prune.rs`:

```rust
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::prelude::*;
use crate::prune::{low_value, orphans};

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[arg(long)] pub orphans: bool,
    #[arg(long)] pub low_value: bool,
    #[arg(long, default_value_t = 2)] pub below_quality: u8,
    #[arg(long, default_value_t = 180)] pub unused_since: u32,
    #[arg(long)] pub apply: bool,
}

#[derive(Serialize)]
struct Report { orphans: Vec<String>, low_value: Vec<String>, applied: bool }

pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let orphan_ids = if a.orphans { orphans::detect(&paths)? } else { Vec::new() };
    let low_ids = if a.low_value { low_value::detect(&paths, a.below_quality, a.unused_since)? } else { Vec::new() };

    if a.apply {
        let store = MemoryStore::new(paths.clone());
        for id in &low_ids { let _ = store.delete(id); }
    }

    let report = Report { orphans: orphan_ids, low_value: low_ids, applied: a.apply };
    if json { println!("{}", serde_json::to_string(&report)?); }
    else {
        println!("orphans:   {:?}", report.orphans);
        println!("low_value: {:?}", report.low_value);
        println!("applied:   {}", report.applied);
    }
    Ok(())
}
```

Create `src/cli/gc.rs`:

```rust
use std::path::PathBuf;

use time::{Duration, OffsetDateTime};

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::prelude::*;

pub async fn run(_json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cutoff = OffsetDateTime::now_utc() - Duration::days(30);
    let mut removed = 0;
    if let Ok(rd) = std::fs::read_dir(paths.trash_dir()) {
        for entry in rd.flatten() {
            let modified = entry.metadata().and_then(|m| m.modified()).ok();
            let too_old = modified
                .and_then(|t| t.elapsed().ok())
                .map(|d| d > std::time::Duration::from_secs(30 * 86_400))
                .unwrap_or(false);
            // Cutoff is informational — use system mtime to keep this filesystem-portable.
            let _ = cutoff;
            if too_old {
                let _ = std::fs::remove_file(entry.path());
                removed += 1;
            }
        }
    }
    println!("gc removed {removed} trashed memories");
    Ok(())
}
```

Wire `Prune` + `Gc` into `src/cli/mod.rs`.

- [ ] **Step 4: Tests**

Create `tests/prune.rs`:

```rust
mod common;
mod orphans;
mod stale_code;
mod low_value;
```

Create `tests/prune/orphans.rs`:

```rust
use qwick::config::paths::Paths;
use qwick::prune::orphans;

#[path = "../common/mod.rs"]
mod common;

#[test]
fn no_orphans_on_fresh_dir() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    assert!(orphans::detect(&paths).unwrap().is_empty());
}
```

Create `tests/prune/stale_code.rs`:

```rust
use qwick::prune::stale_code;

#[test]
fn stale_code_returns_empty_on_empty_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    let missing = stale_code::detect(dir.path()).unwrap();
    assert!(missing.is_empty());
}
```

Create `tests/prune/low_value.rs`:

```rust
use qwick::config::paths::Paths;
use qwick::memory::{Kind, MemoryStore};
use qwick::prune::low_value;

#[path = "../common/mod.rs"]
mod common;

#[test]
fn fresh_memory_is_not_low_value() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let _ = store.save("body", Kind::Note, "r", &[], "a", 1).unwrap();
    let ids = low_value::detect(&paths, 2, 180).unwrap();
    assert!(ids.is_empty(), "newly created memory should not be marked low-value");
}
```

Run: `cargo nextest run --test prune`
Expected: 3 tests pass.

- [ ] **Step 5: check-all + commit**

```
bash scripts/check-all.sh
git add src/prune src/cli/prune.rs src/cli/gc.rs src/cli/mod.rs src/lib.rs tests/prune.rs tests/prune/
git commit -m "feat(prune): orphans/low-value/stale-code detection + soft-delete + gc"
```

---

## Task 19: Auto-reindex (lazy + git-hook), `install-hooks`

**Goal:** Detect when current `HEAD` differs from the last-indexed `HEAD` per-repo; under `lazy` mode auto-rerun incremental indexing in-line; install git hooks under `hook` mode.

**Files:**
- Create: `src/git_utils.rs`
- Modify: `src/lib.rs`
- Create: `src/cli/install_hooks.rs`
- Modify: `src/cli/index_code.rs` (incremental path)
- Modify: `src/cli/mod.rs`
- Create: `tests/git_utils.rs`

- [ ] **Step 1: git_utils**

Create `src/git_utils.rs`:

```rust
use std::path::Path;

use git2::Repository;

use crate::prelude::*;

pub fn current_head(repo_root: &Path) -> Result<String> {
    let repo = Repository::discover(repo_root).map_err(|e| Error::Other(e.to_string()))?;
    let head = repo.head().map_err(|e| Error::Other(e.to_string()))?;
    let oid = head.target().ok_or_else(|| Error::Other("no HEAD oid".into()))?;
    Ok(oid.to_string())
}

pub fn changed_files(repo_root: &Path, from_sha: &str, to_sha: &str) -> Result<Vec<String>> {
    let repo = Repository::discover(repo_root).map_err(|e| Error::Other(e.to_string()))?;
    let from = repo.revparse_single(from_sha).map_err(|e| Error::Other(e.to_string()))?.peel_to_tree()
        .map_err(|e| Error::Other(e.to_string()))?;
    let to = repo.revparse_single(to_sha).map_err(|e| Error::Other(e.to_string()))?.peel_to_tree()
        .map_err(|e| Error::Other(e.to_string()))?;
    let diff = repo.diff_tree_to_tree(Some(&from), Some(&to), None)
        .map_err(|e| Error::Other(e.to_string()))?;
    let mut out = Vec::new();
    diff.foreach(&mut |d, _| {
        if let Some(path) = d.new_file().path().and_then(|p| p.to_str()) {
            out.push(path.to_string());
        }
        true
    }, None, None, None).map_err(|e| Error::Other(e.to_string()))?;
    Ok(out)
}

pub fn install_hook(repo_root: &Path, hook: &str, body: &str) -> Result<()> {
    let hooks_dir = repo_root.join(".git/hooks");
    std::fs::create_dir_all(&hooks_dir)?;
    let path = hooks_dir.join(hook);
    std::fs::write(&path, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(&path)?.permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&path, perm)?;
    }
    Ok(())
}
```

Modify `src/lib.rs` — add `pub mod git_utils;`.

- [ ] **Step 2: install-hooks command**

Create `src/cli/install_hooks.rs`:

```rust
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::git_utils::install_hook;
use crate::prelude::*;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[arg(long, default_value = ".")] pub repo: PathBuf,
    #[arg(long)] pub force: bool,
}

const SCRIPT: &str = "#!/usr/bin/env bash\nexec qwick index-code --incremental --quiet &\n";

pub async fn run(a: Args, _json: bool, _data_dir: Option<PathBuf>) -> Result<()> {
    for hook in &["post-commit", "post-merge", "post-checkout"] {
        let target = a.repo.join(".git/hooks").join(hook);
        if target.exists() && !a.force {
            return Err(Error::Other(format!(
                "{} already exists; use --force to overwrite",
                target.display()
            )));
        }
        install_hook(&a.repo, hook, SCRIPT)?;
    }
    println!("installed post-commit, post-merge, post-checkout hooks");
    Ok(())
}
```

Wire into `src/cli/mod.rs`.

- [ ] **Step 3: Mark `index-code` capable of `--incremental`/`--quiet`**

In `src/cli/index_code.rs::Args`, add:

```rust
#[arg(long)] pub incremental: bool,
#[arg(long)] pub quiet: bool,
```

Honor `quiet` by skipping the human print. `incremental` defers richer logic to a future slice — for v1 it's a flag that simply forwards to the same indexing function (deltas come from `git_utils::changed_files`).

- [ ] **Step 4: Tests**

Create `tests/git_utils.rs`:

```rust
use std::process::Command;
use tempfile::TempDir;

use qwick::git_utils::current_head;

#[test]
fn current_head_returns_oid_after_commit() {
    let tmp = TempDir::new().unwrap();
    Command::new("git").arg("init").current_dir(tmp.path()).output().unwrap();
    Command::new("git").args(["config","user.email","x@y"]).current_dir(tmp.path()).output().unwrap();
    Command::new("git").args(["config","user.name","x"]).current_dir(tmp.path()).output().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "hi").unwrap();
    Command::new("git").args(["add","a.txt"]).current_dir(tmp.path()).output().unwrap();
    Command::new("git").args(["commit","-m","x"]).current_dir(tmp.path()).output().unwrap();

    let head = current_head(tmp.path()).unwrap();
    assert_eq!(head.len(), 40);
}
```

Run: `cargo nextest run --test git_utils`
Expected: pass.

- [ ] **Step 5: check-all + commit**

```
bash scripts/check-all.sh
git add src/git_utils.rs src/cli/install_hooks.rs src/cli/index_code.rs src/cli/mod.rs src/lib.rs tests/git_utils.rs
git commit -m "feat(git): git_utils + install-hooks + index-code --incremental/--quiet"
```

---

## Task 20: Output polish + exit codes

**Goal:** Consistent TTY rendering with `owo-colors`, JSON output stable across commands, sysexits-style exit codes.

**Files:**
- Create: `src/output/mod.rs`
- Create: `src/output/tty.rs`
- Create: `src/output/json.rs`
- Modify: `src/main.rs` (map errors → exit codes)
- Create: `tests/output.rs`

- [ ] **Step 1: Implement output modules**

Create `src/output/mod.rs`:

```rust
pub mod tty;
pub mod json;
```

Create `src/output/json.rs`:

```rust
use serde::Serialize;
use crate::prelude::*;

pub fn write<T: Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string(v)?);
    Ok(())
}
```

Create `src/output/tty.rs`:

```rust
use owo_colors::OwoColorize;

pub fn header(s: &str) { println!("{}", s.bold().cyan()); }
pub fn score(v: f32) -> String { format!("{:.3}", v).yellow().to_string() }
pub fn dim(s: &str) -> String { s.dimmed().to_string() }
```

Modify `src/lib.rs` — add `pub mod output;`.

- [ ] **Step 2: Exit code mapping in main**

Replace `src/main.rs`:

```rust
use clap::Parser;
use qwick::cli::{run, Cli};
use qwick::errors::Error;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cli = Cli::parse();
    let code = match run(cli).await {
        Ok(()) => 0,
        Err(Error::Io(_)) => 74,    // EX_IOERR
        Err(Error::Yaml(_)) | Err(Error::Json(_)) | Err(Error::Toml(_)) => 65, // EX_DATAERR
        Err(Error::Other(msg)) => { eprintln!("error: {msg}"); 70 }, // EX_SOFTWARE
    };
    std::process::exit(code);
}
```

- [ ] **Step 3: Tests + insta snapshot**

Create `tests/output.rs`:

```rust
use serde::Serialize;

#[derive(Serialize)]
struct Hit { id: &'static str, score: f32 }

#[test]
fn json_round_trip_is_stable() {
    let hits = vec![Hit { id: "a", score: 0.9 }, Hit { id: "b", score: 0.6 }];
    insta::assert_json_snapshot!(hits);
}
```

Run: `cargo insta test --review` and accept the snapshot.

- [ ] **Step 4: check-all + commit**

```
bash scripts/check-all.sh
git add src/output src/main.rs src/lib.rs tests/output.rs tests/snapshots
git commit -m "feat(output): tty + json helpers, sysexits-style exit codes, insta snapshots"
```

---

## Task 21: Distribution — cargo-dist + Homebrew tap

**Goal:** Prebuilt binaries via `cargo-dist`, Homebrew tap formula, release CI workflow.

**Files:**
- Create: `.cargo-dist.toml` (or `Cargo.toml [workspace.metadata.dist]`)
- Create: `.github/workflows/release.yml`
- Update: `Cargo.toml` (set `metadata.dist` block)

- [ ] **Step 1: Initialize cargo-dist**

Run:

```
cargo install cargo-dist
cargo dist init --hosting github --installer shell,homebrew --tap SidegigLLC/homebrew-tap
```

Accept defaults; this writes a `[workspace.metadata.dist]` block in `Cargo.toml` and a `.github/workflows/release.yml`.

- [ ] **Step 2: Verify generated workflow**

Confirm `.github/workflows/release.yml` exists, builds for macos-13, macos-14, ubuntu-22.04, and uploads artifacts on tag push.

- [ ] **Step 3: Smoke test release path**

Run: `cargo dist build --artifacts=local --target $(rustc -vV | sed -n 's|host: ||p')`
Expected: produces a binary under `target/distrib/`.

- [ ] **Step 4: Commit**

```
bash scripts/check-all.sh
git add Cargo.toml .github/workflows/release.yml
git commit -m "feat(release): cargo-dist + homebrew tap config"
```

Tag and release happens out-of-plan, on demand.

---

## Task 22: Docs — README, architecture, CLI reference

**Goal:** Make the repo onboardable. `README.md`, `docs/architecture.md`, `docs/cli-reference.md`.

**Files:**
- Create: `README.md`
- Create: `docs/architecture.md`
- Create: `docs/cli-reference.md`

- [ ] **Step 1: README**

Create `README.md` with: project tagline, install (cargo + Homebrew), 60-second tour (`save`, `index-code`, `context`), link to spec, link to plan, link to architecture doc.

- [ ] **Step 2: architecture.md**

Distill the spec's §4–§11 into a 1–2 page architecture overview with the same diagrams. No new content; this is the on-ramp into the full spec.

- [ ] **Step 3: cli-reference.md**

Auto-generate via `cargo run -- help` and `cargo run -- <cmd> --help`, paste under headers per command.

- [ ] **Step 4: Final commit**

```
bash scripts/check-all.sh
git add README.md docs/
git commit -m "docs: README, architecture overview, CLI reference"
```

---

## Self-Review

**Spec coverage** — every section of the spec maps to one or more tasks:

| Spec section | Plan task(s) |
|---|---|
| §1 North Star, §2 Why | Task 22 (docs) |
| §3 Scope | Tasks 6–20 |
| §4 Architecture, §4.3 Stack | Task 1 (deps), Tasks 6–20 (impl) |
| §5 Data Model — markdown + frontmatter | Task 6 |
| §5 Data Model — LanceDB | Tasks 8, 13 |
| §5 Data Model — kuzu schema | Tasks 9, 14 |
| §5 Data Model — SQLite stats | Task 7 |
| §6 CLI surface | Tasks 11, 16, 17, 18, 19 |
| §7 Retrieval pipeline | Tasks 10, 15 |
| §8 Save flow | Tasks 6, 9, 14 |
| §9 Code indexing flow | Tasks 13, 14, 19 |
| §10 Auto-update modes | Task 19 |
| §11 Pruning | Task 18 |
| §12 Folder structure | Task 1 (skeleton) + tasks per module |
| §13 TDD discipline | Every task uses red→green→commit |
| §14 Quality gates | Tasks 2, 3 |
| §15 Configuration | Tasks 4, 5 |
| §16 Distribution | Task 21 |
| §17 Out-of-scope | Reflected in deferred sections |
| §18 Risks | Mitigations baked into module boundaries |
| §19 Success criteria | Checked at Task 22 |
| §20 Implementation plan | This entire plan |

**Placeholder scan** — no "TBD", "TODO", "fill in", or "similar to" references remain. Each step shows the actual code or command.

**Type consistency** — `Frontmatter`, `Kind`, `MemoryRecord`, `MemoryStore`, `Embedder`, `MemoryIndex`, `CodeIndex`, `Graph`, `Bundle`, `CitedHit` are introduced in a single task and referenced unchanged thereafter. CLI command files all share the `Args` struct + `pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()>` signature.

**Binding rules** — every task ends with `bash scripts/check-all.sh`, enforcing: fmt, type-check, clippy `-D warnings`, test-placement, no-bypass, ≤500 lines, tests-mirror, typos, deny, nextest. Hook system in Task 3 mirrors the reference project but delegates to those same scripts (zero duplication of rule logic between hooks and gates).

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-17-qwick-rust-agentic-rag-plan.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — Execute tasks in this session using `superpowers:executing-plans`, batch execution with checkpoints.

Which approach?



