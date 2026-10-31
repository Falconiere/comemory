//! Provider identity and source-scoped document chunk embedding.

use crate::prelude::*;
use crate::store::{Connection, document_vectors};
use crate::utilities::embed;
use crate::utilities::progress::ProgressSink;

/// Identify a provider configuration without storing its command or credentials.
pub fn provider_id(cmd: &str, hint: Option<&str>) -> String {
    crate::utilities::digest::sha256_hex(
        format!("document-chunk-raw-v1\0{cmd}\0{}", hint.unwrap_or_default()).as_bytes(),
    )
}

/// Embed pending chunks, returning successful count and per-chunk failures.
pub fn reconcile(
    conn: &Connection,
    source: &str,
    cmd: &str,
    hint: Option<&str>,
    sink: Option<&dyn ProgressSink>,
    full: bool,
) -> Result<(usize, Vec<String>)> {
    let model = provider_id(cmd, hint);
    let generation = document_vectors::begin(conn, source, &model)?;
    let pending = if full {
        document_vectors::all(conn, source, &model)?
    } else {
        document_vectors::pending(conn, source, &model)?
    };
    let mut embedded = 0;
    let mut errors = Vec::new();
    for chunk in pending {
        if sink.is_some_and(ProgressSink::is_cancelled) {
            return Err(Error::Cancelled);
        }
        match embed::embed_query_cancellable(cmd, &chunk.text, sink).and_then(|v| {
            if !source_unchanged(&chunk) {
                return Ok(false);
            }
            document_vectors::publish_guarded(conn, &chunk, &model, &v, &generation)
        }) {
            Ok(true) => embedded += 1,
            Ok(false) => {
                errors.push("document changed during embedding; retry indexing".to_string());
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => errors.push(format!("document embedding failed: {e}")),
        }
    }
    Ok((embedded, errors))
}

fn source_unchanged(chunk: &document_vectors::PendingChunk) -> bool {
    let root = std::path::Path::new(&chunk.root);
    let path = if chunk.file_root {
        root.to_path_buf()
    } else {
        root.join(&chunk.path)
    };
    let Ok(resolved) = std::fs::canonicalize(path) else {
        return false;
    };
    if !chunk.file_root && !resolved.starts_with(root) {
        return false;
    }
    std::fs::read(resolved)
        .is_ok_and(|bytes| super::document::fingerprint::content_hash(&bytes) == chunk.revision)
}
