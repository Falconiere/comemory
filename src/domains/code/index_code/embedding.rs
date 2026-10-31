//! Source-scoped embedding after the lexical index transaction commits.

use std::path::Path;

use crate::prelude::*;
use crate::store::{Connection, code_embedding, embed as store_embed, vector};
use crate::utilities::{embed, progress::ProgressSink};

/// Bind managed vectors to the command, hint and parent-input version.
pub(super) fn provider_id(cmd: &str, hint: Option<&str>) -> String {
    crate::utilities::digest::sha256_hex(
        format!("code-parent-raw-v1\0{cmd}\0{}", hint.unwrap_or_default()).as_bytes(),
    )
}

/// Refresh eligible parent vectors outside the lexical transaction.
pub(super) fn run(
    conn: &mut Connection,
    repo: &str,
    root: &Path,
    cmd: &str,
    model: &str,
    sink: Option<&dyn ProgressSink>,
    full: bool,
) -> Result<(usize, usize)> {
    let (key, generation) = code_embedding::select_model(conn, repo, model)?;
    let pending = code_embedding::pending(conn, repo, full)?;
    let dim = vector::dim_code(conn)?;
    let mut embedded = 0;
    let mut failed = 0;
    for row in pending {
        if sink.is_some_and(ProgressSink::is_cancelled) {
            return Err(Error::Cancelled);
        }
        let result = embed::embed_query_cancellable(cmd, &row.snippet, sink).and_then(|v| {
            store_embed::guard_dim(&v, dim)?;
            Ok(v)
        });
        let embedding = match result {
            Ok(v) => v,
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => {
                tracing::warn!(error = %e, "code embedding failed; retry indexing to repair coverage");
                failed += 1;
                continue;
            }
        };
        if sink.is_some_and(ProgressSink::is_cancelled) {
            return Err(Error::Cancelled);
        }
        let current = std::fs::read(root.join(&row.path))
            .ok()
            .and_then(|bytes| git2::Oid::hash_object(git2::ObjectType::Blob, &bytes).ok());
        if current.is_some_and(|oid| oid.to_string() == row.oid)
            && code_embedding::publish(conn, repo, &row, &key, &generation, model, &embedding)?
        {
            embedded += 1;
        } else {
            failed += 1;
        }
    }
    Ok((embedded, failed))
}
