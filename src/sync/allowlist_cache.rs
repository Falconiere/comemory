//! Cached GitHub App org-repo allowlist fetched from the platform API.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::config::paths::Paths;
use crate::prelude::*;
use crate::sync::match_key::{AllowlistRepo, MatchOutcome, classify_repo};

/// On-disk allowlist bundle beside `auth.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowlistCache {
    /// HTTP `ETag` from the last successful fetch (optional).
    pub etag: Option<String>,
    /// UTC timestamp when this cache was written.
    pub fetched_at: OffsetDateTime,
    /// Allowlisted repos for the bound org workspace.
    pub repos: Vec<AllowlistRepo>,
    /// Org workspace id this cache belongs to.
    pub workspace_id: String,
}

impl AllowlistCache {
    /// Load `allowlist.json` when present; missing file → `Ok(None)`.
    pub fn load(paths: &Paths) -> Result<Option<Self>> {
        let path = allowlist_path(paths);
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path)?;
        let file: AllowlistCacheFile = serde_json::from_str(&raw)?;
        Ok(Some(file.into()))
    }

    /// Persist this cache atomically beside `auth.json`.
    pub fn save(&self, paths: &Paths) -> Result<()> {
        let rendered = serde_json::to_string_pretty(&AllowlistCacheFile::from(self))?;
        let final_path = allowlist_path(paths);
        let tmp_path: PathBuf = paths.data_dir().join(".allowlist.json.tmp");

        if let Err(e) = fs::write(&tmp_path, &rendered) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e.into());
        }
        if let Err(e) = fs::rename(&tmp_path, &final_path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e.into());
        }
        Ok(())
    }

    /// True when the cache age is strictly less than `ttl`.
    pub fn is_fresh(&self, ttl: Duration) -> bool {
        let elapsed = (OffsetDateTime::now_utc() - self.fetched_at).whole_seconds();
        let Ok(ttl_secs) = i64::try_from(ttl.as_secs()) else {
            return false;
        };
        elapsed >= 0 && elapsed < ttl_secs
    }

    /// Classify a memory `repo` label against this cache's repos.
    pub fn classify(&self, label: &str) -> MatchOutcome {
        classify_repo(label, &self.repos)
    }
}

/// Serde-friendly repo row in `allowlist.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AllowlistRepoRow {
    full_name: String,
    name: String,
}

/// Wire shape of `allowlist.json`.
#[derive(Debug, Serialize, Deserialize)]
struct AllowlistCacheFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    etag: Option<String>,
    fetched_at: OffsetDateTime,
    repos: Vec<AllowlistRepoRow>,
    workspace_id: String,
}

impl From<&AllowlistCache> for AllowlistCacheFile {
    fn from(cache: &AllowlistCache) -> Self {
        Self {
            etag: cache.etag.clone(),
            fetched_at: cache.fetched_at,
            repos: cache
                .repos
                .iter()
                .map(|r| AllowlistRepoRow {
                    full_name: r.full_name.clone(),
                    name: r.name.clone(),
                })
                .collect(),
            workspace_id: cache.workspace_id.clone(),
        }
    }
}

impl From<AllowlistCacheFile> for AllowlistCache {
    fn from(file: AllowlistCacheFile) -> Self {
        Self {
            etag: file.etag,
            fetched_at: file.fetched_at,
            repos: file
                .repos
                .into_iter()
                .map(|r| AllowlistRepo {
                    full_name: r.full_name,
                    name: r.name,
                })
                .collect(),
            workspace_id: file.workspace_id,
        }
    }
}

/// Path to the cached allowlist file (`<data-dir>/allowlist.json`).
pub fn allowlist_path(paths: &Paths) -> PathBuf {
    paths.data_dir().join("allowlist.json")
}

/// Convenience: classify `label` when a fresh-enough cache is on disk.
pub fn classify_with_cache(
    paths: &Paths,
    label: &str,
    ttl: Duration,
) -> Result<Option<MatchOutcome>> {
    let Some(cache) = AllowlistCache::load(paths)? else {
        return Ok(None);
    };
    if !cache.is_fresh(ttl) {
        return Ok(None);
    }
    Ok(Some(cache.classify(label)))
}

#[cfg(test)]
#[path = "tests/allowlist_cache.rs"]
mod tests;
