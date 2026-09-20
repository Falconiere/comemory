//! Embedded agent integration; no repository checkout or download is needed.
use crate::prelude::*;
use std::io::Write as _;
use std::path::{Path, PathBuf};

const FILES: &[(&str, &str)] = &[
    (
        "hooks/memory-lifecycle.sh",
        include_str!("../../../../integrations/agent/hooks/memory-lifecycle.sh"),
    ),
    (
        "hooks/scope.sh",
        include_str!("../../../../integrations/agent/hooks/scope.sh"),
    ),
    (
        "hooks/session-end.sh",
        include_str!("../../../../integrations/agent/hooks/session-end.sh"),
    ),
    (
        "lib/shell-input.sh",
        include_str!("../../../../integrations/agent/lib/shell-input.sh"),
    ),
    (
        "lib/recall.sh",
        include_str!("../../../../integrations/agent/lib/recall.sh"),
    ),
    (
        ".claude-plugin/plugin.json",
        include_str!("../../../../integrations/agent/.claude-plugin/plugin.json"),
    ),
    (
        ".codex-plugin/plugin.json",
        include_str!("../../../../integrations/agent/.codex-plugin/plugin.json"),
    ),
    (
        "hooks/comemory-status.sh",
        include_str!("../../../../integrations/agent/hooks/comemory-status.sh"),
    ),
    (
        "hooks/hooks.json",
        include_str!("../../../../integrations/agent/hooks/hooks.json"),
    ),
    (
        "hooks/project-skills-curate.sh",
        include_str!("../../../../integrations/agent/hooks/project-skills-curate.sh"),
    ),
    (
        "hooks/project-skills-index.sh",
        include_str!("../../../../integrations/agent/hooks/project-skills-index.sh"),
    ),
    (
        "hooks/session-start.sh",
        include_str!("../../../../integrations/agent/hooks/session-start.sh"),
    ),
    (
        "hooks/skill-use.sh",
        include_str!("../../../../integrations/agent/hooks/skill-use.sh"),
    ),
    (
        "lib/project-skills.sh",
        include_str!("../../../../integrations/agent/lib/project-skills.sh"),
    ),
    (
        "lib/project-skills-foundation.sh",
        include_str!("../../../../integrations/agent/lib/project-skills-foundation.sh"),
    ),
    (
        "lib/project-skills-commands.sh",
        include_str!("../../../../integrations/agent/lib/project-skills-commands.sh"),
    ),
    (
        "lib/project-skills-curation.sh",
        include_str!("../../../../integrations/agent/lib/project-skills-curation.sh"),
    ),
    (
        "lib/repo-scope.sh",
        include_str!("../../../../integrations/agent/lib/repo-scope.sh"),
    ),
    (
        "skills/architecture-map/SKILL.md",
        include_str!("../../../../integrations/agent/skills/architecture-map/SKILL.md"),
    ),
    (
        "skills/agent-memory/SKILL.md",
        include_str!("../../../../integrations/agent/skills/agent-memory/SKILL.md"),
    ),
    (
        "skills/agent-memory/scripts/comemory.sh",
        include_str!("../../../../integrations/agent/skills/agent-memory/scripts/comemory.sh"),
    ),
    (
        "skills/memory-bootstrap/SKILL.md",
        include_str!("../../../../integrations/agent/skills/memory-bootstrap/SKILL.md"),
    ),
    (
        "skills/project-skills/SKILL.md",
        include_str!("../../../../integrations/agent/skills/project-skills/SKILL.md"),
    ),
    (
        "skills/project-skills/scripts/skills.sh",
        include_str!("../../../../integrations/agent/skills/project-skills/scripts/skills.sh"),
    ),
];

/// Extract the embedded bundle to `root`, or verify it is already
/// byte-identical there, then write the two local marketplace catalogs.
pub(super) fn extract(root: &Path) -> Result<()> {
    let parent = root
        .parent()
        .ok_or_else(|| Error::Usage("bundle needs a parent".into()))?;
    std::fs::create_dir_all(parent)?;
    let _lock = crate::utilities::file_lock::FileLock::acquire(
        &parent.join("install.lock"),
        "agent-install",
    )?;
    if root
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(Error::Usage(format!(
            "refusing symlink bundle: {}",
            root.display()
        )));
    }
    let files = files()?;
    if root.exists() {
        for (name, body) in &files {
            if std::fs::read(root.join(name))? != body.as_bytes() {
                return Err(Error::Usage(format!(
                    "bundle {} differs from this binary; preserve or move it before reinstalling",
                    root.display()
                )));
            }
        }
        return write_catalogs(parent);
    }
    let staging = parent.join(format!(".install-{}", std::process::id()));
    std::fs::create_dir(&staging)?;
    let result = write_files(&staging, &files).and_then(|()| {
        std::fs::rename(&staging, root)?;
        Ok(())
    });
    if result.is_err() {
        std::fs::remove_dir_all(&staging)?;
    }
    result?;
    write_catalogs(parent)
}

fn files() -> Result<Vec<(String, String)>> {
    let mut files = Vec::new();
    for (name, body) in FILES {
        let body = if name.ends_with("plugin.json") {
            let mut value: serde_json::Value = serde_json::from_str(body)?;
            value["version"] = env!("CARGO_PKG_VERSION").into();
            serde_json::to_string_pretty(&value)?
        } else {
            (*body).to_owned()
        };
        files.push((format!("plugins/comemory/{name}"), body));
    }
    Ok(files)
}

fn write_catalogs(parent: &Path) -> Result<()> {
    let source = format!("./{}/plugins/comemory", env!("CARGO_PKG_VERSION"));
    let claude = serde_json::json!({"name":"comemory", "owner":{"name":"Falconiere Barbosa"},
        "plugins":[{"name":"comemory","source":source}]});
    let codex = serde_json::json!({"name":"comemory", "interface":{"displayName":"comemory"},
        "plugins":[{"name":"comemory","source":{"source":"local","path":source},
        "policy":{"installation":"AVAILABLE","authentication":"ON_INSTALL"},"category":"Productivity"}]});
    for (name, value) in [
        (".claude-plugin/marketplace.json", claude),
        (".agents/plugins/marketplace.json", codex),
    ] {
        let path = parent.join(name);
        if path
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            return Err(Error::Usage(format!(
                "refusing symlink catalog: {}",
                path.display()
            )));
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_string_pretty(&value)?)?;
        std::fs::rename(temp, path)?;
    }
    Ok(())
}

/// Write `<plugin_root>/.mcp.json`, the manifest Claude Code and Codex read
/// to launch `comemory mcp` for this plugin. Written on **every** install —
/// including a re-run over an already-extracted, byte-identical bundle — so
/// moving the binary and reinstalling refreshes the path without tripping
/// the `bundle differs` refusal in [`extract`]. It is deliberately not part
/// of [`FILES`] or the equality check for the same reason.
///
/// # Errors
/// [`Error::Usage`] for a symlinked target (or symlinked temp file) or a
/// non-UTF-8 binary path; otherwise an [`Error::Io`] from the write or
/// rename.
pub(super) fn write_mcp_manifest(plugin_root: &Path, binary: &Path) -> Result<PathBuf> {
    let path = plugin_root.join(".mcp.json");
    if path
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(Error::Usage(format!(
            "refusing symlink mcp manifest: {}",
            path.display()
        )));
    }
    let command = binary
        .to_str()
        .ok_or_else(|| Error::Usage("binary path must be UTF-8".into()))?;
    let manifest = serde_json::json!({
        "mcpServers": {
            "comemory": { "command": command, "args": ["mcp"] }
        }
    });
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("json.tmp");
    write_new_file_no_symlink(
        &temp,
        format!("{}\n", serde_json::to_string_pretty(&manifest)?).as_bytes(),
    )?;
    std::fs::rename(&temp, &path)?;
    Ok(path)
}

/// Write `body` to `temp`, refusing to follow a symlink planted there.
/// A stale *regular* file left over from a crashed prior install is
/// removed first (the rename that follows always replaces it anyway);
/// a stale *symlink* is refused outright rather than opened, since
/// `OpenOptions::create_new` failing on an existing symlink still leaves
/// an attacker-controlled path for a caller who instead reached for
/// `std::fs::write` (which follows it and writes through).
fn write_new_file_no_symlink(temp: &Path, body: &[u8]) -> Result<()> {
    match temp.symlink_metadata() {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(Error::Usage(format!(
                "refusing symlink mcp manifest temp file: {}",
                temp.display()
            )));
        }
        Ok(_) => std::fs::remove_file(temp)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp)?;
    file.write_all(body)?;
    Ok(())
}

fn write_files(root: &Path, files: &[(String, String)]) -> Result<()> {
    for (name, body) in files {
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, body)?;
        #[cfg(unix)]
        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sh"))
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/bundle.rs"]
mod tests;
