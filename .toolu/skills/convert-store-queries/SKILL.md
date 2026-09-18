---
name: convert-store-queries
description: Use when converting comemory store queries to toolu-orm or revisiting raw SQL after an ORM upgrade.
metadata:
  toolu:
    origin: agent
    created: 2026-09-18T14:45:42Z
---
## When to Use

When converting comemory store queries to toolu-orm, or revisiting retained
SQL after an ORM upgrade. This procedure does not change historical migrations.

## Procedure

1. Read `docs/guides/runtime-orm.md` and inspect the resolved crate source.
   Compare the full query semantics with the available builders, including
   predicates, projection order, NULLs, sorting and conflict handling.
2. Use the existing schema table constructors and lowercase companion columns
   (`Memories::select()`, `memories::id`). Send generated `.to_sql()` results
   through `store::orm`, retaining owned row decoders and caller transactions.
   Keep specialized execution helpers when required for error semantics, such
   as `fts::run_fts_query` converting FTS parse errors into empty results.
3. Convert supported statements even inside files containing unsupported ones.
   Record retained capabilities in `docs/guides/runtime-orm.md`; check upstream issues
   before filing new ones when the user authorized issue creation.
4. Review the diff against the original query, then exercise the existing
   colocated real-database tests. Add a regression only for an uncovered
   semantic risk. Update folder indexes and
   `docs/designs/2026-09-17-domain-first-migration-inventory.md` whenever
   extracting a flat sibling module.

## Pitfalls

- `OR REPLACE` deletes/reinserts; it cannot replace a preserving upsert.
- Generated projections put ordinary columns before expression columns.
- Compare bind counts for repeated predicates. The ORM allocates fresh slots
  for each occurrence; converting reused numbered placeholders can exceed
  SQLite's parameter limit on inputs that previously succeeded.
- Nested parameterized raw expressions misnumber placeholders in 0.6.0 and 0.7.0;
  use the supported typed predicates or the documented separate-filter form.
- For repeated writes, generate SQL once and use `orm::execute_many` with
  each row bound in placeholder order; preserve the caller transaction.
- Preserve fractional f32 BM25 decimal literals before supplying f64 ORM
  weights. Direct binary widening can change result score bits.
- Keep set-based cleanup and date normalization in the database. Avoid
  client-side filtering or read-modify-write substitutes for missing builders.
- Inventory bridge columns describe actual test-module includes, not every
  test that indirectly covers a module.

## Verification

Run targeted nextest suites, then `cargo nextest run --all-features` and
`bash scripts/check-all.sh`. Full tests include loopback servers. Check the
final retained-SQL inventory and verify `migrations/` has no diff. Run the pinned
`scripts/dup-check.sh` separately: `check-all.sh` does not include it. Extract
shared behavior within the authorized scope when conversion raises the count;
keep the detector settings fixed, and lower `dup-baseline.txt` together with a
fresh complete `docs/dup-debt.md` inventory when the count improves. Check the
300-code-line ceiling after extracting helpers. The ORM bridge tests cover
binding/error/rollback behavior; the fractional BM25 test compares score bits
with independent SQLite SQL.
