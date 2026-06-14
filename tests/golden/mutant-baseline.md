# Mutant Baseline — Phase 0

This is the Phase-0 mutation baseline. The acceptance bar for Phase 1 is **zero survivors
among these four modules** after test coverage work is complete.

Modules scoped: `src/output/**`, `src/memory/**`, `src/graph/**`, `src/serve/**`.
Config: `.cargo/mutants.toml` (excludes `src/main.rs`, `src/cli/**`; `timeout_multiplier = 3.0`; `test_tool = "nextest"`).
Run date: 2026-06-13.

---

## src/output

**3 survivors**

- `src/output/graph.rs:90` — replace `>` with `>=` in `to_dot`
- `src/output/search.rs:81` — replace `==` with `!=` in `write_tty`
- `src/output/tty.rs:15` — replace `header -> Result<()>` with `Ok(())`

## src/memory

**1 survivor**

- `src/memory/store.rs:207` — replace `||` with `&&` in `MemoryStore::list`

## src/graph

**10 survivors**

- `src/graph/cochange.rs:92` — replace `|` with `&` in `mine_cochange`
- `src/graph/cochange.rs:92` — replace `|` with `^` in `mine_cochange`
- `src/graph/cochange.rs:115` — replace `||` with `&&` in `mine_cochange`
- `src/graph/cochange.rs:129` — replace `>` with `>=` in `mine_cochange`
- `src/graph/cochange.rs:137` — replace `<` with `>` in `mine_cochange`
- `src/graph/cross_link.rs:70` — replace `+` with `-` in `extract_refs`
- `src/graph/cross_link.rs:70` — replace `+` with `*` in `extract_refs`
- `src/graph/cross_link.rs:73` — replace `==` with `!=` in `extract_refs`
- `src/graph/imports.rs:157` — replace match guard `module.contains('/') && !module.starts_with('.')` with `true` in `PathIndex::resolve`
- `src/graph/imports.rs:157` — replace `&&` with `||` in `PathIndex::resolve`

## src/serve

**0 survivors**

All serve mutants were caught by the existing test suite.
