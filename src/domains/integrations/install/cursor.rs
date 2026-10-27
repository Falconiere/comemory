//! File-based install into the Cursor config dir.
//!
//! `cursor-agent plugin` only manages git-URL marketplaces, so there is no
//! local-registration command to call. Install instead copies the bundle's
//! skill dirs into `<config>/skills-cursor/` and merges the `comemory` MCP
//! server entry into `<config>/mcp.json`, owning only that key. Hook wiring
//! is a follow-up: the shared scripts dispatch on Claude payload shapes, so
//! wired cursor hooks would install inert.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::prelude::*;

use super::dirs::mirror_dir;

/// Copy skills and merge the MCP server entry. Both halves are idempotent:
/// identical content is a no-op, foreign content is preserved, and anything
/// unexpected is refused before a byte is written.
///
/// # Errors
/// [`Error::Usage`] for a symlinked target, differing skill content, or an
/// unparsable `mcp.json`; otherwise an [`Error::Io`] from the writes.
pub(super) fn install(config_dir: &Path, root: &Path) -> Result<()> {
    install_skills(config_dir, root)?;
    install_mcp_server(config_dir)?;
    Ok(())
}

/// Mirror every skill dir from the extracted bundle into `skills-cursor/`.
/// Skill names come from the bundle tree itself so a renamed skill cannot
/// drift out of sync with a hardcoded list.
fn install_skills(config_dir: &Path, root: &Path) -> Result<()> {
    let source = root.join("plugins/comemory/skills");
    let target = config_dir.join("skills-cursor");
    let mut names: Vec<PathBuf> = std::fs::read_dir(&source)?
        .map(|entry| entry.map(|entry| PathBuf::from(entry.file_name())))
        .collect::<std::io::Result<_>>()?;
    names.sort();
    for name in names {
        let from = source.join(&name);
        if !from.is_dir() {
            continue;
        }
        mirror_dir(&from, &target.join(name))?;
    }
    Ok(())
}

/// Merge `{mcpServers: {comemory: {command, args: ["mcp"]}}}` into the
/// cursor `mcp.json`, creating it when absent. Only the `comemory` key is
/// owned: foreign servers pass through byte-identical, and an `mcpServers`
/// value that is not an object is refused rather than replaced.
fn install_mcp_server(config_dir: &Path) -> Result<()> {
    let path = config_dir.join("mcp.json");
    if path
        .symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return Err(Error::Usage(format!(
            "refusing symlink mcp config: {}",
            path.display()
        )));
    }
    let mut value = match std::fs::read_to_string(&path) {
        Ok(body) => serde_json::from_str::<Value>(&body).map_err(|error| {
            Error::Usage(format!(
                "refusing to merge into unparsable {}: {error}",
                path.display()
            ))
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Value::Object(Map::new()),
        Err(error) => return Err(error.into()),
    };
    let servers = value
        .as_object_mut()
        .and_then(|object| {
            object
                .entry("mcpServers")
                .or_insert(Value::Object(Map::new()))
                .as_object_mut()
        })
        .ok_or_else(|| {
            Error::Usage(format!(
                "refusing to replace non-object mcpServers in {}",
                path.display()
            ))
        })?;
    let binary = std::fs::canonicalize(std::env::current_exe()?)?;
    let command = binary
        .to_str()
        .ok_or_else(|| Error::Usage("binary path must be UTF-8".to_string()))?;
    servers.insert(
        "comemory".to_string(),
        serde_json::json!({"command": command, "args": ["mcp"]}),
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("json.tmp");
    super::bundle::write_new_file_no_symlink(
        &temp,
        format!("{}\n", serde_json::to_string_pretty(&value)?).as_bytes(),
    )?;
    std::fs::rename(&temp, &path)?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/cursor.rs"]
mod tests;
