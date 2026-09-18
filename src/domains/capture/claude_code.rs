//! Claude Code JSONL transcript adapter (session load + Bash tool_use scan).

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::prelude::*;

/// Parsed Claude Code session ready for receipt assembly.
#[derive(Debug, Clone)]
pub struct ClaudeSession {
    /// Tool session id (`externalId` on the wire).
    pub external_id: String,
    /// Absolute path of the JSONL file read.
    pub path: PathBuf,
    /// Raw file bytes (unredacted).
    pub raw: Vec<u8>,
    /// First user-turn text (unredacted), for the receipt title.
    pub title: String,
    /// Repo label derived from `cwd` basename when present.
    pub repo: Option<String>,
    /// `gitBranch` from the first record that carries it.
    pub branch: Option<String>,
    /// Tool `version` field.
    pub tool_version: Option<String>,
    /// Earliest record timestamp (ISO 8601).
    pub started_at: String,
    /// Latest record timestamp (ISO 8601).
    pub ended_at: String,
    /// Count of `user` + `assistant` records.
    pub turn_count: u32,
}

/// One Bash invocation recovered from a transcript record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BashCommand {
    /// ISO-8601 timestamp from the record, when present.
    pub saved_at: String,
    /// The shell command string from `tool_use.input.command`.
    pub command: String,
}

#[derive(Debug, Deserialize)]
struct Line {
    #[serde(default)]
    r#type: String,
    #[serde(default, rename = "sessionId")]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default, rename = "gitBranch")]
    git_branch: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    message: Option<Value>,
}

/// Load a Claude Code session from an explicit JSONL path.
pub fn load_path(path: &Path) -> Result<ClaudeSession> {
    let raw = fs::read(path).map_err(|e| Error::Usage(format!("read {}: {e}", path.display())))?;
    parse_bytes(path.to_path_buf(), &raw)
}

/// Find `~/.claude/projects/**/<session_id>.jsonl` and load it.
pub fn load_session_id(session_id: &str) -> Result<ClaudeSession> {
    validate_session_id(session_id)?;
    let path = find_session_file(session_id)?;
    load_path(&path)
}

fn validate_session_id(session_id: &str) -> Result<()> {
    if session_id.is_empty() {
        return Err(Error::Usage("session id must not be empty".into()));
    }
    // Claude Code ids are UUID-like; refuse anything that could be a path
    // component (`.`, `..`, separators, or other punctuation).
    if !session_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(Error::Usage(
            "session id must be alphanumeric (optionally with - or _)".into(),
        ));
    }
    Ok(())
}

/// Walk a Claude Code JSONL transcript and collect Bash `tool_use` commands.
///
/// Malformed lines and non-Bash records are skipped rather than failing the run.
/// A missing `timestamp` yields an empty `saved_at` (best-effort provenance).
pub fn bash_commands_from_jsonl(text: &str) -> Vec<BashCommand> {
    text.lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let value: Value = serde_json::from_str(line).ok()?;
            let saved_at = value
                .get("timestamp")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let content = value
                .pointer("/message/content")
                .and_then(Value::as_array)?;
            let mut out = Vec::new();
            for block in content {
                if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                    continue;
                }
                if block.get("name").and_then(Value::as_str) != Some("Bash") {
                    continue;
                }
                let Some(command) = block
                    .pointer("/input/command")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                else {
                    continue;
                };
                out.push(BashCommand {
                    saved_at: saved_at.clone(),
                    command,
                });
            }
            Some(out)
        })
        .flatten()
        .collect()
}

/// Read a UTF-8 JSONL file from disk.
pub fn read_transcript_file(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .map_err(|e| Error::Usage(format!("cannot read transcript {}: {e}", path.display())))
}

fn find_session_file(session_id: &str) -> Result<PathBuf> {
    let home = dirs_home()
        .ok_or_else(|| Error::Usage("HOME unset — cannot locate Claude Code transcripts".into()))?;
    let root = home.join(".claude").join("projects");
    if !root.is_dir() {
        return Err(Error::Usage(format!(
            "no Claude Code projects dir at {} — pass --path",
            root.display()
        )));
    }
    let want = format!("{session_id}.jsonl");
    let mut hits = Vec::new();
    walk_for_name(&root, &want, &mut hits)?;
    match hits.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(Error::Usage(format!(
            "no transcript for session {session_id} under {}",
            root.display()
        ))),
        _ => Err(Error::Usage(format!(
            "multiple transcripts for session {session_id}; pass --path"
        ))),
    }
}

fn walk_for_name(dir: &Path, want: &str, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries =
        fs::read_dir(dir).map_err(|e| Error::Other(format!("read_dir {}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::Other(format!("read_dir entry: {e}")))?;
        let path = entry.path();
        if path.is_dir() {
            walk_for_name(&path, want, out)?;
        } else if path.file_name().and_then(|s| s.to_str()) == Some(want) {
            out.push(path);
        }
    }
    Ok(())
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn parse_bytes(path: PathBuf, raw: &[u8]) -> Result<ClaudeSession> {
    let text = std::str::from_utf8(raw)
        .map_err(|e| Error::Usage(format!("{} is not UTF-8: {e}", path.display())))?;
    let mut external_id: Option<String> = None;
    let mut repo: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut tool_version: Option<String> = None;
    let mut started_at: Option<String> = None;
    let mut ended_at: Option<String> = None;
    let mut title = String::new();
    let mut turn_count: u32 = 0;

    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let rec: Line = serde_json::from_str(line)
            .map_err(|e| Error::Usage(format!("{}:{idx}: json: {e}", path.display())))?;
        if let Some(id) = rec.session_id.filter(|s| !s.is_empty()) {
            external_id.get_or_insert(id);
        }
        if let (None, Some(cwd)) = (repo.as_ref(), rec.cwd.as_deref().filter(|s| !s.is_empty())) {
            repo = Path::new(cwd)
                .file_name()
                .and_then(|s| s.to_str())
                .map(str::to_string);
        }
        if branch.is_none() {
            branch = rec.git_branch.filter(|s| !s.is_empty());
        }
        if tool_version.is_none() {
            tool_version = rec.version.filter(|s| !s.is_empty());
        }
        if let Some(ts) = rec.timestamp.filter(|s| !s.is_empty()) {
            if started_at.is_none() {
                started_at = Some(ts.clone());
            }
            ended_at = Some(ts);
        }
        if rec.r#type == "user" || rec.r#type == "assistant" {
            turn_count = turn_count.saturating_add(1);
            if title.is_empty() && rec.r#type == "user" {
                title = first_text(rec.message.as_ref());
            }
        }
    }

    let external_id = external_id
        .ok_or_else(|| Error::Usage(format!("{}: no sessionId in any record", path.display())))?;
    let started_at = started_at
        .ok_or_else(|| Error::Usage(format!("{}: no timestamp in any record", path.display())))?;
    let ended_at = ended_at.unwrap_or_else(|| started_at.clone());
    if turn_count == 0 {
        return Err(Error::Usage(format!(
            "{}: no user/assistant turns",
            path.display()
        )));
    }
    if title.is_empty() {
        title.clone_from(&external_id);
    }
    if title.chars().count() > 200 {
        title = title.chars().take(200).collect();
    }

    Ok(ClaudeSession {
        external_id,
        path,
        raw: raw.to_vec(),
        title,
        repo,
        branch,
        tool_version,
        started_at,
        ended_at,
        turn_count,
    })
}

fn first_text(message: Option<&Value>) -> String {
    let Some(msg) = message else {
        return String::new();
    };
    match msg.get("content") {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Array(parts)) => {
            for part in parts {
                if part.get("type").and_then(Value::as_str) == Some("text")
                    && let Some(t) = part.get("text").and_then(Value::as_str)
                {
                    let t = t.trim();
                    if !t.is_empty() {
                        return t.to_string();
                    }
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

#[cfg(test)]
#[path = "tests/claude_code.rs"]
mod tests;
