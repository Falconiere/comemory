//! Index-run reporting and scoped file walk.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use git2::Repository;
use ignore::WalkBuilder;
use time::OffsetDateTime;

use super::{Request, Response, walk};
use crate::prelude::*;
use crate::store::{Connection, code_row, index_runs, memory_row, random_id};
use crate::utilities::progress::ProgressSink;

/// Prepare format and incremental cursors, reusing them only for the same root.
pub(super) fn prepare_index(
    tx: &Connection,
    req: &Request,
    root: &Path,
) -> Result<Vec<(String, String)>> {
    let previous = crate::store::indexed_files::list_for_repo(tx, &req.repo)?;
    let same_root = match (
        crate::store::repo_marker::root_path(tx, &req.repo)?,
        root.canonicalize(),
    ) {
        (Some(p), Ok(root)) => Path::new(&p).canonicalize().ok().as_ref() == Some(&root),
        _ => false,
    };
    code_row::ensure_repo_format(tx, &req.repo)?;
    if req.mode == super::IndexMode::Full {
        crate::store::indexed_files::delete_for_repo(tx, &req.repo)?;
    }
    Ok(if same_root { previous } else { Vec::new() })
}

/// Refresh graph data best-effort after the symbol transaction is durable.
pub(super) fn refresh_graph(
    conn: &mut Connection,
    req: &Request,
    root: &Path,
    imports: &BTreeMap<String, Vec<String>>,
    lookback_days: Option<u32>,
    git: bool,
) {
    if git
        && let Err(e) = crate::domains::graph::materialize::materialize(
            conn,
            root,
            &req.repo,
            imports,
            lookback_days,
        )
    {
        tracing::warn!(repo = %req.repo, error = %e,
            "index-code: graph materialization failed; symbol index kept");
    }
    let _stale = crate::domains::graph::derived::refresh_derived_best_effort(conn);
}

/// Insert this run's `index_runs` row from `outcome`. Best-effort: a write
/// failure is logged, never propagated — the symbols already landed (or
/// already rolled back), and a history row is not worth failing over.
pub(super) fn record_run(
    conn: &Connection,
    req: &Request,
    root: &Path,
    started_at: &str,
    elapsed: Duration,
    outcome: &Result<Response>,
) {
    let (slug, error, files_indexed) = match outcome {
        Ok(resp) => ("ok", None, resp.files_indexed),
        Err(Error::Cancelled) => ("cancelled", None, 0),
        Err(e) => ("error", Some(e.to_string()), 0),
    };
    // A failed count is recorded as 0, but never silently: without the log
    // an `index_runs` row saying `symbols: 0` is indistinguishable from a
    // repo that genuinely has none. The run itself is not failed for it —
    // the symbols are already written; this number is history.
    let symbols: i64 = code_row::count_for_repo(conn, &req.repo).unwrap_or_else(|e| {
        tracing::warn!(
            repo = %req.repo,
            error = %e,
            "index-code: symbol count failed; recording 0 in index_runs"
        );
        0
    });
    let root_path = root.canonicalize().ok();
    let written = random_id::random_hex(8).and_then(|id| {
        let finished_at = memory_row::iso_format(OffsetDateTime::now_utc())?;
        index_runs::insert(
            conn,
            &index_runs::NewIndexRun {
                id: &id,
                repo: &req.repo,
                root_path: root_path.as_deref().map(Path::to_string_lossy).as_deref(),
                mode: req.mode.as_str(),
                started_at,
                finished_at: &finished_at,
                duration_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                files_indexed: files_indexed as u64,
                symbols: u64::try_from(symbols).unwrap_or(0),
                outcome: slug,
                error: error.as_deref(),
            },
        )
    });
    if let Err(e) = written {
        tracing::warn!(repo = %req.repo, error = %e, "index-code: index_runs row not recorded");
    }
}

/// Walk every candidate file under `root` (the same `ignore`-crate filters
/// [`super::run_with_progress`] always used), indexing each into `tx` via
/// [`walk::index_file`] and reporting progress through `sink`. Returns the
/// count actually (re)indexed — entries skipped by the blob-OID gate still
/// count toward progress but not toward this return value.
///
/// Entries are collected up front (rather than indexed lazily) so `total`
/// is known before the loop starts — the progress sink needs a fixed
/// denominator.
pub(super) fn walk_repo(
    tx: &Connection,
    repo: &str,
    root: &Path,
    git_repo: Option<&Repository>,
    imports_by_file: &mut BTreeMap<String, Vec<String>>,
    sink: Option<&dyn ProgressSink>,
    scope: (&[(String, String)], Option<&Path>, u64),
) -> Result<(usize, usize, usize, usize)> {
    let entries = files(root, scope.1)?;
    let total = entries.len() as u64;
    let mut files_indexed = 0usize;
    let mut unchanged = 0usize;
    let mut unsupported = 0usize;
    let mut seen = BTreeSet::new();
    for (i, entry) in entries.iter().enumerate() {
        if sink.is_some_and(ProgressSink::is_cancelled) {
            return Err(Error::Cancelled);
        }
        let supported_tracked = supported_tracked(git_repo, entry.path())?;
        match walk::index_file(
            tx,
            repo,
            root,
            (git_repo, scope.2),
            entry.path(),
            imports_by_file,
            sink,
        )? {
            Some(changed) => {
                seen.insert(walk::relative(root, entry.path()));
                if changed {
                    files_indexed += 1;
                } else {
                    unchanged += 1;
                }
            }
            None if supported_tracked => {
                unsupported += 1;
            }
            None => {}
        }
        if let Some(sink) = sink {
            sink.on_progress(i as u64 + 1, total);
        }
    }
    let removed = remove_missing(tx, repo, scope.0, &seen)?;
    Ok((files_indexed, unchanged, removed, unsupported))
}

fn files(root: &Path, excluded: Option<&Path>) -> Result<Vec<ignore::DirEntry>> {
    let mut walker = WalkBuilder::new(root);
    walker.standard_filters(true);
    walker.add_custom_ignore_filename(".comemoryignore");
    if let Some(excluded) = excluded {
        let excluded = excluded.to_path_buf();
        walker.filter_entry(move |entry| !entry.path().starts_with(&excluded));
    }
    Ok(walker
        .build()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| Error::Other(format!("index-code walk incomplete: {e}")))?
        .into_iter()
        .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
        .collect())
}

fn supported_tracked(git: Option<&Repository>, path: &Path) -> Result<bool> {
    let tracked = match git {
        Some(git) => walk::tracked_file(git, path)?,
        None => true,
    };
    Ok(tracked && crate::domains::code::ast::languages::detect(path).is_some())
}

fn remove_missing(
    tx: &Connection,
    repo: &str,
    previous: &[(String, String)],
    seen: &BTreeSet<String>,
) -> Result<usize> {
    let mut removed = 0;
    for (path, _) in previous {
        if !seen.contains(path) {
            code_row::purge_file_symbols(tx, repo, path)?;
            crate::store::indexed_files::delete_one(tx, repo, path)?;
            crate::store::code_graph_edges::delete_file_edges(
                tx,
                &crate::store::edges::file_node_id(repo, path),
            )?;
            removed += 1;
        }
    }
    Ok(removed)
}
