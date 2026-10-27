//! Registration through the muse `plugins` CLI by direct path install.
//!
//! Muse records the source path a plugin was installed from and refuses to
//! install the same id from a different one. The bundle path is versioned,
//! so after an upgrade the previous comemory registration is removed first —
//! but only when its source lies inside comemory's own integrations dir; a
//! `comemory` plugin installed from anywhere else is refused, not replaced.
//!
//! Muse holds a third-party plugin's hooks and MCP server for review until
//! `muse plugins approve comemory` runs. That is the operator's trust
//! decision, so install leaves it to them and the report names the command.

use std::path::{Path, PathBuf};

use crate::prelude::*;

use super::host::Host;

/// The plugin id muse records the bundle under (its manifest `name`).
const PLUGIN_ID: &str = "comemory";

/// Install `<root>/plugins/comemory` into the muse data dir `config_dir`,
/// first removing a comemory registration from another bundle version.
///
/// # Errors
/// [`Error::Usage`] for a non-UTF-8 path or a foreign `comemory` plugin;
/// [`Error::Unavailable`] when a muse command fails.
pub(super) fn install(host: &Host, config_dir: &Path, root: &Path) -> Result<()> {
    let plugin = root.join("plugins/comemory");
    let path = plugin
        .to_str()
        .ok_or_else(|| Error::Usage("integration path must be UTF-8".into()))?;
    // Muse records the source canonicalized (`/private/var/...` for a
    // `/var/...` spelling), so both sides are compared canonical.
    if let Some(source) = installed_source(host, config_dir)?
        && super::canonical_or_raw(&source) != super::canonical_or_raw(&plugin)
    {
        let owned = root
            .parent()
            .ok_or_else(|| Error::Usage("bundle needs a parent".into()))?;
        if !super::canonical_or_raw(&source).starts_with(super::canonical_or_raw(owned)) {
            return Err(Error::Usage(format!(
                "muse already has a `{PLUGIN_ID}` plugin installed from {}, which comemory did not write; \
                 run `muse plugins remove {PLUGIN_ID}` before reinstalling",
                source.display()
            )));
        }
        // Validate the new bundle first: a remove followed by a failed
        // install would leave the operator with no comemory plugin at all.
        super::run_host(host, config_dir, &["plugins", "validate", path])?;
        super::run_host(host, config_dir, &["plugins", "remove", PLUGIN_ID])?;
    }
    super::run_host(host, config_dir, &["plugins", "install", path])?;
    Ok(())
}

/// The source path muse recorded for an installed `comemory` plugin, or
/// `None` when it has none.
fn installed_source(host: &Host, config_dir: &Path) -> Result<Option<PathBuf>> {
    let stdout = super::run_host(host, config_dir, &["plugins", "list", "--json"])?;
    let listing: serde_json::Value = serde_json::from_slice(&stdout)?;
    Ok(listing["plugins"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|plugin| plugin["record"]["id"] == PLUGIN_ID)
        .and_then(|plugin| plugin["record"]["source"]["path"].as_str())
        .map(PathBuf::from))
}
