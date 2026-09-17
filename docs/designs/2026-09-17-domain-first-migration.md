# Domain-first Rust migration contract

Issue [#164](https://github.com/Falconiere/comemory/issues/164) is a
behavior-preserving migration from technical-layer ownership to business
capabilities. This document is the architecture contract implemented first by
#165. It fixes the allowed direction of travel before any source module moves.

## Scope and non-goals

The migration stays in one Rust crate. It must preserve the CLI and HTTP
contracts (flags, help, exit codes, JSON, routes, read-only and confirmation
ordering, cancellation and SSE), ranking, cloud protocols, markdown format,
SQLite format, and released migrations. It does not add features, endpoints,
commands, dependencies, a Cargo workspace, a database redesign, or snapshot
refreshes.

The existing local constraints remain binding: sibling module files rather than
`mod.rs`, descriptive filenames, 300 production code lines per file, 100 lines
per function, colocated unit tests, zero warnings, and the `store` SQLite
chokepoint with a zero driver-leak baseline outside its documented
`errors.rs` exception.

## Final ownership

`src/domains.rs` declares only real capability modules. A grown capability uses
the existing sibling shape: `src/domains/<name>.rs` plus
`src/domains/<name>/`; a single-file capability may initially have only the
`.rs` module. Neither a directory, README, nor module declaration may exist
without production code owned by that capability. Each real domain directory
has a README that indexes its files.

| Area | Final owner | Retained boundary |
| --- | --- | --- |
| Memory markdown, lifecycle, references | `domains::memories` | `cli` and `serve` render or transport requests only |
| AST, code indexing, repositories, hooks | `domains::code` | `store` owns all SQL |
| Extraction, source registry, document index | `domains::documents` | delivery starts jobs and renders reports |
| Edge algorithms and graph assembly | `domains::graph` | `store` provides data access only |
| Search, context, find, suggestion | `domains::retrieval` | `output` is CLI presentation only |
| Feedback, evaluation, mine, tune, bandit | `domains::learning` | corpus dashboards stay maintenance |
| Authentication, sync, watch, memory stores | `domains::sync` | HTTP policy stays in `serve` |
| Capture and distillation | `domains::capture` | host hooks and CLI invocation stay delivery |
| Doctor, retention, rebuild, stats, upgrade | `domains::maintenance` | `store` retains database primitives |
| Setup and agent installation | `domains::integrations` | prompt/rendering and clap stay CLI |

Outside domains, the final direct source directories are exactly `domains`,
`cli`, `serve`, `config`, `utilities`, and `store`. Root `tests/` remains the
integration, snapshot, and real-binary test home. Root entry, error, prelude,
and test-bridge files remain explicitly allowlisted source files; `output`
becomes CLI presentation rather than a final root module. During the staged
migration, the policy separately lists each legacy root module and its owning
child issue; a legacy name is not silently accepted merely because it exists.
`store` and root `migrations/` remain infrastructure. The project deliberately
retains `serve` as the HTTP-adapter name and `store` as the central SQLite
exception rather than renaming either to match a template.

## Ownership inventory and compatibility policy

#165 records a checked-in, line-oriented inventory at
`docs/designs/2026-09-17-domain-first-migration-inventory.md`. One row per
production `src/**/*.rs` file supplies `path`, `current public path` (or
`private`), `colocated-test bridge` (or `none`), `compile-time asset paths`
(or `none`), `owner`, `target path`, and `migration issue`. The checker derives
the production file list with `rg --files src -g '*.rs'`, ignores only
`src/**/tests/**`, and fails for an unlisted file, duplicate row, missing
referenced test bridge or asset, malformed owner, or an owner outside the
contract. This makes the inventory's completeness objective rather than a
manual assertion. It assigns every module to exactly one domain, delivery
adapter, shared primitive, or infrastructure owner. It records the API-core
assignment below verbatim; child folders travel with their parent core unless a
later issue explicitly splits a helper.

| Owner | Current `api` cores |
| --- | --- |
| memories | `save`, `delete`, `list`, `show`, `update`, `restore`, `trash`, `refresh_refs` |
| code | `ast`, `index_code`, `ingest_code`, `index_runs`, `repos`, `repo_admin`, `hooks`, `install_hooks` |
| documents | `index`, `sources`, `unindex` |
| graph | `graph`, `graph_nodes`, `graph_recompute`, `edges` |
| retrieval | `search`, `search_code`, `context`, `find`, `suggest`, `config_retrieval` |
| learning | `feedback`, `eval`, `mine`, `tune`, `bandit`, `learning`, `learning_proposals` |
| sync | `sync`, `memory_store` |
| integrations | `install`, `setup` |
| maintenance | `doctor`, `gc`, `gc_policy`, `prune`, `consolidate`, `rebuild`, `reembed`, `stats`, `overview` |
| delivery | `completions` |

CLI, HTTP, wire, and persisted-data compatibility is mandatory throughout.
Rust module paths are a public library surface only where `src/lib.rs` exports
them. #165 assigns every current public path one of `preserve`,
`crate-root-alias`, or `breaking`. The latter records the release version that
contains the break and its release-note entry before the move. Internal paths
have no compatibility promise. No folder facade is used: a crate-root alias is
permitted only when it preserves a coherent public item rather than recreating
a technical-layer barrel.

## Boundary policy and staged allowlist

Domains may depend on central `store`, `config`, `errors`, `prelude`, and
named shared utilities. They must not import `cli`, `serve`, `output`, or the
legacy `api` command-core tree. The policy includes a directed
owner-to-owner dependency table; cross-domain calls not in that table fail.
It lists setup's permitted runtime dependencies on maintenance doctor, code
hooks/repos, documents sources, and sync local-auth separately from #175's
migration prerequisites. Passive data-model imports are allowed only by an
exact policy entry. `store` must not call domain services: a service is a
domain-owned function, method, or constructor that performs lifecycle,
indexing, graph, retrieval, or maintenance work; owned structs/enums and their
serialization traits are passive models. SQL and `rusqlite` stay centralized
even when a domain has a file called `store`.

The current tree has intentional temporary violations. The policy stores each
as an exact `source file`, `canonical target module`, `edge class`, and one
removal issue. #165 creates this exhaustive record from the source tree; a
category label is never an allowlist entry. The architecture check evaluates
both migrated domain sources and listed legacy sources, so removing an entry
makes its still-present edge fail. The following groups guide the inventory,
but their final policy entries name concrete pairs:

| Temporary edge | Removal issue |
| --- | --- |
| `api::delete` and `api::prune` to CLI deletion helpers | #169 |
| `api` paging, date, reference, and embedding helpers to `cli` | #166 |
| `api::graph` and `api::graph_nodes` to CLI graph builders | #170 |
| `retrieval::code_ref_fetch` and `api::refresh_refs` to `serve::repo_root` | #167 / #169 |
| `store::memory_row` and `store::repo_drop` to graph algorithms | #177 |
| `store::migrate::preflight` to `source::lock` | #166 |

No wildcard exemption is permitted. A policy entry fails validation when its
issue is unknown, its source/target is malformed, it duplicates another entry,
or the named edge is absent. When a slice removes an edge, it removes the
matching allowlist record in the same change; a reintroduced edge then fails
the check.

## Architecture checker

`scripts/architecture-check.sh` is a project-owned shell gate using the
already-required `ast-grep` Rust parser and `jq`; it adds no dependency. It
accepts repository mode (no argument), scoped mode (`--file <path>...`), and a
temporary fixture root (`--root <path>`). `--file` validates the named source
files plus the policy and their ancestor/module context; it has the same
nonzero violations as repository mode. A project `lefthook.yml` command invokes
it beside `scripts/guardrails/run.sh --file` for staged Rust source files, so
the hook cannot bypass the scoped policy. Repository mode is wired into
`scripts/check-all.sh`; a configuration or argument error exits 3, policy
violations exit 1, and success exits 0.

The checker parses production Rust files only (never `src/**/tests/**`) and
uses AST paths rather than text matching, so comments and strings are ignored.
It normalizes direct, grouped, multiline, fully-qualified, and relative crate
paths. Imports or re-exports that alias a forbidden module are violations;
aliases cannot conceal a later call. It performs four deterministic checks:

1. Every direct `src/` directory and root `src/*.rs` module is in the explicit
   staged allowlist.
2. Every declared `domains::<name>` is one of the ten contract capabilities,
   its sibling file/directory shape is valid, permitted deeper paths are
   explicit, and declaration-only/README-only empty scaffolding fails.
3. Every forbidden delivery or legacy-core edge from a domain, and every
   cross-domain edge absent from the owner dependency table, fails unless its
   exact legacy policy entry is active.
4. Every `store` call into a domain service fails unless its exact temporary
   callback entry is active; imports of an exact permitted passive model pass.

It reports violations deterministically by source path then target path. An
allowlisted edge is silent; only its stale or malformed entry is a diagnostic.
The report groups multiple offending targets under one source file without
hiding any target. The script is invoked by `scripts/check-all.sh` after
guardrails and before the store chokepoint check. It deliberately does not
modify the vendored guardrails engine. `guardrails.config.json` separately
gains an explicit `src.topLevel`, exact `domains` / `domains/*` nested-path
rules ahead of the fallback, and `domains` README requirements once the first
capability lands.

The checker has a shell test harness using real temporary fixture trees. It
proves: an allowed staged tree succeeds; forbidden root directories and modules
fail; an undeclared domain, bad nested domain path, and empty scaffold fail;
direct, grouped, relative, and aliased domain-to-delivery imports fail; a
listed legacy edge succeeds and fails when its record is removed; stale,
duplicate, malformed, and unknown-issue entries fail; a store callback fails
while a permitted passive-model import succeeds. Each relevant case runs in
both repository and `--file` modes; a hook fixture proves `lefthook.yml` calls
the scoped checker for staged Rust paths.

## Migration order and dependencies

The work proceeds as independently green slices:

1. #165 records this contract, inventory, staged configuration, and checker.
2. #166 extracts transport-neutral context and shared primitives.
3. #167, #168, and #169 migrate code, documents, and memories. #170 follows
   code; #171 retrieval follows all four.
4. #173 learning follows retrieval; #172 sync follows shared, memory, and
   code; #174 capture follows sync; #176 maintenance follows migrated
   capabilities; #175 integrations follows maintenance, code, documents, and
   sync.
5. #177 removes storage callbacks; #178 removes legacy shells and temporary
   allowlists, then enforces the final tree.

Setup's runtime helpers (doctor, hooks, repos, sources, local auth) are normal
runtime dependencies, not evidence that its migration ticket may run early.
The migration order above prevents moving setup before those owners exist.

## Convention reconciliation

#165 compares AGENTS.md's historical D3/D4/D5 wording with the pinned
toolu-conventions commit `abd091eb50e91e18fdc8e07acf5b4e91c0a8addf`, replaces
any stale description, and records each retained stronger local rule as a
project deviation. The checked [inventory](2026-09-17-domain-first-migration-inventory.md)
is the evidence for the staged policy. It explicitly treats merged
[#162](https://github.com/Falconiere/comemory/issues/162) and
[#163](https://github.com/Falconiere/comemory/issues/163) as baseline behavior:
common-directory hooks and unseen/custom worktree labels, plus setup's seven
stable step IDs, offline missing-database-safe detection, report-only
cloud/document steps, and per-step failure rendering before exit 69 are
compatibility constraints, not migration work. `serve` remains the HTTP adapter
and `store` the SQLite exception throughout that preservation work.

## Verification

Each slice supplies its own focused real-data tests and updates file READMEs,
module declarations, asset paths, and path inventory. #165 specifically runs
the inventory completeness check, checker fixture harness in repository and
scoped modes, the staged-hook fixture, `bash scripts/guardrails/run.sh`, and
`bash scripts/check-all.sh`. The series also runs
`cargo nextest run --all-features`; the final closure runs `just e2e`,
`bash scripts/dup-check.sh`, and `cargo doc --no-deps --all-features`.
