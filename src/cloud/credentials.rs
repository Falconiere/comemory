//! Durable workspace-key credentials at `{data_dir}/auth.json`.
//!
//! Written atomically (tmp + rename) at mode `0600`. `COMEMORY_API_KEY`
//! overrides the on-disk `secret` when set (CI without writing the file).

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::env::env_parse;
use crate::prelude::*;

/// Persisted cloud credentials (workspace-bound `cmk_` secret).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Credentials {
    /// API base URL used when these credentials were minted (or last written).
    pub api_url: String,
    /// Full `cmk_` + 64-hex secret. Shown once at login; never on `status`.
    pub secret: String,
    /// Stable key prefix (first 8 characters) for display.
    pub key_prefix: String,
    /// Workspace UUID the key is bound to.
    pub workspace_id: String,
}

/// Load credentials from `path`. Missing file → `Ok(None)`.
pub fn load(path: &Path) -> Result<Option<Credentials>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)?;
    let creds: Credentials = serde_json::from_str(&raw)?;
    Ok(Some(creds))
}

/// Write `creds` to `path` atomically at mode `0600`. Creates the parent
/// directory when missing.
pub fn save(path: &Path, creds: &Credentials) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let rendered = serde_json::to_string_pretty(creds)?;
    let tmp = path.with_extension("json.tmp");
    write_mode_0600(&tmp, rendered.as_bytes())?;
    if let Err(e) = fs::rename(&tmp, path) {
        if let Err(cleanup) = fs::remove_file(&tmp) {
            tracing::debug!(
                error = %cleanup,
                path = %tmp.display(),
                "auth.json.tmp cleanup failed after rename error"
            );
        }
        return Err(e.into());
    }
    // rename preserves the 0600 mode set on the tmp file at create time.
    Ok(())
}

/// Delete `path` when present. Missing file is success (logout is idempotent).
pub fn clear(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// `COMEMORY_API_KEY` when set and non-empty — overrides the file secret.
pub fn secret_override() -> Result<Option<String>> {
    Ok(env_parse::<String>("COMEMORY_API_KEY")?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty()))
}

/// Effective secret: env override, else `creds.secret`.
pub fn effective_secret(creds: Option<&Credentials>) -> Result<Option<String>> {
    if let Some(s) = secret_override()? {
        return Ok(Some(s));
    }
    Ok(creds.map(|c| c.secret.clone()))
}

fn write_mode_0600(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;

    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/credentials.rs"]
mod tests;
