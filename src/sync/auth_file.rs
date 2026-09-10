//! Device-login credentials persisted at `$COMEMORY_DATA_DIR/auth.json`.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::env;
use crate::config::paths::Paths;
use crate::prelude::*;

/// On-disk device key bundle written by `comemory auth login`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthFile {
    /// Device API secret (`cmk_…`).
    pub secret: String,
    /// Display prefix of the minted key.
    pub key_prefix: String,
    /// Idempotent personal workspace returned at login (v1 sync off).
    pub personal_workspace_id: String,
    /// Platform API base URL the device authenticated against.
    pub api_url: String,
    /// Human-readable device label from login.
    pub device_name: String,
    /// User email when the platform returned one (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

impl AuthFile {
    /// Load `auth.json` when present; missing file → `Ok(None)`.
    pub fn load(paths: &Paths) -> Result<Option<Self>> {
        let path = paths.auth_file();
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path)?;
        let file: Self = serde_json::from_str(&raw)?;
        Ok(Some(file))
    }

    /// Persist this bundle atomically with mode `0600` on unix.
    pub fn save(&self, paths: &Paths) -> Result<()> {
        let rendered = serde_json::to_string_pretty(self)?;
        let final_path = paths.auth_file();
        let tmp_path: PathBuf = paths.data_dir().join(".auth.json.tmp");
        // First login runs before any store exists, so the data dir may not
        // be there yet; the tmp write below fails with ENOENT without this.
        fs::create_dir_all(paths.data_dir())?;

        if let Err(e) = fs::write(&tmp_path, &rendered) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e.into());
        }
        set_private_mode(&tmp_path)?;
        if let Err(e) = fs::rename(&tmp_path, &final_path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e.into());
        }
        set_private_mode(&final_path)?;
        Ok(())
    }

    /// Effective API secret: `COMEMORY_API_KEY` env overrides the stored secret.
    pub fn effective_secret(&self) -> String {
        env::api_key_override().unwrap_or_else(|| self.secret.clone())
    }

    /// Delete `auth.json` when present. A missing file is success, so
    /// `comemory auth logout` stays idempotent.
    pub fn clear(paths: &Paths) -> Result<()> {
        match fs::remove_file(paths.auth_file()) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// Restrict a credential file to owner read/write on unix hosts.
fn set_private_mode(path: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/auth_file.rs"]
mod tests;
