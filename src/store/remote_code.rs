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

/// One file of a pulled manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// Path relative to the repo root.
    pub path: String,
    /// Git blob OID the sender indexed it at.
    pub blob_oid: String,
}

impl File {
    /// The columns [`File::read`] expects, in order.
    const COLUMNS: [&'static dyn toolu_orm::core::query_column::ColumnRef; 2] =
        [&file_col::path, &file_col::blob_oid];

    /// One row of [`File::COLUMNS`].
    fn read(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            path: r.get(0)?,
            blob_oid: r.get(1)?,
        })
    }
}

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

impl Symbol {
    /// The columns [`Symbol::read`] expects, in order.
    const COLUMNS: [&'static dyn toolu_orm::core::query_column::ColumnRef; 6] = [
        &symbol_col::path,
        &symbol_col::symbol,
        &symbol_col::kind,
        &symbol_col::lang,
        &symbol_col::line_start,
        &symbol_col::line_end,
    ];

    /// One row of [`Symbol::COLUMNS`].
    fn read(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            path: r.get(0)?,
            symbol: r.get(1)?,
            kind: r.get(2)?,
            lang: r.get(3)?,
            line_start: r.get(4)?,
            line_end: r.get(5)?,
        })
    }
}

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

impl Edge {
    /// The columns [`Edge::read`] expects, in order.
    const COLUMNS: [&'static dyn toolu_orm::core::query_column::ColumnRef; 5] = [
        &edge_col::rel,
        &edge_col::src_path,
        &edge_col::dst_path,
        &edge_col::weight,
        &edge_col::anchor,
    ];

    /// One row of [`Edge::COLUMNS`].
    fn read(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            rel: r.get(0)?,
            src_path: r.get(1)?,
            dst_path: r.get(2)?,
            weight: r.get(3)?,
            anchor: r.get(4)?,
        })
    }
}

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
    orm::execute(
        tx,
        RemoteCodeFile::delete()
            .filter(file_col::repo.eq(repo))
            .filter(file_col::generation_id.eq(generation_id))
            .to_sql(),
    )?;
    orm::execute(
        tx,
        RemoteCodeSymbol::delete()
            .filter(symbol_col::repo.eq(repo))
            .filter(symbol_col::generation_id.eq(generation_id))
            .to_sql(),
    )?;
    orm::execute(
        tx,
        RemoteCodeEdge::delete()
            .filter(edge_col::repo.eq(repo))
            .filter(edge_col::generation_id.eq(generation_id))
            .to_sql(),
    )?;
    Ok(())
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
        files: orm::query_all(
            conn,
            RemoteCodeFile::select()
                .columns_typed(&File::COLUMNS)
                .filter(file_col::repo.eq(repo))
                .filter(file_col::generation_id.eq(generation_id))
                .order_by(file_col::path.asc())
                .to_sql(),
            File::read,
        )?,
        symbols: orm::query_all(
            conn,
            RemoteCodeSymbol::select()
                .columns_typed(&Symbol::COLUMNS)
                .filter(symbol_col::repo.eq(repo))
                .filter(symbol_col::generation_id.eq(generation_id))
                .order_by(symbol_col::path.asc())
                .order_by(symbol_col::line_start.asc())
                .to_sql(),
            Symbol::read,
        )?,
        edges: orm::query_all(
            conn,
            RemoteCodeEdge::select()
                .columns_typed(&Edge::COLUMNS)
                .filter(edge_col::repo.eq(repo))
                .filter(edge_col::generation_id.eq(generation_id))
                .order_by(edge_col::rel.asc())
                .order_by(edge_col::src_path.asc())
                .to_sql(),
            Edge::read,
        )?,
    })
}

#[cfg(test)]
#[path = "tests/remote_code.rs"]
mod tests;
