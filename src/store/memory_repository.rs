//! Repository-label reads for memory sync authorization, including tombstones.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::{
    orm,
    schema_memory::{Memories, memories},
};
use crate::prelude::*;

/// Read a memory's stored repository label whether it is live or soft-deleted.
pub fn label(conn: &Connection, id: &str) -> Result<Option<String>> {
    orm::query_optional(
        conn,
        Memories::select()
            .columns_typed(&[&memories::repo])
            .filter(memories::id.eq(id))
            .to_sql(),
        |row| row.get::<_, Option<String>>(0),
    )
    .map(Option::flatten)
}
