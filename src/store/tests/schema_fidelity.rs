#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Fidelity of the declared schema (`store::schema::registry`) against the
//! database the frozen `migrations/*.sql` chain actually builds.
//!
//! Two real SQLite catalogs, no snapshot in between: the LIVE side is a
//! fresh database opened through `store::connection::open` (preflight +
//! the whole migration chain); the RENDERED side is an in-memory connection
//! carrying the same `identifier` tokenizer, fed the DDL toolu-orm generates
//! from the registry (`diff(Snapshot::empty(), registry)` →
//! `generate_sql_for`). `sqlite-vec` is a process-global auto-extension the
//! first `open` registers, so the second connection gets `vec0` for free.
//!
//! Compared per declared table: `pragma_table_info` rows in cid order
//! (name, declared type, notnull, raw `dflt_value` — SQLite stores the
//! expression text inside `DEFAULT (…)`, so `(0)` and `0` read back alike
//! — and pk ordinal), the set of unique column-lists (a table-level
//! `UNIQUE (…)` and a `CREATE UNIQUE INDEX` both surface here, which is how
//! `code_symbols`' and `source_files`' constraints are declared), the
//! non-unique named indexes with their partial-index predicate, the
//! `CHECK` clauses lifted out of `sqlite_master.sql` (comments stripped,
//! whitespace/case/quotes normalized), and — for virtual tables — the
//! normalized module arguments.
//!
//! All fns are prefixed `schema_fidelity_` so
//! `cargo nextest run -E 'test(schema_fidelity)'` selects exactly this suite.

use std::collections::{BTreeMap, BTreeSet};

use comemory::store::connection;
use comemory::store::schema::{DECLARED_TABLES, registry};
use comemory::store::tokenizer::ffi;
use rusqlite::Connection;
use tempfile::tempdir;
use toolu_orm::core::dialect::Dialect;
use toolu_orm::core::diff::diff;
use toolu_orm::core::snapshot::Snapshot;
use toolu_orm::core::sql::generate_sql_for;

/// The database the shipped migration chain builds.
fn live_db() -> (tempfile::TempDir, Connection) {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open live db");
    (dir, conn)
}

/// The database the declared schema renders to, on a connection that has
/// the `identifier` tokenizer (per-connection state) — `vec0` is already
/// process-global once [`live_db`] ran.
fn rendered_db() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db");
    ffi::register(&conn).expect("register identifier tokenizer");
    let ops = diff(&Snapshot::empty(), &registry()).expect("diff against empty snapshot");
    let sql = generate_sql_for(&ops, Dialect::Sqlite);
    conn.execute_batch(&sql)
        .unwrap_or_else(|e| panic!("rendered DDL must apply: {e}\n{sql}"));
    conn
}

/// One `pragma_table_info` row.
#[derive(Debug, PartialEq, Eq)]
struct Column {
    name: String,
    declared_type: String,
    not_null: bool,
    default: Option<String>,
    pk: i64,
}

fn table_info(conn: &Connection, table: &str) -> Vec<Column> {
    let mut stmt = conn
        .prepare("SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1)")
        .unwrap();
    stmt.query_map([table], |r| {
        Ok(Column {
            name: r.get(0)?,
            declared_type: r.get::<_, String>(1)?.to_ascii_uppercase(),
            not_null: r.get::<_, i64>(2)? != 0,
            default: r.get(3)?,
            pk: r.get(4)?,
        })
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

/// `(index name, unique, origin)` for every index on `table`.
fn index_list(conn: &Connection, table: &str) -> Vec<(String, bool, String)> {
    let mut stmt = conn
        .prepare("SELECT name, \"unique\", origin FROM pragma_index_list(?1)")
        .unwrap();
    stmt.query_map([table], |r| {
        Ok((r.get(0)?, r.get::<_, i64>(1)? != 0, r.get(2)?))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

/// Key columns of `index` in position order, each suffixed ` desc` when the
/// index sorts it descending (`pragma_index_xinfo.desc`).
fn index_columns(conn: &Connection, index: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name, \"desc\" FROM pragma_index_xinfo(?1) WHERE key = 1 ORDER BY seqno")
        .unwrap();
    stmt.query_map([index], |r| {
        let name: String = r.get(0)?;
        let desc: i64 = r.get(1)?;
        Ok(if desc == 1 {
            format!("{name} desc")
        } else {
            name
        })
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

/// Every unique column-list on `table`, whatever declared it (primary-key
/// autoindex, `UNIQUE` constraint autoindex, or a `CREATE UNIQUE INDEX`).
fn unique_sets(conn: &Connection, table: &str) -> BTreeSet<Vec<String>> {
    index_list(conn, table)
        .into_iter()
        .filter(|(_, unique, _)| *unique)
        .map(|(name, _, _)| index_columns(conn, &name))
        .collect()
}

/// Non-unique `CREATE INDEX` indexes by name: their columns and, for a
/// partial index, the normalized `WHERE` predicate.
fn named_indexes(
    conn: &Connection,
    table: &str,
) -> BTreeMap<String, (Vec<String>, Option<String>)> {
    index_list(conn, table)
        .into_iter()
        .filter(|(_, unique, origin)| !*unique && origin == "c")
        .map(|(name, _, _)| {
            let cols = index_columns(conn, &name);
            let predicate = index_predicate(conn, &name);
            (name, (cols, predicate))
        })
        .collect()
}

/// The normalized text after `WHERE` in an index's `CREATE INDEX`, or
/// `None` for a full index.
fn index_predicate(conn: &Connection, index: &str) -> Option<String> {
    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
            [index],
            |r| r.get(0),
        )
        .unwrap_or_else(|e| panic!("{index} must exist: {e}"));
    let upper = sql.to_ascii_uppercase();
    let at = upper.find(" WHERE ")?;
    Some(normalize_sql(&sql[at + " WHERE ".len()..]))
}

/// Lower-case, unquote, collapse whitespace, and strip the spaces around
/// `(`, `)` and `,` — the shape both hand-written DDL and toolu-orm's
/// renderer reduce to.
fn normalize_sql(text: &str) -> String {
    let lowered = text.to_ascii_lowercase().replace('"', "");
    let collapsed = lowered.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .replace(" (", "(")
        .replace("( ", "(")
        .replace(" )", ")")
        .replace(" ,", ",")
        .replace(", ", ",")
}

/// Every `CHECK (…)` clause in `table`'s `CREATE TABLE`, normalized and
/// sorted. `--` comments are dropped first (the hand-written DDL carries
/// them; toolu-orm's does not), and each clause is the balanced-paren group
/// after the keyword so an `IN (…)` list inside it survives intact.
fn check_clauses(conn: &Connection, table: &str) -> Vec<String> {
    let sql = create_sql(conn, table);
    let uncommented: String = sql
        .lines()
        .map(|line| line.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let upper = uncommented.to_ascii_uppercase();
    let mut clauses = Vec::new();
    let mut from = 0;
    while let Some(rel) = upper[from..].find("CHECK") {
        let start = from + rel;
        let open = start + upper[start..].find('(').expect("CHECK is followed by (");
        let mut depth = 0usize;
        let mut end = open;
        for (i, ch) in uncommented[open..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        clauses.push(normalize_sql(&uncommented[open..=end]));
        from = end;
    }
    clauses.sort();
    clauses
}

fn create_sql(conn: &Connection, table: &str) -> String {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |r| r.get(0),
    )
    .unwrap_or_else(|e| panic!("{table} must exist: {e}"))
}

fn is_virtual(conn: &Connection, table: &str) -> bool {
    create_sql(conn, table)
        .to_ascii_uppercase()
        .starts_with("CREATE VIRTUAL TABLE")
}

/// The module and its arguments after `USING`, lower-cased, unquoted, with
/// whitespace collapsed and stripped around `(`, `)` and `,` — the shape
/// both the hand-written DDL and toolu-orm's renderer reduce to.
fn module_args(conn: &Connection, table: &str) -> String {
    let sql = create_sql(conn, table);
    let (_, after) = sql
        .split_once("USING")
        .unwrap_or_else(|| panic!("{table} is virtual but has no USING clause: {sql}"));
    normalize_sql(after)
}

#[test]
fn schema_fidelity_registry_names_exactly_the_declared_tables() {
    let reg = registry();
    let names: Vec<&str> = reg.tables().iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names, DECLARED_TABLES,
        "registry() and DECLARED_TABLES must agree"
    );
    let sorted: BTreeSet<&str> = DECLARED_TABLES.iter().copied().collect();
    assert_eq!(
        sorted.len(),
        DECLARED_TABLES.len(),
        "DECLARED_TABLES has a duplicate"
    );
    assert!(
        DECLARED_TABLES.windows(2).all(|w| w[0] < w[1]),
        "DECLARED_TABLES must be sorted"
    );
}

#[test]
fn schema_fidelity_ordinary_tables_match_the_live_database() {
    let (_dir, live) = live_db();
    let rendered = rendered_db();
    for &table in DECLARED_TABLES {
        if is_virtual(&live, table) {
            continue; // covered by the virtual-table test below
        }
        let live_columns = table_info(&live, table);
        assert!(
            !live_columns.is_empty(),
            "{table} must exist on the live side"
        );
        assert_eq!(
            table_info(&rendered, table),
            live_columns,
            "{table}: pragma_table_info differs (rendered vs live)"
        );
        assert_eq!(
            unique_sets(&rendered, table),
            unique_sets(&live, table),
            "{table}: unique column-sets differ (rendered vs live)"
        );
        assert_eq!(
            named_indexes(&rendered, table),
            named_indexes(&live, table),
            "{table}: non-unique indexes (columns, WHERE predicate) differ (rendered vs live)"
        );
        assert_eq!(
            check_clauses(&rendered, table),
            check_clauses(&live, table),
            "{table}: CHECK clauses differ (rendered vs live)"
        );
    }
}

#[test]
fn schema_fidelity_check_and_partial_index_coverage_is_not_vacuous() {
    let (_dir, live) = live_db();
    assert_eq!(
        check_clauses(&live, "memories").len(),
        2,
        "memories has two CHECK clauses"
    );
    assert_eq!(
        check_clauses(&live, "source_files").len(),
        2,
        "source_files has two CHECK clauses"
    );
    assert_eq!(
        check_clauses(&live, "feedback").len(),
        0,
        "feedback has none"
    );
    let memories = named_indexes(&live, "memories");
    assert_eq!(
        memories
            .get("idx_memories_repo")
            .and_then(|(_, p)| p.clone())
            .as_deref(),
        Some("deleted_at is null"),
        "the soft-delete predicate is read back"
    );
    assert_eq!(
        memories.get("idx_memories_updated").map(|(_, p)| p.clone()),
        Some(None),
        "a full index has no predicate"
    );
    let gc_runs = named_indexes(&live, "gc_runs");
    assert_eq!(
        gc_runs.get("idx_gc_runs_at").map(|(cols, _)| cols.clone()),
        Some(vec!["at desc".to_owned()]),
        "a DESC index column is read back with its direction"
    );
}

#[test]
fn schema_fidelity_virtual_tables_match_the_live_database() {
    let (_dir, live) = live_db();
    let rendered = rendered_db();
    let virtual_tables: Vec<&str> = DECLARED_TABLES
        .iter()
        .copied()
        .filter(|t| is_virtual(&live, t))
        .collect();
    assert_eq!(
        virtual_tables,
        [
            "code_fts",
            "code_vec",
            "document_fts",
            "edge_fts",
            "memory_fts",
            "memory_vec"
        ],
        "the declared virtual tables"
    );
    for table in virtual_tables {
        assert!(
            is_virtual(&rendered, table),
            "{table} must render as a virtual table"
        );
        assert_eq!(
            module_args(&rendered, table),
            module_args(&live, table),
            "{table}: module arguments differ (rendered vs live)"
        );
        // The module arguments already spell out every column, flag and
        // dim; `pragma_table_info` is the module's own reading of them, so
        // compare that too (FTS5 reports untyped columns, vec0 typed ones).
        let live_columns = table_info(&live, table);
        assert!(
            !live_columns.is_empty(),
            "{table} must expose columns on the live side"
        );
        assert_eq!(
            table_info(&rendered, table),
            live_columns,
            "{table}: pragma_table_info differs (rendered vs live)"
        );
    }
}
