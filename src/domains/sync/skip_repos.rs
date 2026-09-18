//! Local opt-out filter over memory `repo` labels before a push is enqueued.
//!
//! Replaces the per-repo GitHub App allowlist that `match_key.rs` applied:
//! organization membership is now the only gate the platform enforces, so the
//! only client-side control left is the operator's own `[sync] skip_repos`
//! glob list. Label normalization lives here too — it is the same trimmed,
//! lowercased form the allowlist used, and both the matcher and every caller
//! must agree on it for a pattern to mean one thing.

use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::prelude::*;

/// Lowercase trimmed repo label (`CodaSignal/Foo` → `codasignal/foo`).
pub fn normalize_repo_label(label: &str) -> String {
    label.trim().to_lowercase()
}

/// Compiled `[sync] skip_repos` patterns, matched against normalized labels.
#[derive(Debug, Clone)]
pub struct SkipMatcher {
    set: GlobSet,
}

impl SkipMatcher {
    /// Compile `patterns`. Each is lowercased first, so a pattern written
    /// `Acme/Secret-*` matches the same labels as `acme/secret-*`.
    ///
    /// # Errors
    /// [`Error::Config`] naming the offending pattern when a glob is invalid,
    /// so a bad `config.toml` fails at load rather than silently syncing a
    /// repository the operator meant to withhold.
    pub fn compile(patterns: &[String]) -> Result<Self> {
        // The overwhelmingly common case: no patterns configured. `GlobSet`
        // has an explicit empty form, so there is nothing to build.
        if patterns.is_empty() {
            return Ok(Self {
                set: GlobSet::empty(),
            });
        }
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            let normalized = normalize_repo_label(pattern);
            let glob = Glob::new(&normalized).map_err(|e| {
                Error::Config(format!("invalid sync.skip_repos pattern `{pattern}`: {e}"))
            })?;
            builder.add(glob);
        }
        let set = builder
            .build()
            .map_err(|e| Error::Config(format!("invalid sync.skip_repos: {e}")))?;
        Ok(Self { set })
    }

    /// True when `label` matches any pattern. An empty pattern list never
    /// matches, and an empty label is never skipped here — an unlabelled
    /// memory is already withheld by the push filter's own personal check.
    pub fn is_skipped(&self, label: &str) -> bool {
        let normalized = normalize_repo_label(label);
        if normalized.is_empty() {
            return false;
        }
        self.set.is_match(&normalized)
    }
}

#[cfg(test)]
#[path = "tests/skip_repos.rs"]
mod tests;
