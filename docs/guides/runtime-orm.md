# Runtime queries with toolu-orm

Use the declared `schema_*.rs` tables and toolu-orm query builders for runtime
SQL that the installed library can represent. Keep unsupported SQL in the
store module, with its upstream capability issue recorded below. This guide
covers query construction; [schema migrations](schema-migrations.md) describes
schema declarations and the migration runner.

## Execution and behavior

The project resolves toolu-orm 0.7.0 with the `rusqlite` feature. Builders
return SQL and bound `Value` parameters through `to_sql()`.
`src/store/orm.rs` executes those results on the existing connection using
cached statements and the store's row decoders. This preserves native SQLite
errors, optional-row behavior, and caller-owned transactions. Repeated writes
generate one statement and pass it to `execute_many`, which prepares once
and binds each row in placeholder order. The bridge avoids creating another
connection or an async runtime for synchronous store work.
FTS queries use the existing `run_fts_query` helper with generated SQL and
values so malformed MATCH expressions retain their empty-result behavior.

Prefer generated table constants such as `schema_memory::memories::id`, and generated
`select`, `insert`, `update`, and `delete` constructors. Ordinary projections,
equality/NULL/IN predicates, compatible ordered comparisons, ordering,
pagination, count/existence checks, bound inserts, and bound updates/deletes
belong in builders. Optional filters can be added conditionally instead of
retaining parameter-NULL disjunctions.

FTS5 and vec0 are supported query capabilities: use `Expr::table_match`,
`fts5::bm25`, `SelectBuilder::knn`, and `vec0::distance` where applicable.
Keep the existing FTS parse-error handling, BM25 sign convention, vector
dimension checks, scope filters, and KNN oversampling. A virtual table alone
is not a reason to keep an entire raw query.

## Capability inventory for retained SQL

These are capability-based exceptions, not exemptions for every query in a
listed file. Convert supported statements in the same file. When a builder
supports the surrounding statement but needs an unsupported scalar fragment,
keep only that fragment raw and bind its values.

| Capability missing from 0.7.0 | Runtime examples | Upstream tracking |
| --- | --- | --- |
| SQLite `ON CONFLICT ... DO UPDATE`, inserted expressions, and `RETURNING` | Selected-field memory/source/sync upserts; feedback and edge counter increments; `code_row` database timestamps and generated ids | [#108](https://github.com/Falconiere/toolu-orm/issues/108) |
| `DISTINCT`, `GROUP BY`, `HAVING`, and typed aggregate projections | `sources` status counts; unique code paths; grouped graph nodes; deduplicated feedback queries | [#109](https://github.com/Falconiere/toolu-orm/issues/109) |
| Recursive/nonrecursive CTEs, set operations, subquery expressions, and table-valued sources | `edges_retrieval` graph walks; `edges` neighbor queries; `code_graph_nodes` lookup unions; set-based cleanup in `prune_apply`, `repo_drop`, and `code_row`; `json_each` and `pragma_table_info` | [#110](https://github.com/Falconiere/toolu-orm/issues/110) |
| Table aliases, qualified typed projections, and compound `JOIN ... ON` | `code_signals` multi-column feedback identity; `prune_signals` memory self-join; repeated endpoint joins in `edge_fts` | [#111](https://github.com/Falconiere/toolu-orm/issues/111) |
| Composable scalar functions, arithmetic, text ordering comparisons, `LIKE ... ESCAPE`, and expression ordering | Normalized date windows in memory FTS/KNN; literal substring matching in `memory_list`/`retrieval_log`; access increments; `COALESCE` ranking signals; `CASE`/text rendering in `edge_fts` | [#112](https://github.com/Falconiere/toolu-orm/issues/112) |
| `INSERT ... SELECT` and separately quoted database/table identifiers | `rebuild_copy*` copies from `old` into `main`; graph-label materialization in `edge_fts`; inspection of `old.sqlite_master` | [#114](https://github.com/Falconiere/toolu-orm/issues/114) |
| SQLite administration and extension bootstrap | `VACUUM INTO` snapshots; attach/detach lifecycle; pragma inspection; the tokenizer's `SELECT fts5(?1)` pointer handshake | [#115](https://github.com/Falconiere/toolu-orm/issues/115) |
| Reusable bound parameters across typed predicates | `edges_retrieval::co_change_weight` shares file IDs between both edge directions; separate bindings exceed SQLite's variable limit for previously valid working sets | [#116](https://github.com/Falconiere/toolu-orm/issues/116) |

`INSERT OR REPLACE` must not substitute for an upsert that preserves an
existing row: delete/reinsert semantics can cascade references and reset
unassigned fields. Likewise, moving a `LEFT JOIN` condition into `WHERE`
can remove unmatched rows. Keep unsupported set-based operations in SQL
rather than changing them into full-table loads or read-modify-write loops.

The tokenizer bootstrap binds a C pointer with `sqlite3_bind_pointer`; it is
not an ordinary value parameter. Its FFI lifecycle remains a deliberate
driver-level operation. Simple pragma reads may instead use native rusqlite
pragma methods; those methods still belong inside the store boundary.

## Version-specific traps

The audit checked published 0.7.0 source on 2026-09-18 after upgrading from
0.6.0. The retained query capabilities above remain unsupported. Their
issues include concrete consumer examples and inspected upstream APIs.

- **Raw expression parameter numbering:** a parameterized `Expr::raw` after
  another bound predicate inside one `.and()`/`.or()` expression reuses the
  earlier placeholder index. This was reproduced against the installed
  0.6.0 library and remains in 0.7.0 ([#113](https://github.com/Falconiere/toolu-orm/issues/113)).
  Separate top-level `.filter(...)` calls correctly offset their parameters.
  Use bare `?` placeholders in independently composed raw fragments; do not
  carry a whole statement's numbered placeholders into them.
- **One-row fetches:** 0.7.0 fixes ORM `fetch_one`/`fetch_optional` collecting
  all matches before selecting one ([#87](https://github.com/Falconiere/toolu-orm/issues/87)).
  The store bridge still reads one row directly to preserve native errors
  and the existing connection and transaction boundaries.
- **Offset without limit:** 0.7.0 automatically emits SQLite's `LIMIT -1` for
  offset-only pagination, fixing 0.6.0 ([#92](https://github.com/Falconiere/toolu-orm/issues/92)).
  Existing explicit `LIMIT -1` queries remain equivalent.
- **Projection order:** builders emit ordinary columns before `column_expr`
  projections. Keep row decoder indexes aligned when a query mixes them.
  Typed projections currently omit table qualification; duplicate joined
  column names need a qualified expression until #111 is resolved.
- **Fractional BM25 weights:** preserve the former f32 decimal literals when
  supplying the ORM's f64 weights. Binary widening changes some result score
  bits; the code FTS regression compares against an independent SQLite query.

## Rust API

This conversion changes `store::stats_counts::scoped_count` from
`(conn, table, predicate, repo)` to `(conn, Corpus, repo)`; the new signature
is already in `src/store/stats_counts.rs` and ships in the next release. Use
`Corpus::LiveMemories`, `Corpus::TrashedMemories`, `Corpus::CodeSymbols`, or
`Corpus::Documents` instead of passing SQL text. The CLI and HTTP statistics
shapes are unchanged. The other converted public store interfaces are preserved.

## Scope and verification

Shipped migration SQL is immutable and continues through the existing
marker-keyed `execute_batch` runner. Schema declarations and journal files
remain unchanged by runtime query conversion. SQL fixtures and assertions in
tests/benches may remain raw so they can independently verify actual SQLite
behavior rather than reproducing the same builder under test. Runtime
preflight reads are still candidates for conversion when supported.

Verify converted behavior against the real database: errors and optional
rows, NULLs, update counts, transaction rollback, pagination, date precision,
FTS ranking/parse errors, vector scope filtering, and literal wildcard
handling as applicable. Run the repository's `bash scripts/check-all.sh`
before pushing. On a future ORM upgrade, revisit this inventory and remove
exceptions only when the corresponding behavior passes its regression tests.
