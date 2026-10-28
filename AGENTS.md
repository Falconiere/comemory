# AGENTS.md

Canonical agent guidance for this repository. `CLAUDE.md` is removed.

Lean root on purpose: what to run and what not to break. Details live where
they are enforced — follow the pointer for the task at hand instead of loading
the whole tree. `docs/` is indexed by `docs/README.md`; each
`src/<module>/README.md` is the authoritative file-by-file index of its folder.

## Project Overview

`comemory` is Rust CLI that fuses engram-style developer memory, grepai-style
semantic code search, and ast-grep AST patterns into one binary, knit
together by SQLite-backed store (memory + code rows + edges). It is
**standalone agentic-RAG toolbox**, invoked from command line, over its
loopback HTTP API (`comemory serve`), or as MCP stdio server (`comemory
mcp`) that speaks same command cores as those two — still no in-process
LLM. Everything runs locally: markdown is source of truth and one SQLite
file (`comemory.db`) backs FTS5 + `sqlite-vec` + edges.

## Conventions that are easy to misread

- **`requireReadme` names folders, not files.** `src/<module>/README.md` is
  per-file index for files inside `src/<module>/`. Single-file modules need
  no README of their own; they are listed in the parent folder's README and
  documented with their `//!` module doc.
- **`POST` is not automatically mutating.** `RouteEntry::mutating` controls
  whether the read-only gate refuses a route. `POST /jobs/{id}/cancel` is
  deliberately `mutating: false`, so a runaway job can be stopped on a
  read-only server.

## Where to start reading

- `src/main.rs` / `src/lib.rs` — binary entry and library surface
- `src/cli.rs` / `src/cli/save.rs` — command dispatch and save flow
- `src/domains/retrieval/pipeline.rs` — search orchestration
- `src/store.rs` — SQLite connection management
- `docs/architecture.md` — full architecture explanation

## Key Commands

```bash
bash scripts/dev-install.sh     # source install: cargo install --path . + sync daemon ensure + completions
comemory setup                  # detect + apply this machine's/repo's onboarding
just check                      # umbrella gate (alias of scripts/check-all.sh)
just test                       # cargo nextest run --all-features
just qa                         # check-all + cargo-deny + dup-check + jscpd + cargo-dupes + machete + install.sh libc gate (docker)
just e2e                        # real-binary end-to-end harness
bash scripts/deploy.sh [--tag vX.Y.Z] [--dry-run]  # local stable release while Actions are off (docs/release.md §3b)
bash scripts/deploy.sh --resume vX.Y.Z             # finish a pushed release after deploy.sh exited 2; never rebuilds
```

Bare `cargo install --path .` places the binary and starts nothing (no
daemon, no completions) — prefer `scripts/dev-install.sh`.

Full command/flag inventory: `docs/cli-reference.md` (mechanically checked
against `--help`; every flag needs a scenario citing a test). Prefer
`comemory --help` over guessing flags.

## Binding Rules (apply to every contribution)

comemory follows toolu-conventions Rust stack
(`github.com/Falconiere/toolu-conventions`), plus stricter local ceilings.
Deviations are enumerated under "Deviations from toolu-conventions" — nowhere else.

1. **No duplication / redundancy.** Shared logic is extracted into helper.
   Enforced by `scripts/dup-check.sh` (excludes test trees) and review: a
   one-sided count ratchet against `dup-baseline.txt`, plus a hard
   **unledgered** residual budget of **5% of the 285-pair anchor (ceiling
   14)**. Earned residuals live in `dup-ledger.txt`
   (`path|fnA|fnB|category|evidence`); same-name pairs cannot be ledgered
   and must be burned. Over-ceiling unledgered fails even when within the
   ratchet file.
2. **No barrels — no `mod.rs`.** A module that grows into a folder keeps its
   file beside it (`src/store.rs` declares `mod migrate;`,
   `src/store/migrate.rs` holds it). A file whose only
   content is `pub use` re-exports is banned; `pub use` is legitimate only in
   `src/lib.rs`. Enforced by the `no-barrels` guardrails check.
3. **One responsibility per file; filename matches content.** `snake_case`,
   named after the file's primary item. Enforced by `filename-case` check.
4. **Size ceilings.** 300 code lines per file in `src/`, 100 per function
   (blanks/comments excluded; tests exempt) — stricter than kit's 500 by
   choice. Declared in `guardrails.config.json`; enforced by `file-size`
   check and `clippy::too_many_lines = "deny"`.
5. **Zero errors, zero warnings, no silencing.** Clippy over all targets and
   features with `-D warnings` must be clean. No `#[allow(...)]` in
   production code; no `.unwrap()` / `.expect(...)` / `panic!` / `todo!` /
   `unimplemented!` / `println!` / `eprintln!` / `dbg!` outside tests and benches — return `Result` with `?`,
   diagnose through `tracing`. Every `unsafe` block/fn carries a `// SAFETY:`
   line directly above it.
6. **Tests never share a file with production logic, and colocate by
   default.** No `#[cfg(test)] mod tests { ... }` body in any `src/` file.
   See "Testing" for the placement rule.
7. **Doc line on every module and public item.** `//!` on every module,
   `///` on every `pub` item (`missing_docs = "warn"` under `-D warnings`).
   Folder READMEs are a separate, folder-only rule: see D5.
8. **Docs in sync.** A user-facing change (CLI flags, public API, config, env
   vars) updates `README.md`, `AGENTS.md`, `docs/`, and the module's own
   `src/<module>/README.md` in the same change.
9. **Real data, no mock-data tests.** A test that only proves a mock returns
   what it was told is banned.
10. **`rusqlite` is confined to the store module.** Only `src/store.rs` and
    `src/store/` may name the driver — sole permanent exception:
    `src/errors.rs`'s `Error::Sqlite(#[from] rusqlite::Error)`, kept so `?`
    works inside `store/` without `.map_err`; every other module takes
    `store::Connection` / `store::Transaction` and reads back owned `Vec<T>`,
    `Option<T>`, or scalars. Two-way: `store/` calls no domain logic either —
    callers pass row data in (`store::CreatedWindow`, `store::MemoryLinks`,
    passive types via the `passive_store_models` allowlist) and own post-write
    derived refresh. Enforced by `scripts/store-chokepoint-check.sh`
    (baseline `0`, ratcheted both ways) and `scripts/architecture-check.sh`.

One command must be green before every push:

    bash scripts/check-all.sh      # or: just check

## Code Style

`rustfmt` stable-only options — 4-space indent, 100 columns, edition 2024
(`rustfmt.toml`; no nightly-gated knobs so `cargo fmt` is deterministic).
`Result<T>` alias from `crate::prelude::*`; errors flow through `Error` in
`src/errors.rs`. Lint policy lives once in `Cargo.toml [lints]` plus
`clippy.toml` — never per-call-site `#[allow(...)]`.

## Module Map

Top level: `cli`, `config`, `domains`, `mcp`, `serve`, `store`, `utilities`.
Capabilities live one-per-folder under `src/domains` (`architecture`,
`memories`, `code`, `documents`, `graph`, `retrieval`, `learning`, `sync`,
`capture`, `maintenance`, `integrations`, `projects`); delivery adapters are
`cli` / `serve` / `mcp`; `store` is the SQLite chokepoint. No per-module
narrative here — each folder's `README.md` is the authoritative file index
(Binding Rule 7); start from `src/domains/README.md` for capability layout.

## Environment Variables

Layered: `Config::defaults` → config file → environment
(`src/config/env.rs`). Full table (every variable, default, validation) is
in `docs/configuration.md` — read it instead of guessing. `[tune]` knobs and
`[reinforce] enabled` are file-only (`COMEMORY_REINFORCE_SEARCH_EDIT_DAYS`
still overrides the `[reinforce]` lookback); vector dims (1024/768) are baked
into vec0 DDL, not env-configurable. Read env only through `src/config/` —
direct `std::env::var` elsewhere fails `no-direct-env-var`. `--data-dir` and
`--json` are global CLI flags, valid before or after the subcommand.

## Memory Data Model and Save Flow

Frontmatter schema v1 is defined by `Frontmatter`
(`src/domains/memories/frontmatter.rs`); the save contract (content-addressed
id = 8-hex SHA-256 prefix, idempotent replay with `created: false`,
`409 id_collision` on same-id/different-body, advisory SimHash near-dup) is
specified in `docs/guides/http-api.md` § Save contract and
`docs/architecture.md`. Headlines: markdown is source of truth (kept when the
SQLite mirror fails — run `comemory rebuild`); vectors are BYO (`--vector` /
`--vector-stdin`, else lexical-only); `serve` never pushes outward, `cli`
pushes inline best-effort after local writes.

## Testing

- Runner: `cargo nextest run --all-features` (alias `just test`).
- Match `.github/workflows/test.yml`'s pinned Rust 1.99.1 on quality gates:
  `PATH="$(dirname "$(rustup which --toolchain 1.99.1 cargo)"):$PATH"
  bash scripts/check-all.sh` (`RUSTUP_TOOLCHAIN` alone is ignored when
  Homebrew `cargo` comes first on `PATH`).
- Placement: drives the real binary (`assert_cmd`), owns an `insta`
  snapshot, or exercises CLI surface → crate-root `tests/` (`tests/cli__<cmd>.rs`
  per command, `tests/cli_scenario_*.rs` + `tests/serve_scenario_*.rs` journeys,
  `docs/scenarios/` plan enforced by `tests/cli_scenario_catalog.rs`).
  Otherwise colocate in a sibling `tests/` folder with a one-line
  `#[path]` include from the production file; keep each `tests/` tree flat.
- Why `tests/` cannot go away: `CARGO_BIN_EXE_comemory` is published only to
  integration targets (colocated `assert_cmd` goes stale-binary green);
  `insta` resolves snapshots from the test file's location; shared fixtures
  live once in `tests/common/` (bridged via `src/test_common.rs`).
- Every test and bench file's first line is the shared lint header
  `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp, clippy::too_many_lines)]`;
  it imports through
  `use comemory::...` (`extern crate self as comemory;` in `lib.rs`), addresses
  data via `concat!(env!("CARGO_MANIFEST_DIR"), "/tests/...")`. Real data only.
- `.config/nextest.toml` serializes two groups: `embedder`
  (`embedder|memory_index|code_index`, so the model download cannot race) and
  `config::env::tests*`. Colocated tests share one lib test binary, so only
  that name filter isolates env mutation — a test that calls
  `std::env::set_var` must live under `config::env::tests`.

## Quality Gates

`bash scripts/check-all.sh` is the umbrella gate (CI parity on
ready-for-review PRs). It runs, in order:

```
scripts/fmt-check.sh             # cargo fmt --check
scripts/type-check.sh            # cargo check --all-targets --all-features
scripts/lint-check.sh            # cargo clippy --all-targets --all-features -- -D warnings
scripts/guardrails-check.sh      # scripts/guardrails/run.sh
scripts/architecture-check.sh    # module ownership and dependency policy
scripts/store-chokepoint-check.sh # rusqlite confined to src/store/
scripts/typos-check.sh           # typos
scripts/cli-docs-check.sh        # docs/cli-reference.md vs the real --help output
scripts/http-docs-check.sh       # docs/guides/http-api.md documents every live /api/v1 route
scripts/migration-check.sh       # shipped migrations/*.sql byte-identical to first release tag
scripts/replication-coverage-check.sh # replication harness coverage
```

`guardrails.config.json` is the single declaration of every structural
ceiling; nothing else declares a competing number. One rule, one enforcer:

| Rule | Owner |
| --- | --- |
| Formatting / type-check | `rustfmt.toml` + `scripts/fmt-check.sh`; `scripts/type-check.sh` |
| unwrap/expect/panic/todo/unimplemented/print_*/`too_many_lines`/pedantic | `Cargo.toml [lints]` + `clippy.toml`, `scripts/lint-check.sh` |
| File size 300 / function size 100 | `guardrails.config.json` (`fileSize`, `functionSize`), `file-size` check + `clippy::too_many_lines` |
| No `mod.rs` barrels; folder tree shape; `snake_case` names; folder READMEs | `guardrails.config.json` (`barrelNames`, `src.nested`, `filenameCase`, `src.requireReadme`), guardrails checks |
| No inline `#[cfg(test)] mod tests { ... }` | `scripts/guardrails/patterns/rust/no-inline-test-module.yml` |
| No direct `std::env::var` outside `config/` / `tests/` | `scripts/guardrails/patterns/rust/no-direct-env-var.yml` |
| No `unsafe` without `// SAFETY:` (D1); no `#[allow(...)]` in production (D6) | `no-unsafe-without-safety.yml`, `no-allow-attribute.yml` |
| Committed secrets, shadow configs | `guardrails.config.json` (`secrets`, `shadowConfigs`) |
| Typos | `typos.toml`, `scripts/typos-check.sh` |
| `docs/cli-reference.md` / `docs/guides/http-api.md` drift | `scripts/cli-docs-check.sh`, `scripts/http-docs-check.sh` |
| Shipped migration SQL immutable | `scripts/migration-check.sh` |
| Duplication ratchet | `scripts/dup-check.sh` vs `dup-baseline.txt` (pinned `similarity-rs`); `scripts/jscpd-check.sh` (5% lines + clone count); `scripts/cargo-dupes-check.sh` (5% near-lines + group counts) |
| rusqlite confined to `src/store/` | `scripts/store-chokepoint-check.sh`, `store-leak-baseline.txt` |

`just qa` adds the tool-pinned gates (`deny-check`, `dup-check`,
`jscpd-check`, `cargo-dupes-check`, `machete-check` — version mismatch
hard-fails everywhere, missing tool warns locally but fails in CI)
plus the `install.sh` libc gate on real distro images. Task is not "done"
until `scripts/check-all.sh` exits 0.

## Distribution

Releases are driven by the **release-plz** bot (version + changelog + tag)
with **cargo-dist** building artifacts; see `docs/release.md`. `install.sh`
(hand-written, POSIX sh) is the supported installer; `comemory upgrade`
drives it. While GitHub Actions are off, `scripts/deploy.sh` cuts the release
locally (see Key Commands). Not published to crates.io; Homebrew via
`Falconiere/tap/comemory`.

## Claude Code Hooks

`.claude/settings.json` wires **only** toolu-conventions guardrails entries —
comemory's old dispatcher layer (`.claude/hooks/`) is **retired**, one
enforcement surface, not two:

- **PostToolUse** (`Edit|Write`) — `bash scripts/guardrails/run.sh --hook`
- **Stop** — `bash scripts/guardrails/run.sh --stop`

The hook writes to stderr and exits **2** (Claude Code ignores `1`). The
retired dispatcher was a tool-call interceptor, never a repo-state checker, so
guardrails did not absorb it. Only its `--no-verify` / `--no-gpg-sign` block
has a replacement: `lefthook.yml` (fmt/guardrails/typos pre-commit,
`check_all` pre-push) and CI's `scripts/check-all.sh`, so a local bypass
cannot land. Everything else it blocked is now **unguarded**:

- `rm -rf`, `git reset --hard`, `git checkout .`, `chmod -R 777` — only the
  agent harness's own destructive-command confirmation.
- `git push --force` — nothing. Use `--force-with-lease` for a rebased branch.
- `scripts/guardrails/` is vendored verbatim from the kit — never hand-edit
  it; `guardrails.config.json` and `scripts/guardrails/patterns/rust/` are the
  sanctioned knobs. Convention only.
- Rust project: no `npm` / `bun` / `pip` / `uv`. Convention only.
- Run `rustfmt` / `cargo clippy` through `scripts/fmt-check.sh` /
  `scripts/lint-check.sh`, the canonical invocations. Convention only.

User-facing docs live under `docs/`, in Diátaxis tiers indexed by
`docs/README.md`: tutorial (`getting-started.md`), how-tos (`guides/`),
reference (`cli-reference.md`), explanation (`architecture.md`).

## Deviations from toolu-conventions

Deliberate, documented here. None weakens kit's intent; several are strictly
stronger. `scripts/test-architecture-policy.sh` greps this section for the
D3/D4/D5 policy lines — keep them verbatim.

- **D1 — `unsafe_code = "forbid"` is NOT set.** 7 FFI-necessary `unsafe`
  blocks + 3 `unsafe extern "C" fn` items (FTS5 tokenizer via
  `libsqlite3-sys`, `sqlite-vec` auto-extension), plus 1 `Command::pre_exec`
  block between `fork`/`exec`, have no safe wrapper; `forbid` admits no local override.
  Stronger replacement: `no-unsafe-without-safety.yml` fails any `unsafe`
  lacking `// SAFETY:` directly above it.
- **D2 — `fileSize.max` is 300, not kit's 500**, with one override: 400 for
  hand-maintained `.github/workflows/release.yml` (generated by
  `cargo dist init`; keep hand blocks minimal). Re-measure with the gate's own
  awk (`GR_FS_AWK` in `scripts/guardrails/checks/file-size.sh`), not a
  hand-rolled one.
- **D3 — `barrelNames: ["mod.rs"]`** is inherited unchanged from pinned
  Rust conventions, not local deviation. It makes `<dir>.rs` beside
  `<dir>/` layout permanent and machine-checked.
- **D4 — `src.nested` replaces the starter map** with exact, project-specific
  folder policy below. It retains kit's test separation while replacing its
  generic domain shape with comemory's staged capabilities and delivery adapters:

  - `domains`: `architecture`, `memories`, `code`, `documents`, `graph`, `retrieval`, `learning`, `sync`, `capture`, `maintenance`, `integrations`, `projects`
  - `domains/*`: `ast`, `cloud`, `consolidation`, `daemon`, `doctor`, `document`, `drain`, `evaluation`, `exchange`, `index_code`, `install`, `memory_store`, `replica`, `rebuild`, `repos`, `retention`, `setup`, `source`, `unified`, `upgrade`, `tests`, `proptest-regressions`
  - `store`: `tokenizer`, `migrate`, `tests`
  - `cli`: `output`, `tests`, `proptest-regressions`, `setup`
  - `serve`: `routes`, `jobs`, `tests`, `proptest-regressions`
  - `serve/routes`: `memories`, `maint`, `tests`, `proptest-regressions`
  - `*`: `tests`, `proptest-regressions`

  `serve` remains HTTP adapter, `mcp` stdio MCP adapter, `store` the SQLite
  exception; no template name controls their ownership.
- **D5 — `src.requireReadme` replaces the pinned single-`domains` rule during staging with exactly 47 folders.**
  It names folders, not files: each listed grown folder has `README.md` that
  indexes its contents, while a single-file module is listed in its parent folder
  README and documented by its module doc. Configured list, not prose count,
  is source of truth.
- **D6 — two project-local ast-grep rules** (`no-unsafe-without-safety.yml`,
  `no-allow-attribute.yml`) are additions to
  `scripts/guardrails/patterns/rust/`; no kit file is modified, so kit
  re-copy merges rather than clobbers.
- **D7 — eight `clippy::pedantic` lints start at `allow` with counted
  burn-down** (measured counts inline in `Cargo.toml [lints.clippy]`). Honest
  and burnable; the in-code `#[allow]` ban stands. See `docs/lint-debt.md`.
- **D8 — extra quality gates retained** beyond kit's four-step command
  (`typos`, `cli-docs`, `coverage`, `eval`, plus tool-pinned `dup`,
  `machete`, `deny`); each guards a comemory-specific failure mode. Mutation
  CI job removed; `just mutation` still runs locally.
- **D9 — `tests/common/` stays at crate root**, bridged via
  `src/test_common.rs`, so colocated and integration tests share one fixture
  copy. Its `#![allow(dead_code)]` is the one sanctioned exception.
- **D10 — `extern crate self as comemory;`** in `src/lib.rs` keeps colocated
  test imports identical to the crate-root suite's.
- **D11 — 192 of 692 test files remain crate-root integration tests** (kit's
  "one file per public surface" category): pinned by the
  `CARGO_BIN_EXE_comemory` constraint — see "Testing".
- **D12 — RESOLVED.** Dispatcher retired; `.claude/settings.json` matches kit.
  See "Claude Code Hooks".
- **D13 — `benches/` untouched.** Criterion harnesses are neither `src/` nor
  `tests/`; they carry only the canonical test/bench lint header.
- **D14 — `[workspace] members = ["."]` stays**, required by `cargo-dist`;
  workspace mode keys on `guardrails.workspace.json`, not Cargo metadata.

## Agent integration

`comemory install claude`, `comemory install codex` and `comemory install muse`
install bundled skills, hooks and the MCP server through the host plugin
manager (`--dry-run` previews; muse then needs `muse plugins approve
comemory` before its hooks run); `comemory install cursor-agent` links skills
into `~/.cursor/skills/` plus the MCP server entry (hooks are a follow-up)
and `comemory install hermes` links skills only. Prefer MCP
`find` with `k: 3`, then `show`; `context` returns full bodies. See
[installation and migration](docs/guides/agent-integration.md).
