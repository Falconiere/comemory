# Issue #165 Domain Guardrails Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish #165's checked, behavior-preserving domain-migration contract without moving product capabilities.

**Architecture:** A checked-in markdown inventory and JSON policy define ownership, compatibility, staged paths, and exact temporary edges. A project-owned, AST-aware checker validates them with real temporary source trees. CI and the local hook invoke that checker; the vendored guardrails engine is not changed.

**Tech Stack:** Bash 3.2-compatible scripts, `jq`, installed `ast-grep`, Git, Rust source tree, existing guardrails, lefthook.

**Spec:** `docs/designs/2026-09-17-domain-first-migration.md`

## Global Constraints

- Preserve all CLI, HTTP, wire, ranking, cloud, markdown, SQLite, and released-migration behavior; #165 moves no capability module.
- Keep one crate; add no dependency, workspace, database change, or mass snapshot refresh.
- Do not modify `scripts/guardrails/`; retain the `store` SQL/`rusqlite` chokepoint and baseline zero.
- Each temporary edge is an exact source file, canonical target module, edge class, and one removal issue. Wildcards and stale entries fail.
- Keep descriptive names, sibling Rust module files (never `mod.rs`), the 300/100-line ceilings, and real-file tests.

---

### Task 1: Record the machine-checked staged ownership inventory

**Files:**
- Create: `docs/designs/2026-09-17-domain-first-migration-inventory.md`
- Create: `scripts/architecture-policy.json`
- Create: `scripts/test-architecture-policy.sh`

**Interfaces:**
- Produces JSON keys `version`, `staged_top_level_dirs`, `staged_root_modules`, `domains`, `legacy_modules`, `owner_dependencies`, `legacy_edges`, `store_callbacks`, and `passive_store_models`.
- Produces one inventory row per production Rust file: `path | public path | test bridge | assets | owner | target | issue`.
- Task 2 consumes the policy and inventory as its only contract inputs.

- [ ] **Step 1: Write the failing policy/inventory harness**

Create a temporary-copy test that expects policy validation to reject a missing
production-file row, a duplicate inventory row, an unknown removal issue, a
missing exact target, and an allowlisted target absent from its source file.

```bash
assert_fails "$CHECK" --policy "$BAD_POLICY" --inventory "$INVENTORY"
assert_fails "$CHECK" --policy "$POLICY" --inventory "$MISSING_ROW"
assert_fails "$CHECK" --policy "$POLICY" --inventory "$DUPLICATE_ROW"
```

- [ ] **Step 2: Verify the red failure**

Run: `bash scripts/test-architecture-policy.sh`

Expected: it fails because the policy and inventory do not exist.

- [ ] **Step 3: Create the inventory and exact policy**

Use the following canonical file list, excluding only colocated test trees:

```bash
rg --files src -g '*.rs' -g '!src/**/tests/**' | sort
```

Assign every result exactly once to a domain, delivery, shared, or
infrastructure owner. Record the 50 API cores with the spec's exact owners;
each `#[path]` bridge and `include_str!`, `include_bytes!`, or `include!` asset
uses a resolved repository-relative path. List each `src/lib.rs` public path as
`preserve`, `crate-root-alias`, or `breaking`; a breaking row names its release
version and release-note target. The policy lists the ten domain names, all
staged root paths, setup's actual runtime dependencies, every existing
API/retrieval delivery edge, every store callback, and exact passive model
imports. Every legacy edge names only one of #166, #167, #169, #170, or #177.
Set `staged_top_level_dirs` exactly to
`api ast capture cli cloud config consolidate document domains eval graph
memory output prune retrieval serve source stats store sync upgrade utilities`.
Set `staged_root_modules` exactly to
`api ast capture cli cloud config consolidate document embed errors eval fetch
git_utils graph http_error index lib main memory output prelude prune retrieval
serve simhash source stats store sync test_common upgrade`. Set `domains` to
`memories code documents graph retrieval learning sync capture maintenance
integrations`.

- [ ] **Step 4: Verify green**

Run: `bash scripts/test-architecture-policy.sh`

Expected: it exits 0 after validating every production path, inventory row,
asset/bridge, and policy edge.

- [ ] **Step 5: Commit**

```bash
git add docs/designs/2026-09-17-domain-first-migration-inventory.md scripts/architecture-policy.json scripts/test-architecture-policy.sh
git commit -m "docs: inventory domain migration ownership"
```

### Task 2: Implement the architecture policy checker test-first

**Files:**
- Create: `scripts/architecture-check.sh`
- Create: `scripts/test-architecture-check.sh`
- Modify: `scripts/architecture-policy.json`

**Interfaces:**
- Consumes Task 1 policy/inventory inputs.
- Provides `bash scripts/architecture-check.sh [--root <path>] [--file <path>...] [--policy <path>] [--inventory <path>]`.
- Returns 0 valid, 1 violation, and 3 bad argument, malformed policy, missing tool, or missing input.
- Task 3 calls it from CI and lefthook.

- [ ] **Step 1: Write real-tree fixtures before the checker**

Create temporary source trees and assert the following, in repository and
`--file` modes when applicable:

```bash
assert_ok "$CHECK" --root "$ALLOWED"
assert_fails "$CHECK" --root "$BAD_ROOT_MODULE"
assert_fails "$CHECK" --root "$BAD_DOMAIN_IMPORT"
assert_ok "$CHECK" --root "$ALLOWLISTED_LEGACY"
assert_fails "$CHECK" --root "$STALE_ALLOWLIST"
assert_fails "$CHECK" --root "$STORE_CALLBACK"
assert_ok "$CHECK" --root "$PASSIVE_MODEL"
```

Cover an unapproved domain, declaration-only and README-only scaffolds, an
invalid nested domain path, direct/grouped/multiline/relative/aliased forbidden
imports, comments and literals, and duplicate/malformed/unknown-issue policy
records.

- [ ] **Step 2: Verify the red failure**

Run: `bash scripts/test-architecture-check.sh`

Expected: it fails because `scripts/architecture-check.sh` is absent.

- [ ] **Step 3: Implement the minimal checker**

Use `set -eu`, `jq -e`, and `ast-grep` to parse production Rust only. Normalize
the required import forms; reject forbidden aliases/re-exports; validate all
policy records before scanning; sort diagnostics by source then target; group
all targets for one source; keep valid allowlisted edges silent. A store service
means a domain-owned lifecycle, indexing, graph, retrieval, or maintenance
function/method/constructor; exact passive structs/enums may import. Do not
edit `scripts/guardrails/`.

```bash
case "$status" in
  0) exit 0 ;;
  1) exit 1 ;;
  *) exit 3 ;;
esac
```

- [ ] **Step 4: Verify green**

Run: `bash scripts/test-architecture-check.sh`

Expected: every allowed fixture exits 0 and every invalid fixture reports the
specified violation in repository and scoped modes.

- [ ] **Step 5: Commit**

```bash
git add scripts/architecture-check.sh scripts/test-architecture-check.sh scripts/architecture-policy.json
git commit -m "feat: add domain architecture check"
```

### Task 3: Wire staged structure policy into project gates

**Files:**
- Modify: `guardrails.config.json`
- Modify: `scripts/check-all.sh`
- Modify: `lefthook.yml`
- Modify: `scripts/test-architecture-check.sh`

**Interfaces:**
- `check-all` runs `architecture-check` after `guardrails-check` and before `store-chokepoint-check`.
- Pre-commit separately runs `bash scripts/architecture-check.sh --file {staged_files}` for `*.rs` files.

- [ ] **Step 1: Add failing gate and hook assertions**

Extend the real fixture test to fail when either required `check-all` ordering
or the scoped lefthook command is omitted from a temporary copy.

```bash
assert_fails "$TEST_GATE_WIRING" "$CHECK_ALL_WITHOUT_GATE" "$HOOK"
assert_fails "$TEST_GATE_WIRING" "$CHECK_ALL" "$HOOK_WITHOUT_SCOPED_CHECK"
```

- [ ] **Step 2: Verify the red failure**

Run: `bash scripts/test-architecture-check.sh`

Expected: it fails because the checker is not wired into either gate.

- [ ] **Step 3: Add configuration and wiring**

Set `src.topLevel` to `api`, `ast`, `capture`, `cli`, `cloud`, `config`,
`consolidate`, `document`, `domains`, `eval`, `graph`, `memory`, `output`,
`prune`, `retrieval`, `serve`, `source`, `stats`, `store`, `sync`, `upgrade`,
and `utilities`. Add exact `domains` and `domains/*` nested rules before the
`*` fallback but do not add an empty domain directory or README requirement.
Preserve legitimate legacy paths until their owning child issue removes them.
Add the named check-all gate and scoped pre-commit command without changing the
vendored guardrails runner.

- [ ] **Step 4: Verify green**

Run: `bash scripts/test-architecture-check.sh && bash scripts/guardrails/run.sh`

Expected: both commands exit 0, while temporary omissions prove the required
gate or hook would fail.

- [ ] **Step 5: Commit**

```bash
git add guardrails.config.json scripts/check-all.sh lefthook.yml scripts/test-architecture-check.sh
git commit -m "chore: enforce staged domain structure"
```

### Task 4: Reconcile repository guidance with the pinned conventions

**Files:**
- Modify: `AGENTS.md`
- Modify: `docs/designs/2026-09-17-domain-first-migration.md`
- Modify: `scripts/test-architecture-policy.sh`

**Interfaces:**
- Documents the same policy implemented by Tasks 1–3.
- Preserves stronger local deviations while updating only stale D3/D4/D5 text.

- [ ] **Step 1: Add a failing guidance-consistency assertion**

Require D3 to name `barrelNames: ["mod.rs"]`, D4 to describe the actual
`src.nested` additions, D5 to describe actual README policy, and the design to
link the inventory and #162/#163 baseline constraints.

```bash
rg -F 'D3 — `barrelNames: ["mod.rs"]`' AGENTS.md
rg -F '2026-09-17-domain-first-migration-inventory.md' docs/designs/2026-09-17-domain-first-migration.md
```

- [ ] **Step 2: Verify the red failure**

Run: `bash scripts/test-architecture-policy.sh`

Expected: it fails until D3/D4/D5 agree with the actual policy.

- [ ] **Step 3: Reconcile AGENTS and design evidence**

Compare D3/D4/D5 with toolu-conventions commit
`abd091eb50e91e18fdc8e07acf5b4e91c0a8addf`. Correct stale wording, retain the
local deviations, and state `serve` is the HTTP adapter and `store` the SQLite
exception. Treat #162 common-dir hook/worktree-label behavior and #163 setup's
seven stable IDs, offline no-DB probe, report-only steps, and exit-69 ordering
as preserved baseline behavior.

- [ ] **Step 4: Verify green**

Run: `bash scripts/test-architecture-policy.sh`

Expected: it exits 0 with guidance, policy, and inventory agreeing.

- [ ] **Step 5: Commit**

```bash
git add AGENTS.md docs/designs/2026-09-17-domain-first-migration.md scripts/test-architecture-policy.sh
git commit -m "docs: align domain migration guidance"
```

### Task 5: Verify the foundation slice and prepare review

**Files:**
- Modify: Task 1–4 files only if verification identifies a defect.

**Interfaces:**
- Produces evidence that #165 has no product-module move and each acceptance criterion is checked.

- [ ] **Step 1: Run foundation verification**

```bash
bash scripts/test-architecture-policy.sh
bash scripts/test-architecture-check.sh
bash scripts/architecture-check.sh
bash scripts/guardrails/run.sh
```

Expected: each exits 0.

- [ ] **Step 2: Run project gates**

```bash
bash scripts/check-all.sh
cargo nextest run --all-features
```

Expected: both exit 0 with no warnings or failures.

- [ ] **Step 3: Audit scope**

```bash
git diff --check face086..HEAD
git diff --name-only face086..HEAD
rg -n 'rusqlite' src --glob '*.rs' -g '!src/store/**' -g '!src/errors.rs'
```

Expected: no whitespace errors, no capability move, and no new production
SQLite-driver reference outside `src/store/` and `src/errors.rs`.

- [ ] **Step 4: Commit a verified repair only if needed**

```bash
git add AGENTS.md guardrails.config.json lefthook.yml scripts/check-all.sh \
  scripts/architecture-check.sh scripts/architecture-policy.json \
  scripts/test-architecture-check.sh scripts/test-architecture-policy.sh \
  docs/designs/2026-09-17-domain-first-migration.md \
  docs/designs/2026-09-17-domain-first-migration-inventory.md
git commit -m "fix: complete domain guardrail foundation"
```

- [ ] **Step 5: Request final review before PR creation**

Give reviewers the `face086..HEAD` diff, fresh command outputs, and every #165
acceptance criterion. Resolve each load-bearing finding with a regression test,
rerun the affected test plus both project gates, then obtain final review.
