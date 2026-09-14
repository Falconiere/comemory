//! `[sync]` and `[embed]` config sections (memory-sync design spec).

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::prelude::*;
use crate::store::Connection;

/// Cloud-sync knobs — separate from `[git]` auto-commit of markdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// Deprecated: in-process push after save. Default off; the user daemon
    /// owns continuous sync. Kept so existing `config.toml` still loads.
    pub after_save: bool,
    /// Deprecated: in-process pull before `context`. Empty / `"0"` means off.
    /// The user daemon owns continuous sync.
    pub pull_before_context_after: String,
    /// Interval hint for verify (`comemory sync --action verify` and the
    /// daemon's occasional verify pass).
    pub verify_every: String,
    /// Sleep between daemon pull+push cycles (e.g. `5s`). Wake-on-save can
    /// end the sleep early — see `daemon_wake`.
    pub daemon_interval: String,
    /// Repo labels withheld from push, as globs over the normalized label.
    /// The only client-side sync filter left now that organization membership
    /// is the platform's gate.
    #[serde(default)]
    pub skip_repos: Vec<String>,
    /// Deprecated, parsed and ignored: repo-label → workspace-id cache. It was
    /// written by the removed `comemory link` and read by nothing.
    #[serde(default)]
    pub repos: BTreeMap<String, String>,
    /// Deprecated, parsed and ignored: the workspace now comes from the
    /// org-scoped key in `auth.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_workspace: Option<String>,
    /// Deprecated, parsed and ignored: there is no allowlist cache to expire.
    pub allowlist_ttl: String,
}

/// `[sync]` keys kept only so an existing `config.toml` still loads.
///
/// `PartialSyncConfig` is `deny_unknown_fields`, so deleting a key outright
/// would turn every config that sets it into a hard load error. They are
/// parsed, ignored, and warned about for one release.
const DEPRECATED_KEYS: &[(&str, &str)] = &[
    (
        "sync.repos",
        "the removed `comemory link` wrote it; nothing reads it",
    ),
    (
        "sync.default_workspace",
        "the workspace comes from the org-scoped key in auth.json",
    ),
    (
        "sync.allowlist_ttl",
        "organization membership replaced the per-repo allowlist",
    ),
    (
        "sync.after_save",
        "continuous sync is the user daemon (`comemory sync daemon`); in-process after_save is no longer wired",
    ),
    (
        "sync.pull_before_context_after",
        "continuous sync is the user daemon (`comemory sync daemon`); in-process pull-before-context is no longer wired",
    ),
];

/// Embedder model id recorded in `schema_meta.memory_vector_model`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedConfig {
    /// Model name; empty means do not overwrite `schema_meta`.
    #[serde(default)]
    pub model: String,
}

/// File-overlay partial for [`SyncConfig`].
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PartialSyncConfig {
    after_save: Option<bool>,
    pull_before_context_after: Option<String>,
    verify_every: Option<String>,
    daemon_interval: Option<String>,
    skip_repos: Option<Vec<String>>,
    repos: Option<BTreeMap<String, String>>,
    default_workspace: Option<String>,
    allowlist_ttl: Option<String>,
}

/// File-overlay partial for [`EmbedConfig`].
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PartialEmbedConfig {
    model: Option<String>,
}

impl SyncConfig {
    /// Shipped defaults for the `[sync]` section.
    pub fn defaults() -> Self {
        Self {
            after_save: false,
            pull_before_context_after: String::new(),
            verify_every: "7d".into(),
            daemon_interval: "5s".into(),
            skip_repos: Vec::new(),
            repos: BTreeMap::new(),
            default_workspace: None,
            allowlist_ttl: "1h".into(),
        }
    }

    /// Overlay sparse `[sync]` keys from `config.toml`.
    pub fn apply(&mut self, partial: PartialSyncConfig) {
        if let Some(v) = partial.after_save {
            self.after_save = v;
            warn_deprecated("sync.after_save");
        }
        if let Some(v) = partial.pull_before_context_after {
            self.pull_before_context_after = v;
            warn_deprecated("sync.pull_before_context_after");
        }
        if let Some(v) = partial.verify_every {
            self.verify_every = v;
        }
        if let Some(v) = partial.daemon_interval {
            self.daemon_interval = v;
        }
        if let Some(v) = partial.skip_repos {
            self.skip_repos = v;
        }
        if let Some(v) = partial.repos {
            self.repos = v;
            warn_deprecated("sync.repos");
        }
        if let Some(v) = partial.default_workspace {
            self.default_workspace = Some(v);
            warn_deprecated("sync.default_workspace");
        }
        if let Some(v) = partial.allowlist_ttl {
            self.allowlist_ttl = v;
            warn_deprecated("sync.allowlist_ttl");
        }
    }

    /// Compile [`Self::skip_repos`] into a matcher.
    ///
    /// # Errors
    /// [`Error::Config`] when a pattern is not a valid glob.
    pub fn skip_matcher(&self) -> Result<crate::sync::skip_repos::SkipMatcher> {
        crate::sync::skip_repos::SkipMatcher::compile(&self.skip_repos)
    }

    /// Parse [`Self::allowlist_ttl`] as a [`Duration`].
    pub fn allowlist_ttl_duration(&self) -> Result<Duration> {
        parse_duration(&self.allowlist_ttl)
    }

    /// Parse [`Self::pull_before_context_after`] as a [`Duration`].
    ///
    /// Empty or `"0"` / `"0s"` means the hook is off.
    pub fn pull_before_context_after_duration(&self) -> Result<Duration> {
        let trimmed = self.pull_before_context_after.trim();
        if trimmed.is_empty() || trimmed == "0" {
            return Ok(Duration::ZERO);
        }
        parse_duration(trimmed)
    }

    /// Parse [`Self::verify_every`] as a [`Duration`].
    pub fn verify_every_duration(&self) -> Result<Duration> {
        parse_duration(&self.verify_every)
    }

    /// Parse [`Self::daemon_interval`] as a [`Duration`].
    pub fn daemon_interval_duration(&self) -> Result<Duration> {
        parse_duration(&self.daemon_interval)
    }
}

impl EmbedConfig {
    /// Shipped defaults for the `[embed]` section.
    pub fn defaults() -> Self {
        Self {
            model: String::new(),
        }
    }

    /// Overlay sparse `[embed]` keys from `config.toml`.
    pub fn apply(&mut self, partial: PartialEmbedConfig) {
        if let Some(v) = partial.model {
            self.model = v;
        }
    }
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self::defaults()
    }
}

impl Default for EmbedConfig {
    fn default() -> Self {
        Self::defaults()
    }
}

/// Warn that `key` is set but no longer does anything.
///
/// Fires once per config load — so once per CLI invocation, and once at
/// `serve` startup. It is deliberately not deduplicated across process runs:
/// a warning the user sees once and forgets is worse than one that keeps
/// pointing at a key they still have to remove.
fn warn_deprecated(key: &str) {
    let reason = DEPRECATED_KEYS
        .iter()
        .find(|(name, _)| *name == key)
        .map_or("no longer used", |(_, reason)| *reason);
    tracing::warn!(
        "config.toml sets `{key}`, which is deprecated and ignored: {reason}. Remove it."
    );
}

/// Parse a compact duration string: `<n><s|m|h|d>` (e.g. `5m`, `7d`, `1h`).
pub fn parse_duration(raw: &str) -> Result<Duration> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::Config("duration must not be empty".into()));
    }
    let (num, unit) = trimmed
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(i, c)| (&trimmed[..i], c))
        .ok_or_else(|| Error::Config(format!("invalid duration `{raw}`: missing unit")))?;
    if num.is_empty() {
        return Err(Error::Config(format!(
            "invalid duration `{raw}`: missing number"
        )));
    }
    let n: u64 = num
        .parse()
        .map_err(|_| Error::Config(format!("invalid duration `{raw}`: bad number")))?;
    let secs = match unit {
        's' | 'S' => n,
        'm' | 'M' => n
            .checked_mul(60)
            .ok_or_else(|| Error::Config(format!("invalid duration `{raw}`: overflow")))?,
        'h' | 'H' => n
            .checked_mul(3600)
            .ok_or_else(|| Error::Config(format!("invalid duration `{raw}`: overflow")))?,
        'd' | 'D' => n
            .checked_mul(86_400)
            .ok_or_else(|| Error::Config(format!("invalid duration `{raw}`: overflow")))?,
        _ => {
            return Err(Error::Config(format!(
                "invalid duration `{raw}`: unit must be s, m, h, or d"
            )));
        }
    };
    Ok(Duration::from_secs(secs))
}

/// When `[embed].model` is set, mirror it into `schema_meta.memory_vector_model`.
pub fn apply_embed_model(conn: &Connection, embed: &EmbedConfig) -> Result<()> {
    if embed.model.is_empty() {
        return Ok(());
    }
    crate::store::schema_meta::set_memory_vector_model(conn, &embed.model)
}

#[cfg(test)]
#[path = "tests/sync.rs"]
mod tests;
