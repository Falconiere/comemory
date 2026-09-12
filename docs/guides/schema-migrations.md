# Schema migrations

**Goal:** change `comemory.db`'s schema the way the project expects — declare
the table as a struct where toolu-orm can express it, generate the SQL, and
wire the numbered file into the runner. This is a contributor guide; nothing
here changes what a user sees (schema upgrades stay automatic and are
described in [Upgrading comemory](upgrading.md)).

## The two halves

comemory uses [toolu-orm](https://github.com/Falconiere/toolu-orm) for the
*generate* half of the migration loop and its own runner for the *apply*
half:

| Half | Owner | What it is |
| --- | --- | --- |
| **Declare + generate** | toolu-orm | `#[table]` / `#[fts5_table]` / `#[vec0_table]` structs under `src/store/schema_*.rs`; `store::schema::registry()` assembles them. `just migration <name>` diffs the registry against `migrations/<newest>.snapshot.json` and writes `migrations/NNNN_<name>.sql`, a new snapshot, and a `_journal.json` entry with the file's SHA-256 |
| **Apply** | comemory (`store::migrate`) | `execute_batch` over every file in `MIGRATIONS`, gated by `schema_meta` markers, with the preflight snapshot and forward-compat refusal unchanged. toolu-orm's own runner is not used: it would take ownership of the connection `open` returns, and the marker-keyed runner is what the preflight guards read — so no `_migrations` table exists |

A generated file separates statements with `--> statement-breakpoint` lines.
Those are SQL comments, so the runner applies a generated file exactly as it
applies a hand-written one.

## Which tables are declared

`store::schema::DECLARED_TABLES` is the authoritative list. As of v0.29, pinned to toolu-orm 0.5.0 (#70 below is fixed upstream after that release; bump and declare those tables when it ships):

| Declared (`registry()`) | Hand-SQL until toolu-orm can express it |
| --- | --- |
| `bandit_arms`, `code_symbols`, `code_fts`, `code_vec`, `document_fts`, `documents`, `edge_fts`, `feedback`, `feedback_events`, `memories`, `memory_fts`, `memory_vec`, `repo_marker`, `retrieval_log`, `schema_meta`, `source_files`, `source_roots`, `sync_binding`, `sync_state` | `memory_tags`, `indexed_files`, `code_feedback`, `query_expansions`, `document_chunks`, `edges`, `code_ref` (composite primary key [#65](https://github.com/Falconiere/toolu-orm/issues/65)); `index_failures`, `sync_log` (`AUTOINCREMENT`, #65); `eval_runs`, `index_runs`, `gc_runs` (`DESC` index column [#70](https://github.com/Falconiere/toolu-orm/issues/70), fixed upstream after 0.5.0) |

`code_symbols` and `source_files` declare their table-level `UNIQUE (…)`
as `#[unique_index]` — the same uniqueness, a named index instead of an
autoindex; the fidelity test compares unique column-sets, so both forms
satisfy it. `memories`, `feedback_events`, `source_roots` and `source_files`
carry their `CHECK` constraints as `#[column(check = "…")]` and `memories`
its soft-delete partial indexes as `#[index(…, where = "…")]`; the fidelity
test compares normalized `CHECK` clauses and index predicates too.

When an upstream fix is released, declaring a hand-SQL table is: write its struct
in the matching `schema_*.rs`, add it to `registry()` and `DECLARED_TABLES`,
run `just migration-adopt` (so the newest snapshot describes it and the next
generate does not emit a `CREATE`), and let `schema_fidelity_*` prove the
struct matches the live DDL.

## Authoring a migration

1. **Declared table:** edit the struct, then

   ```bash
   just migration add_probe        # → migrations/0017_add_probe.sql + snapshot + journal entry
   ```

   Read the generated SQL. `no schema change` means the registry already
   matches the snapshot.

   **Hand-SQL table:** write `migrations/0017_add_probe.sql` yourself, then

   ```bash
   just migration-journal migrations/0017_add_probe.sql
   ```

   A name already in the journal is refused (exit 64), so numbering cannot
   collide silently.

2. **Wire it** — append the entry to `store::migrate::list::MIGRATIONS`
   (`key`, `sql: include_str!("../../migrations/0017_add_probe.sql")`,
   `class`, `post`, `markers`), bump `CURRENT_VERSION` in
   `src/store/migrate.rs`, and add the row to `migrations/README.md`.

3. **Prove it** — these fail until everything agrees:

   ```bash
   cargo nextest run --all-features -E 'test(migration_integrity) or test(schema_fidelity) or test(schema_drift) or test(upgrade_matrix)'
   bash scripts/migration-check.sh
   ```

   `migration_integrity_journal_matches_migrations` checks every journal
   entry's hash against the bytes `include_str!` baked in;
   `schema_drift_registry_matches_shipped_snapshot` runs `run_generate` over
   a copy of the shipped journal and expects nothing; `schema_fidelity_*`
   renders the registry onto a second connection and compares it, table by
   table, with the database the frozen chain builds.

## Rules that did not change

- Shipped migrations are immutable. `scripts/migration-check.sh` compares
  every released `migrations/*.sql` against its first release tag (under
  `migrations/` or its pre-v0.29 location under `src/store/`, by basename).
  Append; never edit.
- Every new migration also carries its `Class` (`Additive` / `Destructive`)
  and the `schema_meta` markers it writes, exactly as before — the preflight
  snapshot policy and the forward-compat refusal read those.
- The `memory_vec` / `code_vec` dims (1024 / 768) are in the vec0 DDL and in
  the `#[vec0_table]` structs; change both or the fidelity test fails.
