//! Derived document chunk vectors, scoped by provider identity.

use rusqlite::{Connection, params};

use crate::prelude::*;
use crate::store::embed;

/// Document vector width, independent of the memory model.
pub const DIM: usize = 1024;

/// Start a new source generation, invalidating concurrent older publishers.
pub fn begin(conn: &Connection, source: &str, model: &str) -> Result<String> {
    Ok(conn.query_row(
        "INSERT INTO document_vector_source(source_id,model,generation) VALUES(?1,?2,lower(hex(randomblob(16)))) \
         ON CONFLICT(source_id) DO UPDATE SET model=excluded.model,generation=excluded.generation RETURNING generation",
        params![source, model], |row| row.get(0),
    )?)
}

/// A chunk awaiting a vector for the selected provider.
pub struct PendingChunk {
    /// Owning document.
    pub document_id: String,
    /// Position within the document.
    pub ordinal: i64,
    /// Source revision used for the late-result guard.
    pub revision: String,
    /// Exact inference input.
    pub text: String,
    /// Canonical source root.
    pub root: String,
    /// Source-relative file path.
    pub path: String,
    /// Whether the root itself names a file.
    pub file_root: bool,
}

/// Return source chunks lacking compatible vectors, or every chunk for a full refresh.
pub fn pending(
    conn: &Connection,
    source: &str,
    model: &str,
    full: bool,
) -> Result<Vec<PendingChunk>> {
    let mut stmt = conn.prepare(
        "SELECT c.document_id, c.ordinal, d.revision_hash, c.text, s.canonical_path, f.relative_path, s.kind FROM document_chunks c \
         JOIN documents d ON d.id=c.document_id JOIN source_files f ON f.id=d.source_file_id \
         JOIN source_roots s ON s.id=f.source_id \
         LEFT JOIN document_vectors v ON v.document_id=c.document_id AND v.ordinal=c.ordinal \
         WHERE f.source_id=?1 AND (?3 OR v.model IS NULL OR v.model != ?2) ORDER BY c.document_id,c.ordinal",
    )?;
    let rows = stmt.query_map(params![source, model, full], |r| {
        Ok(PendingChunk {
            document_id: r.get(0)?,
            ordinal: r.get(1)?,
            revision: r.get(2)?,
            text: r.get(3)?,
            root: r.get(4)?,
            path: r.get(5)?,
            file_root: r.get::<_, String>(6)? == "file",
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Publish while revision and text match, checking the source generation when supplied.
pub fn publish(
    conn: &Connection,
    chunk: &PendingChunk,
    model: &str,
    vector: &[f32],
    generation: Option<&str>,
) -> Result<bool> {
    embed::guard_dim(vector, DIM)?;
    let n = conn.execute(
        "INSERT INTO document_vectors(document_id, ordinal, model, embedding) \
         SELECT c.document_id,c.ordinal,?4,?5 FROM document_chunks c \
         JOIN documents d ON d.id=c.document_id \
         WHERE c.document_id=?1 AND c.ordinal=?2 AND d.revision_hash=?3 AND c.text=?6 \
         AND (?7 IS NULL OR EXISTS (SELECT 1 FROM source_files f JOIN document_vector_source s ON s.source_id=f.source_id WHERE f.id=d.source_file_id AND s.model=?4 AND s.generation=?7)) \
         ON CONFLICT(document_id,ordinal) DO UPDATE SET model=excluded.model, embedding=excluded.embedding",
        params![chunk.document_id, chunk.ordinal, chunk.revision, model, embed::to_vec_blob(vector), chunk.text, generation],
    )?;
    Ok(n != 0)
}

/// Search only vectors made by the matching provider, ordered by cosine distance.
pub fn search(
    conn: &Connection,
    model: &str,
    vector: &[f32],
    k: usize,
) -> Result<Vec<(String, i64, f32)>> {
    embed::guard_dim(vector, DIM)?;
    let mut stmt = conn.prepare(
        "SELECT document_id, ordinal, vec_distance_cosine(embedding, ?2) AS distance \
         FROM document_vectors WHERE model=?1 ORDER BY distance, document_id, ordinal LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        params![
            model,
            embed::to_vec_blob(vector),
            i64::try_from(k).unwrap_or(i64::MAX)
        ],
        |r| Ok((r.get(0)?, r.get(1)?, 1.0_f32 - r.get::<_, f32>(2)?)),
    )?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[cfg(test)]
#[path = "tests/document_vectors.rs"]
mod tests;
