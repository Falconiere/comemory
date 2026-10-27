//! File-based install into the Cursor config dir.
//!
//! `cursor-agent plugin` only manages git-URL marketplaces, so there is no
//! local-registration command to call. Install instead links the bundle's
//! skill dirs into `<config>/skills/` — Cursor's personal skills dir;
//! `skills-cursor/` is reserved for its built-ins — and merges the
//! `comemory` MCP server entry into `<config>/mcp.json`, owning only that
//! key. Hook wiring is a follow-up: the shared scripts dispatch on Claude
//! payload shapes, so wired cursor hooks would install inert.

use std::path::Path;

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;
use serde_json::value::RawValue;

use crate::prelude::*;

/// A JSON object read without re-serializing its values, in file order.
type RawObject = IndexMap<String, Box<RawValue>>;

/// One value of the rewritten `mcp.json`: an untouched original, the
/// rebuilt `mcpServers` object, or comemory's own entry.
#[derive(Serialize)]
#[serde(untagged)]
enum Node {
    /// A value copied through byte-identical.
    Raw(Box<RawValue>),
    /// A nested object, keys in file order.
    Object(IndexMap<String, Node>),
    /// A value comemory owns.
    Owned(Value),
}

/// Link skills and merge the MCP server entry. Both halves are idempotent:
/// a current install is a no-op, foreign content is preserved, and
/// anything unexpected is refused.
///
/// # Errors
/// [`Error::Usage`] for a skill entry or `mcp.json` comemory did not write
/// or cannot merge into; otherwise an [`Error::Io`] from the writes.
pub(super) fn install(config_dir: &Path, root: &Path, server: &Value) -> Result<()> {
    super::skill_links::link_skills(root, &config_dir.join("skills"))?;
    merge_mcp_server(&config_dir.join("mcp.json"), server)
}

/// Merge `{mcpServers: {comemory: server}}` into the cursor `mcp.json` at
/// `path`, creating it when absent. Only the `comemory` key is owned: other
/// keys and servers keep their order and their bytes, the file keeps its
/// permissions, and an already-current entry leaves the file untouched.
fn merge_mcp_server(path: &Path, server: &Value) -> Result<()> {
    if path
        .symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return Err(Error::Usage(format!(
            "refusing symlink mcp config: {}",
            path.display()
        )));
    }
    let (top, permissions) = match std::fs::read_to_string(path) {
        Ok(body) => (
            parse_object(&body, path, "mcp config")?,
            Some(std::fs::metadata(path)?.permissions()),
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (RawObject::new(), None),
        Err(error) => return Err(error.into()),
    };
    let servers = match top.get("mcpServers") {
        Some(raw) => parse_object(raw.get(), path, "mcpServers")?,
        None => RawObject::new(),
    };
    if let Some(current) = servers.get("comemory")
        && serde_json::from_str::<Value>(current.get())? == *server
    {
        return Ok(());
    }
    let mut merged: IndexMap<String, Node> = servers
        .into_iter()
        .map(|(key, value)| (key, Node::Raw(value)))
        .collect();
    merged.insert("comemory".to_string(), Node::Owned(server.clone()));
    let mut document: IndexMap<String, Node> = top
        .into_iter()
        .map(|(key, value)| (key, Node::Raw(value)))
        .collect();
    document.insert("mcpServers".to_string(), Node::Object(merged));
    write_replacing(path, &document, permissions)
}

/// Parse `body` as a JSON object, refusing anything else by name.
fn parse_object(body: &str, path: &Path, what: &str) -> Result<RawObject> {
    serde_json::from_str(body).map_err(|error| {
        Error::Usage(format!(
            "refusing to merge into {}: {what} is not a JSON object ({error})",
            path.display()
        ))
    })
}

/// Pretty-print `document` to a temp sibling, give it the original file's
/// permissions, and rename it over `path`.
fn write_replacing(
    path: &Path,
    document: &IndexMap<String, Node>,
    permissions: Option<std::fs::Permissions>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("json.tmp");
    super::bundle::write_new_file_no_symlink(
        &temp,
        format!("{}\n", serde_json::to_string_pretty(document)?).as_bytes(),
    )?;
    if let Some(permissions) = permissions {
        std::fs::set_permissions(&temp, permissions)?;
    }
    std::fs::rename(&temp, path)?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/cursor.rs"]
mod tests;
