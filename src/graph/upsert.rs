//! kuzu-backed [`Graph`] handle: opens (and lazily migrates) the memory-layer
//! schema, then exposes idempotent `MERGE`-based upserts for memories and their
//! cross-memory relations.
//!
//! Every interpolated identifier is passed through [`esc`] so single quotes in
//! user-controlled strings (ids, tags, repo names, authors) cannot break out of
//! the surrounding Cypher string literal.

use std::path::Path;

use kuzu::{Connection, Database, SystemConfig};
use time::format_description::well_known::Iso8601;
use time::OffsetDateTime;

use crate::graph::schema::MEMORY_LAYER_DDL;
use crate::memory::{Kind, MemoryRecord};
use crate::prelude::*;

/// Long-lived kuzu database handle. Cheap to share by reference; create a fresh
/// [`Connection`] per call via [`Graph::conn`].
pub struct Graph {
    db: Database,
}

impl Graph {
    /// Open (or create) a kuzu database at `dir` and ensure the memory-layer DDL
    /// is applied. Replaying the DDL on every open is safe because every
    /// statement uses `IF NOT EXISTS`.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        std::fs::create_dir_all(dir.as_ref())?;
        let db = Database::new(dir.as_ref(), SystemConfig::default())
            .map_err(|e| Error::Other(format!("kuzu open: {e}")))?;
        {
            let conn =
                Connection::new(&db).map_err(|e| Error::Other(format!("kuzu connect: {e}")))?;
            for ddl in MEMORY_LAYER_DDL {
                conn.query(ddl)
                    .map_err(|e| Error::Other(format!("kuzu ddl '{ddl}': {e}")))?;
            }
        }
        Ok(Self { db })
    }

    /// Build a new [`Connection`] borrowing this graph. kuzu connections are
    /// lightweight (no socket / handshake) so per-operation construction is
    /// preferred over sharing one connection across threads.
    pub fn conn(&self) -> Result<Connection<'_>> {
        Connection::new(&self.db).map_err(|e| Error::Other(format!("kuzu connect: {e}")))
    }

    /// Upsert the `Memory` node and its `InRepo` / `AuthoredBy` / `Tagged`
    /// provenance edges. Safe to call repeatedly for the same record — every
    /// statement uses `MERGE`.
    pub fn upsert_memory(&self, rec: &MemoryRecord) -> Result<()> {
        let conn = self.conn()?;
        let fm = &rec.frontmatter;
        let created = fm
            .created
            .format(&Iso8601::DEFAULT)
            .map_err(|e| Error::Other(format!("iso8601 format: {e}")))?;

        run(
      &conn,
      &format!(
        "MERGE (m:Memory {{id: '{id}'}}) SET m.kind = '{kind}', m.created = '{created}', m.quality = {quality}",
        id = esc(&fm.id),
        kind = kind_str(fm.kind),
        created = esc(&created),
        quality = fm.quality as i64,
      ),
    )?;

        if !fm.repo.is_empty() {
            run(
                &conn,
                &format!("MERGE (:Repo {{name: '{}'}})", esc(&fm.repo)),
            )?;
            run(
                &conn,
                &format!(
          "MATCH (m:Memory {{id: '{}'}}), (r:Repo {{name: '{}'}}) MERGE (m)-[:InRepo]->(r)",
          esc(&fm.id),
          esc(&fm.repo),
        ),
            )?;
        }

        if !fm.author.is_empty() {
            run(
                &conn,
                &format!("MERGE (:Author {{name: '{}'}})", esc(&fm.author)),
            )?;
            run(
                &conn,
                &format!(
          "MATCH (m:Memory {{id: '{}'}}), (a:Author {{name: '{}'}}) MERGE (m)-[:AuthoredBy]->(a)",
          esc(&fm.id),
          esc(&fm.author),
        ),
            )?;
        }

        for tag in &fm.tags {
            run(&conn, &format!("MERGE (:Tag {{name: '{}'}})", esc(tag)))?;
            run(
                &conn,
                &format!(
          "MATCH (m:Memory {{id: '{}'}}), (t:Tag {{name: '{}'}}) MERGE (m)-[:Tagged]->(t)",
          esc(&fm.id),
          esc(tag),
        ),
            )?;
        }

        Ok(())
    }

    /// Record that `new_id` supersedes `old_id`. Stores the UTC timestamp on the
    /// edge so the retrieval pipeline can prefer the most recent decision.
    pub fn add_supersedes(&self, new_id: &str, old_id: &str) -> Result<()> {
        let now = OffsetDateTime::now_utc()
            .format(&Iso8601::DEFAULT)
            .map_err(|e| Error::Other(format!("iso8601 format: {e}")))?;
        let conn = self.conn()?;
        run(
      &conn,
      &format!(
        "MATCH (n:Memory {{id: '{n}'}}), (o:Memory {{id: '{o}'}}) MERGE (n)-[:Supersedes {{at: '{now}'}}]->(o)",
        n = esc(new_id),
        o = esc(old_id),
        now = esc(&now),
      ),
    )?;
        Ok(())
    }

    /// Record a weighted relatedness edge between two memories. Used to wire up
    /// vector-similar neighbours so graph traversal can fan out to them.
    pub fn add_relates_to(&self, a: &str, b: &str, score: f64) -> Result<()> {
        let conn = self.conn()?;
        run(
      &conn,
      &format!(
        "MATCH (x:Memory {{id: '{a}'}}), (y:Memory {{id: '{b}'}}) MERGE (x)-[:RelatesTo {{score: {s}}}]->(y)",
        a = esc(a),
        b = esc(b),
        s = score,
      ),
    )?;
        Ok(())
    }
}

/// Lowercase string label used in YAML frontmatter, mirrored on `Memory.kind`.
fn kind_str(k: Kind) -> &'static str {
    match k {
        Kind::Decision => "decision",
        Kind::Bug => "bug",
        Kind::Convention => "convention",
        Kind::Discovery => "discovery",
        Kind::Pattern => "pattern",
        Kind::Note => "note",
    }
}

/// Escape a string for inclusion inside a single-quoted Cypher literal.
///
/// Only backslash and apostrophe need handling — kuzu accepts the resulting
/// `\\` and `\'` sequences inside string literals.
pub(crate) fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            _ => out.push(ch),
        }
    }
    out
}

fn run(conn: &Connection<'_>, cypher: &str) -> Result<()> {
    conn.query(cypher)
        .map_err(|e| Error::Other(format!("kuzu query failed: {e}")))?;
    Ok(())
}
