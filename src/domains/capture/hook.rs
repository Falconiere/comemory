//! The Claude Code `SessionEnd` contract, owned end to end: the hook command
//! written into `settings.json`, and the payload that command hands back.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::prelude::*;

/// Marker comment embedded in the hook command so re-installs are detectable.
pub const HOOK_MARKER: &str = "comemory-capture-session-end";

/// Project-local settings file the hook is installed into when the caller
/// names none.
///
/// Deliberately **relative**: it resolves against the invoking process's
/// current directory, so `capture install-hook` writes the hook into the
/// project you are standing in rather than a single machine-wide file. Callers
/// wanting a fixed location pass an absolute path to [`install`] instead.
pub const DEFAULT_SETTINGS_PATH: &str = ".claude/settings.json";

/// Default command the SessionEnd hook runs.
pub fn hook_command() -> String {
    format!("comemory capture session --from-hook # {HOOK_MARKER}")
}

/// The `SessionEnd` payload as [`hook_command`] receives it on stdin.
#[derive(Debug, Deserialize)]
struct SessionEndPayload {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    transcript_path: Option<String>,
}

/// Decode a `SessionEnd` payload into the transcript path / session id to
/// capture.
///
/// At least one of the two is `Some`; an empty string counts as absent, since
/// the tool writes `""` rather than omitting the field.
pub fn session_end_target(raw: &str) -> Result<(Option<PathBuf>, Option<String>)> {
    let payload: SessionEndPayload = serde_json::from_str(raw.trim())
        .map_err(|e| Error::Usage(format!("--from-hook expects SessionEnd JSON on stdin: {e}")))?;
    let path = payload
        .transcript_path
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let session_id = payload.session_id.filter(|s| !s.is_empty());
    if path.is_none() && session_id.is_none() {
        return Err(Error::Usage(
            "SessionEnd payload missing session_id and transcript_path".into(),
        ));
    }
    Ok((path, session_id))
}

/// Install (or refresh) the SessionEnd hook in `settings.json`.
pub fn install(settings_path: &Path, force: bool) -> Result<InstallReport> {
    let mut root = if settings_path.exists() {
        let raw = fs::read_to_string(settings_path)?;
        if raw.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&raw)
                .map_err(|e| Error::Usage(format!("{}: {e}", settings_path.display())))?
        }
    } else {
        json!({})
    };
    if !root.is_object() {
        return Err(Error::Usage(format!(
            "{}: root must be a JSON object",
            settings_path.display()
        )));
    }

    let hooks = root
        .as_object_mut()
        .ok_or_else(|| Error::Other("settings root lost object shape".into()))?
        .entry("hooks")
        .or_insert_with(|| json!({}));
    if !hooks.is_object() {
        return Err(Error::Usage(format!(
            "{}: hooks must be an object",
            settings_path.display()
        )));
    }

    let session_end = hooks
        .as_object_mut()
        .ok_or_else(|| Error::Other("hooks lost object shape".into()))?
        .entry("SessionEnd")
        .or_insert_with(|| json!([]));

    if session_end.as_array().is_some_and(|a| !a.is_empty())
        && !force
        && !contains_marker(session_end)
    {
        return Err(Error::Usage(format!(
            "{} already has SessionEnd hooks — pass --force to replace comemory's entry",
            settings_path.display()
        )));
    }

    // Drop prior comemory-marked entries, keep others when force-merging.
    let mut kept: Vec<Value> = session_end
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| !entry_has_marker(entry))
        .collect();
    kept.push(json!({
        "hooks": [{
            "type": "command",
            "command": hook_command(),
        }]
    }));
    *session_end = Value::Array(kept);

    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let pretty = serde_json::to_string_pretty(&root)?;
    atomic_write(settings_path, format!("{pretty}\n").as_bytes())?;
    Ok(InstallReport {
        path: settings_path.to_path_buf(),
        command: hook_command(),
    })
}

/// Report from a successful install.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InstallReport {
    /// Settings file written.
    pub path: PathBuf,
    /// Hook command installed.
    pub command: String,
}

fn contains_marker(session_end: &Value) -> bool {
    session_end
        .as_array()
        .into_iter()
        .flatten()
        .any(entry_has_marker)
}

fn entry_has_marker(entry: &Value) -> bool {
    let Some(hooks) = entry.get("hooks").and_then(Value::as_array) else {
        return false;
    };
    hooks.iter().any(|h| {
        h.get("command")
            .and_then(Value::as_str)
            .is_some_and(|c| c.contains(HOOK_MARKER))
    })
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    // Append `.tmp` rather than replacing the extension (`settings.json` →
    // `settings.json.tmp`), and remove the temp file if the rename fails.
    let mut tmp_os = path.as_os_str().to_owned();
    tmp_os.push(".tmp");
    let tmp = PathBuf::from(tmp_os);
    fs::write(&tmp, bytes)?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/hook.rs"]
mod tests;
