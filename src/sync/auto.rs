//! Best-effort auto-sync hooks wired from `api::save` and `api::context`.

use std::thread;

use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::config::{Config, Paths};
use crate::prelude::*;
use crate::store::Connection;
use crate::sync::AuthFile;

/// After a successful local save, optionally spawn a background push when
/// `[sync].after_save` is enabled. Never propagates errors to the caller.
pub fn after_save_best_effort(paths: &Paths, cfg: &Config, _conn: &Connection) {
    if !cfg.sync.after_save {
        return;
    }
    let paths = paths.clone();
    let cfg = cfg.clone();
    thread::spawn(move || {
        if let Err(e) = push_default_workspace(&paths, &cfg) {
            tracing::warn!(error = %e, "auto-sync after save failed");
        }
    });
}

/// Before `context`, pull when the last sync is older than
/// `[sync].pull_before_context_after`. Best-effort — errors are logged only.
pub fn pull_before_context_best_effort(paths: &Paths, cfg: &Config, conn: &mut Connection) {
    let Ok(threshold) = cfg.sync.pull_before_context_after_duration() else {
        return;
    };
    let auth = match AuthFile::load(paths) {
        Ok(Some(v)) => v,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(error = %e, "auto pull-before-context: auth load failed");
            return;
        }
    };
    let workspace_id = resolve_default_workspace(cfg, &auth);
    let sync_row = match crate::store::sync_state::get(conn, &workspace_id) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "auto pull-before-context: sync_state read failed");
            return;
        }
    };
    let needs_pull = match sync_row.and_then(|s| s.last_sync_at) {
        Some(at) => last_sync_stale(&at, threshold),
        None => true,
    };
    if !needs_pull {
        return;
    }
    if let Err(e) = crate::sync::pull::run_pull(paths, cfg, conn, &auth, &workspace_id, 500) {
        tracing::warn!(error = %e, "auto pull-before-context failed");
    }
}

fn last_sync_stale(at: &str, threshold: std::time::Duration) -> bool {
    let Ok(parsed) = OffsetDateTime::parse(at, &Iso8601::DEFAULT) else {
        return true;
    };
    let elapsed = (OffsetDateTime::now_utc() - parsed).whole_seconds();
    let Ok(limit) = i64::try_from(threshold.as_secs()) else {
        return true;
    };
    elapsed >= limit
}

fn push_default_workspace(paths: &Paths, cfg: &Config) -> Result<()> {
    let auth = AuthFile::load(paths)?
        .ok_or_else(|| Error::Usage("not logged in — run `comemory auth login`".into()))?;
    let workspace_id = resolve_default_workspace(cfg, &auth);
    let mut conn = crate::store::connection::open(paths.db_path())?;
    crate::config::sync::apply_embed_model(&conn, &cfg.embed)?;
    crate::sync::push::run_push(paths, cfg, &mut conn, &auth, &workspace_id, None, 500)?;
    Ok(())
}

fn resolve_default_workspace(cfg: &Config, auth: &AuthFile) -> String {
    cfg.sync
        .default_workspace
        .clone()
        .unwrap_or_else(|| auth.personal_workspace_id.clone())
}

#[cfg(test)]
#[path = "tests/auto.rs"]
mod tests;
