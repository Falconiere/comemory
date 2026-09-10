//! Best-effort auto-sync hooks wired from `api::save` and `api::context`.

use std::thread::{self, JoinHandle};

use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::config::{Config, Paths};
use crate::prelude::*;
use crate::store::Connection;
use crate::sync::AuthFile;

/// After a successful local save, spawn a background push when
/// `[sync].after_save` is enabled. Never propagates errors to the caller.
///
/// The push runs on a plain OS thread rather than the caller's: it uses
/// `reqwest::blocking`, which panics if a tokio runtime is in scope, and every
/// CLI subcommand body runs inside one (see [`crate::cli::off_runtime`]).
///
/// **Join the returned handle if your process may exit soon.** A CLI process
/// exits within milliseconds of the save returning, and a detached thread dies
/// with it — which is why `[sync] after_save` used to be a race the push
/// usually lost. [`AutoPush::wait`] is how the CLI collects it; `serve` is
/// long-lived and lets it run on.
#[must_use = "a CLI caller must AutoPush::wait, or the push dies with the process"]
pub fn after_save_best_effort(paths: &Paths, cfg: &Config, _conn: &Connection) -> AutoPush {
    if !cfg.sync.after_save {
        return AutoPush(None);
    }
    let paths = paths.clone();
    let cfg = cfg.clone();
    AutoPush(
        thread::Builder::new()
            .name("comemory-auto-push".into())
            .spawn(move || {
                if let Err(e) = push_default_workspace(&paths, &cfg) {
                    tracing::warn!(error = %e, "auto-sync after save failed");
                }
            })
            .inspect_err(|e| tracing::warn!(error = %e, "auto-sync thread not started"))
            .ok(),
    )
}

/// A running after-save push, or nothing when `[sync] after_save` is off.
#[derive(Debug, Default)]
pub struct AutoPush(Option<JoinHandle<()>>);

impl AutoPush {
    /// Block until the push finishes. A panic inside it is logged, not
    /// re-raised: the local save already succeeded and must stay successful.
    pub fn wait(self) {
        if let Some(handle) = self.0
            && handle.join().is_err()
        {
            tracing::warn!("auto-sync after save panicked");
        }
    }
}

/// Before `context`, pull when the last sync is older than
/// `[sync].pull_before_context_after`. Best-effort — errors are logged only.
pub fn pull_before_context_best_effort(paths: &Paths, cfg: &Config, conn: &mut Connection) {
    let Ok(threshold) = cfg.sync.pull_before_context_after_duration() else {
        return;
    };
    let auth = match AuthFile::load_usable(paths) {
        Ok(Some(v)) => v,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(error = %e, "auto pull-before-context: auth load failed");
            return;
        }
    };
    let workspace_id = auth.workspace_id.clone();
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
    if let Err(e) = crate::sync::pull::run_pull(paths, cfg, conn, &auth, 500) {
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

/// Push everything the org key accepts. Runs on the [`AutoPush`] thread.
fn push_default_workspace(paths: &Paths, cfg: &Config) -> Result<()> {
    // `load_usable` so a credential predating org scoping is treated as "not
    // logged in" here: this runs after every save, and a stale auth.json must
    // not turn each one into a warning. `comemory sync` still reports it.
    let Some(auth) = AuthFile::load_usable(paths)? else {
        return Ok(());
    };
    let mut conn = crate::store::connection::open(paths.db_path())?;
    crate::config::sync::apply_embed_model(&conn, &cfg.embed)?;
    crate::sync::push::run_push(paths, cfg, &mut conn, &auth, None, 500)?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/auto.rs"]
mod tests;
