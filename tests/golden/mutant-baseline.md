# Mutant Baseline — Phase 0

This is the Phase-0 mutation baseline. The acceptance bar for Phase 1 is **zero survivors
among these four modules** after test coverage work is complete.

Modules scoped: `src/cli/output/**`, `src/domains/memories/**`,
`src/domains/graph/**`, `src/serve/**`. Those first two were `src/output/**`
and `src/memory/**` when this snapshot was taken; the folders moved with #169
and #178. `scripts/mutation-check.sh` does not run in CI, so the scan has NOT
been re-run since — the module paths below are updated, and each line citation
is marked with whether it was re-verified against the current tree.
Config: `.cargo/mutants.toml` (excludes `src/main.rs`, `src/cli/**`; `timeout_multiplier = 3.0`; `test_tool = "nextest"`).
Run date: 2026-06-13.

---

## src/cli/output

**3 survivors** (citations re-verified against the current tree, 2026-09-18:
the two line numbers below still name the mutated expression; the third did
not, and is cited by symbol)

- `src/cli/output/graph.rs:56` — replace `>` with `>=` in `to_dot`
- `src/cli/output/search.rs:63` — replace `==` with `!=` in `write_tty`
- `src/cli/output/tty.rs` — replace the body of the bold-cyan header writer
  with `Ok(())`. Recorded as `:15` against `header`; `header` was later split
  into a stdout wrapper (now line 21) and the testable `write_header` (now
  line 15), so the line number names a different function than the scan meant.
  Cited by symbol instead.

## src/domains/memories

**1 survivor** (line citation STALE — not re-verified)

- `src/domains/memories/store.rs` — replace `||` with `&&` in
  `MemoryStore::list`. Recorded as `src/memory/store.rs:207`; the file moved
  with #169 and `MemoryStore::list` now begins at line 327, so the original
  line number no longer resolves and is deliberately not restated as if it
  did. Re-run `scripts/mutation-check.sh` to refresh it.

## src/domains/graph

**10 survivors**

- `src/domains/graph/cochange.rs:92` — replace `|` with `&` in `mine_cochange`
- `src/domains/graph/cochange.rs:92` — replace `|` with `^` in `mine_cochange`
- `src/domains/graph/cochange.rs:115` — replace `||` with `&&` in `mine_cochange`
- `src/domains/graph/cochange.rs:129` — replace `>` with `>=` in `mine_cochange`
- `src/domains/graph/cochange.rs:137` — replace `<` with `>` in `mine_cochange`
- `src/domains/graph/cross_link.rs:70` — replace `+` with `-` in `extract_refs`
- `src/domains/graph/cross_link.rs:70` — replace `+` with `*` in `extract_refs`
- `src/domains/graph/cross_link.rs:73` — replace `==` with `!=` in `extract_refs`
- `src/domains/graph/imports.rs:157` — replace match guard `module.contains('/') && !module.starts_with('.')` with `true` in `PathIndex::resolve`
- `src/domains/graph/imports.rs:157` — replace `&&` with `||` in `PathIndex::resolve`

## src/serve

**0 survivors**

All serve mutants were caught by the existing test suite.
