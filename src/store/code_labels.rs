//! Local public repository metadata for canonical-root code identities.

use crate::prelude::*;
use crate::store::schema_meta;
use rusqlite::{Connection, params};

const PREFIX: &str = "code_source_label:";

/// Attach optional public metadata without changing the source's durable identity.
pub fn set(conn: &Connection, identity: &str, label: Option<&str>) -> Result<()> {
    let key = format!("{PREFIX}{identity}");
    match label {
        Some(label) => schema_meta::upsert(conn, &key, label),
        None => schema_meta::delete(conn, &key),
    }
}

/// Match either an exact identity or every source carrying the public label.
pub fn identities(conn: &Connection, label: &str) -> Result<Vec<String>> {
    let mut statement = conn.prepare("SELECT substr(key, ?1) FROM schema_meta WHERE key LIKE 'code_source_label:%' AND value=?2 ORDER BY key")?;
    let rows = statement.query_map(params![(PREFIX.len() + 1) as i64, label], |row| row.get(0))?;
    let mut identities = rows.collect::<std::result::Result<Vec<String>, _>>()?;
    if !identities.iter().any(|identity| identity == label) {
        identities.push(label.to_string());
    }
    Ok(identities)
}
