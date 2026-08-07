//! Deterministic link deriver: `member_of_source` (file → source,
//! filtering/explain only) and `references_document` (the resolved, typed
//! form of a `<repo>:<path>` mention once its target has a `documents`
//! row). [`derive_after_document`] and [`derive_after_memory_save`] both
//! resolve through the same lookup, so whichever seam fires second
//! completes the link regardless of order. An ambiguous match — more than
//! one live document at the same `(repo, relative_path)` — stays an
//! unresolved diagnostic, never a guessed edge.

use std::path::{Component, Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::graph::edges::{self, EdgeKey, MEMBER_OF_SOURCE, REFERENCES_DOCUMENT, REFERENCES_FILE};
use crate::prelude::*;

/// Derive every deterministic edge owed by one just-committed document:
/// its `member_of_source` row, any pre-existing memory `references_file`
/// mention of its identity resolved to `references_document`, and any of
/// its own Markdown links that resolve to another indexed document.
pub fn derive_after_document(
    conn: &Connection,
    source_id: &str,
    file_id: &str,
    document_id: &str,
    repo: Option<&str>,
    relative_path: &str,
    links: &[String],
) -> Result<()> {
    edges::insert(
        conn,
        EdgeKey {
            src_kind: "file",
            src_id: file_id,
            dst_kind: "source",
            dst_id: source_id,
            rel: MEMBER_OF_SOURCE,
        },
    )?;
    if let Some(repo) = repo {
        resolve_memory_references(conn, repo, relative_path, document_id)?;
    }
    resolve_markdown_links(conn, source_id, repo, relative_path, document_id, links)?;
    Ok(())
}

/// Resolve `file_refs` (bare `<repo>:<path>` strings, as harvested by
/// [`crate::graph::cross_link::extract_refs`]) against any already-indexed
/// document and materialize `references_document` for each live match.
pub fn derive_after_memory_save(
    conn: &Connection,
    memory_id: &str,
    file_refs: &[String],
) -> Result<()> {
    for file_q in file_refs {
        let Some((repo, path)) = file_q.split_once(':') else {
            continue;
        };
        if let Some(document_id) = resolve_document_id(conn, repo, path)? {
            edges::insert(
                conn,
                EdgeKey {
                    src_kind: "memory",
                    src_id: memory_id,
                    dst_kind: "document",
                    dst_id: &document_id,
                    rel: REFERENCES_DOCUMENT,
                },
            )?;
        }
    }
    Ok(())
}

/// Seam A: find every memory whose `references_file` edge already names
/// this document's `<repo>:<path>` identity, and materialize the resolved
/// `references_document` edge for each.
fn resolve_memory_references(
    conn: &Connection,
    repo: &str,
    relative_path: &str,
    document_id: &str,
) -> Result<()> {
    let bare = format!("{repo}:{relative_path}");
    let mut stmt = conn.prepare(
        "SELECT src_id FROM edges \
          WHERE rel = ?1 AND src_kind = 'memory' AND dst_kind = 'file' AND dst_id = ?2",
    )?;
    let memory_ids: Vec<String> = stmt
        .query_map(params![REFERENCES_FILE, bare], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    for memory_id in memory_ids {
        edges::insert(
            conn,
            EdgeKey {
                src_kind: "memory",
                src_id: &memory_id,
                dst_kind: "document",
                dst_id: document_id,
                rel: REFERENCES_DOCUMENT,
            },
        )?;
    }
    Ok(())
}

/// Resolve every Markdown link target against the indexed corpus and
/// materialize a `document → document` `references_document` edge for
/// each unique resolution. An unresolved or ambiguous target is a
/// `tracing::debug!` diagnostic, never a guessed edge.
fn resolve_markdown_links(
    conn: &Connection,
    source_id: &str,
    repo: Option<&str>,
    current_relative_path: &str,
    document_id: &str,
    links: &[String],
) -> Result<()> {
    for raw in links {
        let Some(target_path) = normalize_link_target(current_relative_path, raw) else {
            continue;
        };
        let resolved = match same_source_document(conn, source_id, &target_path)? {
            Some(id) => Some(id),
            None => match repo {
                Some(repo) => resolve_document_id(conn, repo, &target_path)?,
                None => None,
            },
        };
        let Some(target_id) = resolved else {
            tracing::debug!(document_id, target = %raw, "unresolved markdown link; no edge derived");
            continue;
        };
        if target_id == document_id {
            continue;
        }
        edges::insert(
            conn,
            EdgeKey {
                src_kind: "document",
                src_id: document_id,
                dst_kind: "document",
                dst_id: &target_id,
                rel: REFERENCES_DOCUMENT,
            },
        )?;
    }
    Ok(())
}

/// Look up a document by relative path within one source. `source_files`
/// carries `UNIQUE (source_id, relative_path)`, so this can never be
/// ambiguous — the caller's same-source lookup is a deterministic 0-or-1
/// match.
fn same_source_document(
    conn: &Connection,
    source_id: &str,
    relative_path: &str,
) -> Result<Option<String>> {
    conn.query_row(
        "SELECT d.id FROM documents d JOIN source_files sf ON sf.id = d.source_file_id \
          WHERE sf.source_id = ?1 AND sf.relative_path = ?2",
        params![source_id, relative_path],
        |r| r.get(0),
    )
    .optional()
    .map_err(Error::from)
}

/// Look up a live document by `(repo, relative_path)` across every
/// indexed source. `0` matches is `None`; `1` match resolves; `2+` is an
/// ambiguous reference — logged, not guessed (design spec: "Ambiguous
/// references stay unresolved diagnostics, not guessed edges").
fn resolve_document_id(
    conn: &Connection,
    repo: &str,
    relative_path: &str,
) -> Result<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT d.id FROM documents d JOIN source_files sf ON sf.id = d.source_file_id \
          WHERE d.repo = ?1 AND sf.relative_path = ?2",
    )?;
    let ids: Vec<String> = stmt
        .query_map(params![repo, relative_path], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    match ids.as_slice() {
        [] => Ok(None),
        [id] => Ok(Some(id.clone())),
        _ => {
            tracing::debug!(
                repo,
                relative_path,
                count = ids.len(),
                "ambiguous document reference; no edge derived"
            );
            Ok(None)
        }
    }
}

/// Resolve a raw Markdown link target against `current_relative_path`'s
/// directory: strip a `#fragment`, reject an external URL or `mailto:`
/// link, treat a leading `/` as source-root-relative, then normalize
/// `.`/`..` components. Returns `None` for anything that cannot name a
/// local document.
fn normalize_link_target(current_relative_path: &str, raw: &str) -> Option<String> {
    let target = raw.split('#').next().unwrap_or("");
    if target.is_empty() || target.contains("://") || target.starts_with("mailto:") {
        return None;
    }
    let joined = if let Some(rooted) = target.strip_prefix('/') {
        PathBuf::from(rooted)
    } else {
        let dir = Path::new(current_relative_path)
            .parent()
            .unwrap_or_else(|| Path::new(""));
        dir.join(target)
    };
    Some(normalize_components(&joined))
}

/// Collapse `.`/`..` path components into a clean `/`-joined relative
/// path, matching the separator [`crate::source::discover`] stores in
/// `source_files.relative_path`.
fn normalize_components(path: &Path) -> String {
    let mut out: Vec<&std::ffi::OsStr> = Vec::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(part) => out.push(part),
            _ => {}
        }
    }
    out.iter()
        .map(|s| s.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
#[path = "tests/doc_link.rs"]
mod tests;
