//! File-based install into the Cursor config dir.
//!
//! `cursor-agent plugin` only manages git-URL marketplaces, so there is no
//! local-registration command to call. Install instead links the bundle's
//! skill dirs into `<config>/skills/` — Cursor's personal skills dir;
//! `skills-cursor/` is reserved for its built-ins — and merges the
//! `comemory` MCP server entry into `<config>/mcp.json` (or the file it
//! links to), owning only that key. Hook wiring is a follow-up: the shared scripts dispatch on Claude
//! payload shapes, so wired cursor hooks would install inert.

use std::path::{Path, PathBuf};

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

/// Link skills and merge the MCP server entry. `mcp.json` is read and
/// checked first, so a config comemory cannot merge into is refused before
/// any skill link is written. Both halves are idempotent: a current install
/// is a no-op, foreign content is preserved, and anything unexpected is
/// refused.
///
/// # Errors
/// [`Error::Usage`] for a skill entry or `mcp.json` comemory did not write
/// or cannot merge into; otherwise an [`Error::Io`] from the writes.
pub(super) fn install(config_dir: &Path, root: &Path, server: &Value) -> Result<()> {
    let merge = plan_mcp_merge(&config_dir.join("mcp.json"), server)?;
    super::skill_links::link_skills(root, &config_dir.join("skills"))?;
    match merge {
        Some(merge) => merge.write(),
        None => Ok(()),
    }
}

/// A rewritten `mcp.json`, ready to replace `target`.
struct McpMerge {
    /// The file to replace: `mcp.json`, or the file it links to.
    target: PathBuf,
    /// The merged document, keys in file order.
    document: IndexMap<String, Node>,
    /// The original file's permissions, kept on the replacement.
    permissions: Option<std::fs::Permissions>,
}

/// Merge `{mcpServers: {comemory: server}}` into the cursor `mcp.json` at
/// `path`, or `None` when the entry is already current. Only the `comemory`
/// key is owned: other keys and servers keep their order and their bytes,
/// and an empty file counts as `{}`. A symlinked `mcp.json` (a dotfile
/// manager's) is merged into the file it links to, so the link survives.
fn plan_mcp_merge(path: &Path, server: &Value) -> Result<Option<McpMerge>> {
    let target = resolve_target(path)?;
    let (top, permissions) = match std::fs::read_to_string(&target) {
        Ok(body) if body.trim().is_empty() => (
            RawObject::new(),
            Some(std::fs::metadata(&target)?.permissions()),
        ),
        Ok(body) => (
            parse_object(&body, &target, "mcp config")?,
            Some(std::fs::metadata(&target)?.permissions()),
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (RawObject::new(), None),
        Err(error) => return Err(error.into()),
    };
    let servers = match top.get("mcpServers") {
        Some(raw) => parse_object(raw.get(), &target, "mcpServers")?,
        None => RawObject::new(),
    };
    if let Some(current) = servers.get("comemory")
        && serde_json::from_str::<Value>(current.get())? == *server
    {
        return Ok(None);
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
    Ok(Some(McpMerge {
        target,
        document,
        permissions,
    }))
}

/// The file a write to `path` must replace: `path` itself, or — for a
/// symlink — the regular file it resolves to. A dangling link or one to a
/// non-file is refused.
fn resolve_target(path: &Path) -> Result<PathBuf> {
    let is_link = path
        .symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink());
    if !is_link {
        return Ok(path.to_path_buf());
    }
    match std::fs::canonicalize(path) {
        Ok(target) if target.is_file() => Ok(target),
        _ => Err(Error::Usage(format!(
            "refusing mcp config {}: it links to a missing file or a non-file",
            path.display()
        ))),
    }
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

impl McpMerge {
    /// Pretty-print the document to a temp sibling created with the
    /// original file's permissions, and rename it over the target.
    fn write(self) -> Result<()> {
        if let Some(parent) = self.target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let name = self.target.file_name().ok_or_else(|| {
            Error::Usage(format!("bad mcp config path: {}", self.target.display()))
        })?;
        let temp = self
            .target
            .with_file_name(format!(".{}.comemory.tmp", name.to_string_lossy()));
        super::bundle::write_new_file_no_symlink(
            &temp,
            format!("{}\n", serde_json::to_string_pretty(&self.document)?).as_bytes(),
            self.permissions.as_ref().map(unix_mode),
        )?;
        if let Some(permissions) = self.permissions {
            std::fs::set_permissions(&temp, permissions)?;
        }
        std::fs::rename(&temp, &self.target)?;
        Ok(())
    }
}

/// The unix permission bits of `permissions` (0o644 elsewhere).
fn unix_mode(permissions: &std::fs::Permissions) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        permissions.mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        let _ = permissions;
        0o644
    }
}

#[cfg(test)]
#[path = "tests/cursor.rs"]
mod tests;
