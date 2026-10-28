//! The three pulled-projection tables, always written and read per
//! generation.
//!
//! [`replace_generation`] rewrites a generation's whole projection rather than
//! merging into it — which is what makes a replayed acceptance idempotent:
//! re-applying the same generation writes the same rows instead of doubling
//! every edge weight.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_code_generation::{
    RemoteCodeEdge, RemoteCodeFile, RemoteCodeSymbol, remote_code_edge as edge_col,
    remote_code_file as file_col, remote_code_symbol as symbol_col,
};
use crate::prelude::*;

/// Emit the per-generation reads for one projection row type from an
/// `index => field` table: `COLUMNS`, the positional `read` decoder, `load`
/// (every row of a generation in `order` order) and `purge` (delete them).
/// Everything comes from the same rows, so the column list and decoder cannot
/// drift on order. `$table` / `$cols` are the table's generated builder and
/// column module; each field shares its column's name.
macro_rules! row_decoder {
    (
        $ty:ident, $table:ident, $cols:ident, order [$($ord:ident),+],
        $($idx:literal => $field:ident),+ $(,)?
    ) => {
        impl $ty {
            /// The columns [`Self::read`] expects, in order.
            const COLUMNS: &'static [&'static dyn toolu_orm::core::query_column::ColumnRef] =
                &[$(&$cols::$field),+];

            /// One row of [`Self::COLUMNS`].
            fn read(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
                Ok(Self { $($field: r.get($idx)?),+ })
            }

            /// Every row of `(repo, generation_id)`, in a stable order.
            fn load(conn: &Connection, repo: &str, generation_id: &str) -> Result<Vec<Self>> {
                let query = $table::select()
                    .columns_typed(Self::COLUMNS)
                    .filter($cols::repo.eq(repo))
                    .filter($cols::generation_id.eq(generation_id));
                $(let query = query.order_by($cols::$ord.asc());)+
                orm::query_all(conn, query.to_sql(), Self::read)
            }

            /// Delete every row of `(repo, generation_id)`.
            fn purge(tx: &Connection, repo: &str, generation_id: &str) -> Result<()> {
                orm::execute(
                    tx,
                    $table::delete()
                        .filter($cols::repo.eq(repo))
                        .filter($cols::generation_id.eq(generation_id))
                        .to_sql(),
                )?;
                Ok(())
            }
        }
    };
}

/// One file of a pulled manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// Path relative to the repo root.
    pub path: String,
    /// Git blob OID the sender indexed it at.
    pub blob_oid: String,
}

row_decoder!(
    File,
    RemoteCodeFile,
    file_col,
    order [path],
    0 => path, 1 => blob_oid);
/// One snippet-free symbol of a pulled generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// Path relative to the repo root.
    pub path: String,
    /// Qualified symbol name.
    pub symbol: String,
    /// `function` / `struct` / ….
    pub kind: String,
    /// `rust` / `typescript` / ….
    pub lang: String,
    /// First line (1-based).
    pub line_start: i64,
    /// Last line (1-based, inclusive).
    pub line_end: i64,
}

row_decoder!(
    Symbol,
    RemoteCodeSymbol,
    symbol_col,
    order [path, line_start],
    0 => path,
    1 => symbol,
    2 => kind,
    3 => lang,
    4 => line_start,
    5 => line_end,
);
/// One edge of a pulled generation's graph projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// `imports` or `co_changed`.
    pub rel: String,
    /// Source file path.
    pub src_path: String,
    /// Target file path.
    pub dst_path: String,
    /// Co-change count, or `1` for a resolved import.
    pub weight: i64,
    /// The revision the edge was derived at.
    pub anchor: Option<String>,
}

row_decoder!(
    Edge,
    RemoteCodeEdge,
    edge_col,
    order [rel, src_path],
    0 => rel,
    1 => src_path,
    2 => dst_path,
    3 => weight,
    4 => anchor,
);
/// A whole pulled projection, as one generation carries it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Projection {
    /// The manifest.
    pub files: Vec<File>,
    /// Every symbol, snippet-free.
    pub symbols: Vec<Symbol>,
    /// Every import and co-change edge.
    pub edges: Vec<Edge>,
}

/// Replace everything `(repo, generation_id)` holds with `projection`, in the
/// caller's transaction.
///
/// # Errors
/// Propagates SQLite failures.
pub fn replace_generation(
    tx: &Connection,
    repo: &str,
    generation_id: &str,
    projection: &Projection,
) -> Result<()> {
    purge_generation(tx, repo, generation_id)?;
    for file in &projection.files {
        orm::execute(
            tx,
            RemoteCodeFile::insert()
                .set(&file_col::repo, repo)
                .set(&file_col::generation_id, generation_id)
                .set(&file_col::path, file.path.as_str())
                .set(&file_col::blob_oid, file.blob_oid.as_str())
                .to_sql(),
        )?;
    }
    for symbol in &projection.symbols {
        orm::execute(
            tx,
            RemoteCodeSymbol::insert()
                .set(&symbol_col::repo, repo)
                .set(&symbol_col::generation_id, generation_id)
                .set(&symbol_col::path, symbol.path.as_str())
                .set(&symbol_col::symbol, symbol.symbol.as_str())
                .set(&symbol_col::kind, symbol.kind.as_str())
                .set(&symbol_col::lang, symbol.lang.as_str())
                .set(&symbol_col::line_start, symbol.line_start)
                .set(&symbol_col::line_end, symbol.line_end)
                .to_sql(),
        )?;
    }
    for edge in &projection.edges {
        orm::execute(
            tx,
            RemoteCodeEdge::insert()
                .set(&edge_col::repo, repo)
                .set(&edge_col::generation_id, generation_id)
                .set(&edge_col::rel, edge.rel.as_str())
                .set(&edge_col::src_path, edge.src_path.as_str())
                .set(&edge_col::dst_path, edge.dst_path.as_str())
                .set(&edge_col::weight, edge.weight)
                .set(&edge_col::anchor, edge.anchor.as_deref())
                .to_sql(),
        )?;
    }
    Ok(())
}

/// Remove every row of one generation's projection.
///
/// # Errors
/// Propagates SQLite failures.
pub fn purge_generation(tx: &Connection, repo: &str, generation_id: &str) -> Result<()> {
    File::purge(tx, repo, generation_id)?;
    Symbol::purge(tx, repo, generation_id)?;
    Edge::purge(tx, repo, generation_id)
}

/// Everything `(repo, generation_id)` holds, in one read.
///
/// A generation is written and read as a unit — it is the only state in which
/// its manifest, symbols and edges describe the same tree — so there is one
/// reader rather than three that could be called out of step with each other.
///
/// # Errors
/// Propagates SQLite failures.
pub fn projection(conn: &Connection, repo: &str, generation_id: &str) -> Result<Projection> {
    Ok(Projection {
        files: File::load(conn, repo, generation_id)?,
        symbols: Symbol::load(conn, repo, generation_id)?,
        edges: Edge::load(conn, repo, generation_id)?,
    })
}

#[cfg(test)]
#[path = "tests/remote_code.rs"]
mod tests;
