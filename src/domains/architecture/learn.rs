//! `architecture learn`: hand the scaffold to the agent command the caller
//! named, and validate whatever it prints.
//!
//! comemory ships no command string, reads none from disk, and detects no
//! installed agent: the template arrives on the invocation that uses it. What
//! lives here is a substitution, a spawn under a deadline, and the same
//! validated save path every other route uses.

use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::domains::architecture::save::Saved;
use crate::domains::architecture::{extract, prompt, save, scaffold};
use crate::prelude::*;
use crate::utilities::context::Ctx;

/// Placeholder replaced with the path of the written prompt file.
const FILE_PLACEHOLDER: &str = "{prompt_file}";

/// Placeholder replaced with the prompt text itself, shell-quoted.
const TEXT_PLACEHOLDER: &str = "{prompt}";

/// Bytes of the child's stderr echoed when it fails.
const STDERR_TAIL: usize = 400;

/// `architecture learn` knobs.
#[derive(Debug, Clone)]
pub struct Options {
    /// Shell command template; must contain one of the two placeholders.
    pub command: String,
    /// Child deadline in seconds.
    pub timeout_secs: u64,
    /// Write the prompt and stop, spawning nothing.
    pub dry_run: bool,
    /// Scaffold knobs the prompt is built from.
    pub scaffold: scaffold::Options,
}

/// What a learn run produced.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Learned {
    /// Path of the prompt file handed to the agent.
    pub prompt_path: String,
    /// The command actually run, after substitution. `None` on a dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The save result. `None` on a dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved: Option<Saved>,
}

/// Scaffold, prompt, spawn, extract, validate, save.
pub async fn run(ctx: &mut Ctx<'_>, repo: &str, opts: &Options) -> Result<Learned> {
    let scaffolded = scaffold::run(ctx.conn()?, repo, &opts.scaffold)?;
    let text = prompt::build(repo, &scaffolded)?;
    let path = std::env::temp_dir().join(format!("comemory-architecture-{repo}-prompt.md"));
    std::fs::write(&path, &text)?;
    let prompt_path = path.to_string_lossy().to_string();

    let command = substitute(&opts.command, &prompt_path, &text)?;
    if opts.dry_run {
        return Ok(Learned {
            prompt_path,
            command: None,
            saved: None,
        });
    }

    let stdout = spawn(&command, opts.timeout_secs).await?;
    let model = serde_json::from_str(extract::model_json(&stdout))?;
    let saved = save::run(ctx, repo, &model)?;
    Ok(Learned {
        prompt_path,
        command: Some(command),
        saved: Some(saved),
    })
}

/// Substitute the prompt into the template. A template naming neither
/// placeholder is refused before anything is spawned.
fn substitute(template: &str, prompt_path: &str, text: &str) -> Result<String> {
    if template.contains(FILE_PLACEHOLDER) {
        return Ok(template.replace(FILE_PLACEHOLDER, prompt_path));
    }
    if template.contains(TEXT_PLACEHOLDER) {
        return Ok(template.replace(TEXT_PLACEHOLDER, &shell_quote(text)));
    }
    Err(Error::Usage(format!(
        "--command must contain {FILE_PLACEHOLDER} or {TEXT_PLACEHOLDER}, got {template:?}"
    )))
}

/// Single-quote `text` for `sh -c`, closing and reopening around each quote.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Run `command` through `sh -c` under a deadline, returning its stdout.
/// `kill_on_drop` makes the timeout path terminate and reap the child.
async fn spawn(command: &str, timeout_secs: u64) -> Result<String> {
    let child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let deadline = Duration::from_secs(timeout_secs);
    let output = match tokio::time::timeout(deadline, child.wait_with_output()).await {
        Ok(result) => result?,
        Err(_) => {
            return Err(Error::Other(format!(
                "agent command timed out after {timeout_secs}s: {command}"
            )));
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail = stderr
            .char_indices()
            .nth(stderr.chars().count().saturating_sub(STDERR_TAIL))
            .map_or(stderr.as_ref(), |(i, _)| &stderr[i..]);
        return Err(Error::Other(format!(
            "agent command failed ({}): {tail}",
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
