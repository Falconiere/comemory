//! `[sync]` and `[embed]` config sections (memory-sync design spec).

use std::collections::BTreeMap;
use std::time::Duration;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::prelude::*;

/// Cloud-sync knobs — separate from `[git]` auto-commit of markdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// Best-effort push after each local save when true.
    pub after_save: bool,
    /// Pull before `context` when last sync is older than this (e.g. `5m`).
    pub pull_before_context_after: String,
    /// Interval hint for `comemory sync --verify` (e.g. `7d`).
    pub verify_every: String,
    /// Optional repo-label → workspace-id override cache (not source of truth).
    #[serde(default)]
    pub repos: BTreeMap<String, String>,
    /// Default workspace when `--workspace` is omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_workspace: Option<String>,
    /// TTL for the on-disk org-repo allowlist cache (e.g. `1h`).
    pub allowlist_ttl: String,
}

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
            after_save: true,
            pull_before_context_after: "5m".into(),
            verify_every: "7d".into(),
            repos: BTreeMap::new(),
            default_workspace: None,
            allowlist_ttl: "1h".into(),
        }
    }

    /// Overlay sparse `[sync]` keys from `config.toml`.
    pub fn apply(&mut self, partial: PartialSyncConfig) {
        if let Some(v) = partial.after_save {
            self.after_save = v;
        }
        if let Some(v) = partial.pull_before_context_after {
            self.pull_before_context_after = v;
        }
        if let Some(v) = partial.verify_every {
            self.verify_every = v;
        }
        if let Some(v) = partial.repos {
            self.repos = v;
        }
        if let Some(v) = partial.default_workspace {
            self.default_workspace = Some(v);
        }
        if let Some(v) = partial.allowlist_ttl {
            self.allowlist_ttl = v;
        }
    }

    /// Parse [`Self::allowlist_ttl`] as a [`Duration`].
    pub fn allowlist_ttl_duration(&self) -> Result<Duration> {
        parse_duration(&self.allowlist_ttl)
    }

    /// Parse [`Self::pull_before_context_after`] as a [`Duration`].
    pub fn pull_before_context_after_duration(&self) -> Result<Duration> {
        parse_duration(&self.pull_before_context_after)
    }

    /// Parse [`Self::verify_every`] as a [`Duration`].
    pub fn verify_every_duration(&self) -> Result<Duration> {
        parse_duration(&self.verify_every)
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
    conn.execute(
        "INSERT INTO schema_meta(key, value) VALUES('memory_vector_model', ?1) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![&embed.model],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/sync.rs"]
mod tests;
