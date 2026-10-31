//! Store operations for source-scoped code embedding.

use rusqlite::{Connection, params};
use toolu_orm::core::query_column::CommonOps;

use super::{
    orm,
    schema_code::{CodeSymbols, code_symbols as symbol, code_vec as vec},
    schema_embeddings::code_vector_provenance as provenance,
};
use crate::prelude::*;
use crate::store::{schema_meta, vector};

/// A parent symbol awaiting an embedding.
pub(crate) struct Pending {
    pub id: i64,
    pub path: String,
    pub oid: String,
    pub snippet: String,
}

/// A vector whose symbol identity and text can survive re-extraction.
pub(crate) struct Preserved {
    symbol: String,
    kind: String,
    start: i64,
    end: i64,
    snippet: String,
    parent: bool,
    vector: Vec<u8>,
    model: Option<String>,
}

orm::reads! {
    /// Capture existing vectors before replacing a file's symbols.
    pub(crate) capture(repo: &str, path: &str) -> Vec<Preserved> = query_all(
        CodeSymbols::select().columns_typed(&[])
            .column_expr(&symbol::symbol.qualified(), "symbol")
            .column_expr(&symbol::kind.qualified(), "kind")
            .column_expr(&symbol::line_start.qualified(), "line_start")
            .column_expr(&symbol::line_end.qualified(), "line_end")
            .column_expr(&symbol::snippet.qualified(), "snippet")
            .column_expr(&vec::embedding.qualified(), "embedding")
            .column_expr(&provenance::model.qualified(), "model")
            .column_expr(&format!("{} IS NULL", symbol::parent_id.qualified()), "is_parent")
            .join("code_vec", vec::symbol_id.equals(&symbol::id))
            .left_join("code_vector_provenance", provenance::symbol_id.equals(&symbol::id))
            .filter(symbol::repo.eq(repo)).filter(symbol::path.eq(path)).to_sql(),
        |r| {
        Ok(Preserved {
            symbol: r.get(0)?,
            kind: r.get(1)?,
            start: r.get(2)?,
            end: r.get(3)?,
            snippet: r.get(4)?,
            vector: r.get(5)?,
            model: r.get(6)?,
            parent: r.get(7)?,
        })
        }
    );
}

/// Restore vectors only when the exact symbol identity and text survive.
pub(crate) fn restore(
    conn: &Connection,
    repo: &str,
    path: &str,
    saved: &[Preserved],
) -> Result<()> {
    for row in saved {
        let mut stmt = conn.prepare("SELECT id FROM code_symbols WHERE repo = ?1 AND path = ?2 AND symbol = ?3 AND kind = ?4 AND line_start = ?5 AND line_end = ?6 AND snippet = ?7 AND (parent_id IS NULL) = ?8")?;
        let mut matches = stmt.query(params![
            repo,
            path,
            row.symbol,
            row.kind,
            row.start,
            row.end,
            row.snippet,
            row.parent
        ])?;
        if let Some(found) = matches.next()? {
            let id: i64 = found.get(0)?;
            conn.execute(
                "INSERT INTO code_vec(symbol_id, embedding) VALUES (?1, ?2)",
                params![id, row.vector],
            )?;
            if let Some(model) = &row.model {
                conn.execute(
                    "INSERT INTO code_vector_provenance(symbol_id, model) VALUES (?1, ?2)",
                    params![id, model],
                )?;
            }
        }
    }
    Ok(())
}

/// Select a model generation without discarding existing BYO vectors.
pub(crate) fn select_model(
    conn: &mut Connection,
    repo: &str,
    model: &str,
) -> Result<(String, String)> {
    let key = format!("code_vector_model:{repo}");
    let generation_key = format!("code_vector_generation:{repo}");
    let tx = conn.transaction()?;
    if schema_meta::get(&tx, &key)?.as_deref() != Some(model) {
        tx.execute("DELETE FROM code_vec WHERE symbol_id IN (SELECT p.symbol_id FROM code_vector_provenance p JOIN code_symbols s ON s.id = p.symbol_id WHERE s.repo = ?1)", [repo])?;
        tx.execute("DELETE FROM code_vector_provenance WHERE symbol_id IN (SELECT id FROM code_symbols WHERE repo = ?1)", [repo])?;
        schema_meta::upsert(&tx, &key, model)?;
        schema_meta::upsert(
            &tx,
            &generation_key,
            &crate::store::random_id::random_hex(16)?,
        )?;
    }
    let generation = schema_meta::get(&tx, &generation_key)?
        .ok_or_else(|| Error::Other("code model generation missing".into()))?;
    tx.commit()?;
    Ok((generation_key, generation))
}

/// Read missing parent vectors and, on full runs, provider-managed vectors.
pub(crate) fn pending(conn: &Connection, repo: &str, full: bool) -> Result<Vec<Pending>> {
    let mut stmt = conn.prepare("SELECT id, path, blob_oid, snippet FROM code_symbols WHERE repo = ?1 AND parent_id IS NULL AND (id NOT IN (SELECT symbol_id FROM code_vec) OR (?2 AND id IN (SELECT symbol_id FROM code_vector_provenance))) ORDER BY id")?;
    let rows = stmt.query_map(params![repo, full], |r| {
        Ok(Pending {
            id: r.get(0)?,
            path: r.get(1)?,
            oid: r.get(2)?,
            snippet: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Publish only while the row and model generation still match the input.
pub(crate) fn publish(
    conn: &mut Connection,
    repo: &str,
    row: &Pending,
    key: &str,
    model: &str,
    provider: &str,
    embedding: &[f32],
) -> Result<bool> {
    let tx = conn.transaction()?;
    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM code_symbols WHERE id = ?1 AND repo = ?2 AND path = ?3 AND blob_oid = ?4 AND snippet = ?5 AND parent_id IS NULL)", params![row.id, repo, row.path, row.oid, row.snippet], |r| r.get(0))?;
    if !exists || schema_meta::get(&tx, key)?.as_deref() != Some(model) {
        return Ok(false);
    }
    vector::replace_code(&tx, row.id, embedding)?;
    tx.execute("INSERT INTO code_vector_provenance(symbol_id, model) VALUES (?1, ?2) ON CONFLICT(symbol_id) DO UPDATE SET model = excluded.model", params![row.id, provider])?;
    tx.commit()?;
    Ok(true)
}
