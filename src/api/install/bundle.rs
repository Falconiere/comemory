//! Embedded agent integration; no repository checkout or download is needed.
use crate::prelude::*;
use std::path::Path;

const FILES: &[(&str, &str)] = &[
    (
        "hooks/memory-lifecycle.sh",
        include_str!("../../../integrations/agent/hooks/memory-lifecycle.sh"),
    ),
    (
        "hooks/scope.sh",
        include_str!("../../../integrations/agent/hooks/scope.sh"),
    ),
    (
        "lib/shell-input.sh",
        include_str!("../../../integrations/agent/lib/shell-input.sh"),
    ),
    (
        ".claude-plugin/plugin.json",
        include_str!("../../../integrations/agent/.claude-plugin/plugin.json"),
    ),
    (
        ".codex-plugin/plugin.json",
        include_str!("../../../integrations/agent/.codex-plugin/plugin.json"),
    ),
    (
        "hooks/comemory-status.sh",
        include_str!("../../../integrations/agent/hooks/comemory-status.sh"),
    ),
    (
        "hooks/hooks.json",
        include_str!("../../../integrations/agent/hooks/hooks.json"),
    ),
    (
        "hooks/project-skills-curate.sh",
        include_str!("../../../integrations/agent/hooks/project-skills-curate.sh"),
    ),
    (
        "hooks/project-skills-index.sh",
        include_str!("../../../integrations/agent/hooks/project-skills-index.sh"),
    ),
    (
        "hooks/session-start.sh",
        include_str!("../../../integrations/agent/hooks/session-start.sh"),
    ),
    (
        "hooks/skill-use.sh",
        include_str!("../../../integrations/agent/hooks/skill-use.sh"),
    ),
    (
        "lib/project-skills.sh",
        include_str!("../../../integrations/agent/lib/project-skills.sh"),
    ),
    (
        "lib/project-skills-foundation.sh",
        include_str!("../../../integrations/agent/lib/project-skills-foundation.sh"),
    ),
    (
        "lib/project-skills-commands.sh",
        include_str!("../../../integrations/agent/lib/project-skills-commands.sh"),
    ),
    (
        "lib/project-skills-curation.sh",
        include_str!("../../../integrations/agent/lib/project-skills-curation.sh"),
    ),
    (
        "lib/repo-scope.sh",
        include_str!("../../../integrations/agent/lib/repo-scope.sh"),
    ),
    (
        "skills/agent-memory/SKILL.md",
        include_str!("../../../integrations/agent/skills/agent-memory/SKILL.md"),
    ),
    (
        "skills/agent-memory/scripts/comemory.sh",
        include_str!("../../../integrations/agent/skills/agent-memory/scripts/comemory.sh"),
    ),
    (
        "skills/project-skills/SKILL.md",
        include_str!("../../../integrations/agent/skills/project-skills/SKILL.md"),
    ),
    (
        "skills/project-skills/scripts/skills.sh",
        include_str!("../../../integrations/agent/skills/project-skills/scripts/skills.sh"),
    ),
];

pub(super) fn extract(root: &Path) -> Result<()> {
    let parent = root
        .parent()
        .ok_or_else(|| Error::Usage("bundle needs a parent".into()))?;
    std::fs::create_dir_all(parent)?;
    let _lock =
        crate::source::lock::FileLock::acquire(&parent.join("install.lock"), "agent-install")?;
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
