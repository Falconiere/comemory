//! Organization-scoped login credentials at `$COMEMORY_DATA_DIR/auth.json`.
//!
//! Schema v2. The key minted by `comemory auth login` is scoped to one
//! organization and one workspace, so every sync call reads its target from
//! here instead of naming a workspace per request.
//!
//! A v1 file — written before organization scoping, carrying `device_name`
//! and `personal_workspace_id` — is **rejected, not migrated**. The secret it
//! holds is an unbound device key the platform no longer accepts for sync, so
//! defaulting the missing fields would produce a credential that parses
//! cleanly and then fails on every call, with a worse message and further from
//! the cause.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::env;
use crate::config::paths::Paths;
use crate::prelude::*;

/// Schema version this build writes and is willing to read.
pub const AUTH_SCHEMA_VERSION: u8 = 2;

/// Version stamped on a file written before organization scoping.
const LEGACY_SCHEMA_VERSION: u8 = 1;

/// On-disk org-scoped key bundle written by `comemory auth login`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthFile {
    /// Schema version; absent on a pre-org-scoping file, which means v1.
    #[serde(default = "legacy_version")]
    pub version: u8,
    /// Organization-scoped API secret (`cmk_…`).
    pub secret: String,
    /// Display prefix of the minted key.
    pub key_prefix: String,
    /// Platform API base URL the key authenticated against.
    pub api_url: String,
    /// Organization the key is scoped to.
    pub organization_id: String,
    /// Organization slug, for display.
    pub organization_slug: String,
    /// Organization display name.
    pub organization_name: String,
    /// The organization's workspace — the only one this key can reach.
    pub workspace_id: String,
    /// User email when the platform returned one (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

fn legacy_version() -> u8 {
    LEGACY_SCHEMA_VERSION
}

/// Read `path` when it exists; a missing file is `Ok(None)`.
fn read_if_present(path: &std::path::Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(fs::read_to_string(path)?))
}

/// The error for a credential this build cannot read, or `None` when the
/// version matches.
///
/// The version is probed before the strict parse because a v1 file is missing
/// every org field, so parsing it directly would report a confusing
/// "missing field `organization_id`" instead of "log in again". A *newer*
/// file gets its own message: telling someone their v3 credential predates
/// organization scoping would send them to fix the wrong thing.
fn schema_mismatch(raw: &str, path: &std::path::Path) -> Result<Option<Error>> {
    let probe: VersionProbe = serde_json::from_str(raw)?;
    if probe.version < AUTH_SCHEMA_VERSION {
        return Ok(Some(Error::Usage(format!(
            "credentials at {} predate organization scoping — run `comemory auth login`",
            path.display()
        ))));
    }
    if probe.version > AUTH_SCHEMA_VERSION {
        return Ok(Some(Error::Usage(format!(
            "credentials at {} were written by a newer comemory (schema v{}, this build reads v{AUTH_SCHEMA_VERSION}) — run `comemory upgrade`",
            path.display(),
            probe.version
        ))));
    }
    Ok(None)
}

/// Just enough of the file to decide whether the rest is worth parsing.
#[derive(Deserialize)]
struct VersionProbe {
    #[serde(default = "legacy_version")]
    version: u8,
}

impl AuthFile {
    /// Load `auth.json` when present; missing file → `Ok(None)`.
    ///
    /// # Errors
    /// [`Error::Usage`] when the file predates organization scoping, naming
    /// `comemory auth login` as the fix. Any other malformed file surfaces the
    /// underlying `serde_json` error.
    pub fn load(paths: &Paths) -> Result<Option<Self>> {
        let path = paths.auth_file();
        let Some(raw) = read_if_present(&path)? else {
            return Ok(None);
        };
        if let Some(mismatch) = schema_mismatch(&raw, &path)? {
            return Err(mismatch);
        }
        let file: Self = serde_json::from_str(&raw)?;
        // A field the platform left blank is as unusable as one it omitted,
        // and a hand-edited file can carry either. Refuse both here so no
        // caller has to re-check before addressing a workspace.
        if file.organization_id.trim().is_empty() || file.workspace_id.trim().is_empty() {
            return Err(Error::Usage(format!(
                "credentials at {} carry no organization scope — run `comemory auth login`",
                path.display()
            )));
        }
        Ok(Some(file))
    }

    /// Like [`Self::load`], but a credential this build cannot use reads as
    /// absent instead of raising.
    ///
    /// Best-effort callers (`save`, `context`) use this: they already do
    /// nothing when `auth.json` is missing, and turning every local save into
    /// a warning about a stale credential would be noise. The commands the
    /// user ran on purpose — `sync`, `auth status` — still report it.
    ///
    /// It re-derives the version rather than catching [`Error::Usage`] from
    /// [`Self::load`]: matching on the error variant would silently swallow
    /// any *future* usage error `load` grows, turning a real misconfiguration
    /// into a silent no-sync.
    pub fn load_usable(paths: &Paths) -> Result<Option<Self>> {
        let path = paths.auth_file();
        let Some(raw) = read_if_present(&path)? else {
            return Ok(None);
        };
        if schema_mismatch(&raw, &path)?.is_some() {
            return Ok(None);
        }
        Self::load(paths)
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

    /// Delete `auth.json` and any stale `allowlist.json` beside it. A missing
    /// file is success, so `comemory auth logout` stays idempotent.
    pub fn clear(paths: &Paths) -> Result<()> {
        remove_if_present(&paths.auth_file())?;
        clear_stale_allowlist(paths)
    }
}

/// Remove the `allowlist.json` left by releases before organization scoping.
///
/// Nothing reads it any more. It is deleted at both login and logout so a
/// cached repo list cannot outlive the credential it was fetched for.
pub fn clear_stale_allowlist(paths: &Paths) -> Result<()> {
    remove_if_present(&paths.allowlist_file())
}

/// Delete `path`, treating "already gone" as success.
fn remove_if_present(path: &std::path::Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
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
