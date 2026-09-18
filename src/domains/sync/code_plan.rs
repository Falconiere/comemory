//! The pure half of the code-index push: diff the local `indexed_files`
//! rows against the workspace's manifest, and cut the result into batches
//! the platform's caps admit. No store, no network — every branch is a
//! function of its arguments, which is what makes the batching assertable.

use std::collections::BTreeMap;

use crate::domains::sync::exchange::{CoChangeWire, CodeFileRef, CodeFileWire, CodeImportRequest};

/// Files per batch — the server's own cap.
pub const MAX_BATCH_FILES: usize = 500;

/// Serialized bytes per batch, kept well under the engine's 5 MiB body
/// ceiling so a symbol-dense repo cannot produce a batch the server refuses.
pub const MAX_BATCH_BYTES: usize = 1024 * 1024;

/// What one repo needs pushed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodePlan {
    /// Local paths whose blob is absent or different on the workspace,
    /// ascending.
    pub changed: Vec<String>,
    /// Workspace paths the local index no longer has, ascending.
    pub removed: Vec<String>,
    /// Whether the co-change set must be sent — the workspace's cursor is
    /// not the local one (a never-mined local repo sends nothing).
    pub send_cochange: bool,
    /// Whether the workspace's head disagrees with the local one.
    pub head_moved: bool,
}

impl CodePlan {
    /// Nothing to send: every file matches, nothing was removed, and both
    /// cursors already agree.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty()
            && self.removed.is_empty()
            && !self.send_cochange
            && !self.head_moved
    }
}

/// Diff `local` (`(path, blob_oid)` rows) against the workspace's `remote`
/// file list and cursors.
#[must_use]
pub fn plan(
    local: &[(String, String)],
    remote: &[CodeFileRef],
    local_head: Option<&str>,
    remote_head: Option<&str>,
    local_mined: Option<&str>,
    remote_mined: Option<&str>,
) -> CodePlan {
    let remote_by_path: BTreeMap<&str, &str> = remote
        .iter()
        .map(|f| (f.path.as_str(), f.blob_oid.as_str()))
        .collect();
    let local_by_path: BTreeMap<&str, &str> = local
        .iter()
        .map(|(path, oid)| (path.as_str(), oid.as_str()))
        .collect();
    let changed = local_by_path
        .iter()
        .filter(|(path, oid)| remote_by_path.get(*path) != Some(oid))
        .map(|(path, _)| (*path).to_owned())
        .collect();
    let removed = remote_by_path
        .keys()
        .filter(|path| !local_by_path.contains_key(*path))
        .map(|path| (*path).to_owned())
        .collect();
    CodePlan {
        changed,
        removed,
        send_cochange: local_mined.is_some() && local_mined != remote_mined,
        head_moved: local_head != remote_head,
    }
}

/// Cut `files` and `removed` into import requests: at most
/// [`MAX_BATCH_FILES`] files and roughly [`MAX_BATCH_BYTES`] of files per
/// batch, removals spread over the same batches. The head, the mining
/// cursor and the co-change set ride the LAST batch only: the workspace
/// stamps `repo_marker` when it sees them, and a push that dies between
/// batches must leave the marker at the head whose files all landed, not
/// at one it holds half of. Always at least one request, so a head-only
/// move still lands.
///
/// # Errors
/// Only a serialization failure while sizing a file entry.
pub fn batches(
    repo: &str,
    head: Option<&str>,
    mined_commit: Option<&str>,
    files: Vec<CodeFileWire>,
    removed: Vec<String>,
    cochange: Option<Vec<CoChangeWire>>,
) -> crate::prelude::Result<Vec<CodeImportRequest>> {
    let mut file_chunks: Vec<Vec<CodeFileWire>> = Vec::new();
    let mut current: Vec<CodeFileWire> = Vec::new();
    let mut current_bytes = 0usize;
    for file in files {
        let bytes = serde_json::to_vec(&file)?.len();
        if !current.is_empty()
            && (current.len() >= MAX_BATCH_FILES || current_bytes + bytes > MAX_BATCH_BYTES)
        {
            file_chunks.push(std::mem::take(&mut current));
            current_bytes = 0;
        }
        current_bytes += bytes;
        current.push(file);
    }
    if !current.is_empty() {
        file_chunks.push(current);
    }
    let removed_chunks: Vec<Vec<String>> = removed
        .chunks(MAX_BATCH_FILES)
        .map(<[String]>::to_vec)
        .collect();
    let count = file_chunks.len().max(removed_chunks.len()).max(1);
    let mut files_iter = file_chunks.into_iter();
    let mut removed_iter = removed_chunks.into_iter();
    let mut cochange = cochange;
    Ok((0..count)
        .map(|index| {
            let last = index + 1 == count;
            CodeImportRequest {
                repo: repo.to_owned(),
                head: head.filter(|_| last).map(str::to_owned),
                mined_commit: mined_commit.filter(|_| last).map(str::to_owned),
                files: files_iter.next().unwrap_or_default(),
                removed: removed_iter.next().unwrap_or_default(),
                cochange: if last { cochange.take() } else { None },
            }
        })
        .collect())
}

#[cfg(test)]
#[path = "tests/code_plan.rs"]
mod tests;
