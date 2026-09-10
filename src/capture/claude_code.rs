//! Claude Code JSONL transcript → Bash tool_use command lines.

use serde_json::Value;

use crate::prelude::*;

/// One Bash invocation recovered from a transcript record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BashCommand {
    /// ISO-8601 timestamp from the record, when present.
    pub saved_at: String,
    /// The shell command string from `tool_use.input.command`.
    pub command: String,
}

/// Walk a Claude Code JSONL transcript and collect Bash `tool_use` commands.
///
/// Malformed lines and non-Bash records are skipped rather than failing the run.
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
pub fn read_transcript_file(path: &std::path::Path) -> Result<String> {
    std::fs::read_to_string(path)
        .map_err(|e| Error::Usage(format!("cannot read transcript {}: {e}", path.display())))
}
