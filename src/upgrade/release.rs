//! Talking to GitHub Releases without an HTTP client in the binary:
//! `comemory upgrade` shells out to `curl` (falling back to `wget`), the same
//! tools the shell installer itself needs, so the upgrade path adds no TLS
//! stack to the crate. Two operations: resolve the `latest` redirect to a
//! tag, and download one asset to a file.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::config::env::env_parse;
use crate::prelude::*;

/// Where releases live. `COMEMORY_RELEASES_URL` overrides it — a test hook
/// so the suite can point at a loopback fixture server, not a user knob.
pub const DEFAULT_RELEASES_URL: &str = "https://github.com/Falconiere/comemory/releases";

/// The release base URL, trailing slash trimmed.
pub fn releases_url() -> Result<String> {
    Ok(env_parse::<String>("COMEMORY_RELEASES_URL")?.map_or_else(
        || DEFAULT_RELEASES_URL.to_string(),
        |u| u.trim_end_matches('/').to_string(),
    ))
}

/// The tag `<base>/latest` redirects to (`v0.19.0`).
pub fn latest_tag(base: &str) -> Result<String> {
    let url = format!("{base}/latest");
    let landed = final_url(&url)?;
    let tag = landed
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("");
    let looks_like_tag = tag
        .strip_prefix('v')
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()));
    if looks_like_tag {
        Ok(tag.to_string())
    } else {
        Err(Error::Unavailable(format!(
            "could not resolve the latest release from {url} (landed on {landed})"
        )))
    }
}

/// Download `url` to `dest`, failing on any non-2xx status.
pub fn download(url: &str, dest: &Path) -> Result<()> {
    let mut cmd = match tool()? {
        Tool::Curl => {
            let mut c = Command::new("curl");
            c.args(tls_args(url))
                .args(["-fsSL", "--retry", "2", "-o"])
                .arg(dest)
                .arg(url);
            c
        }
        Tool::Wget => {
            let mut c = Command::new("wget");
            c.args(["-q", "-O"]).arg(dest).arg(url);
            c
        }
    };
    run(&mut cmd, url).map(drop)
}

/// Follow redirects from `url` and return where they end.
fn final_url(url: &str) -> Result<String> {
    match tool()? {
        Tool::Curl => {
            let mut c = Command::new("curl");
            c.args(tls_args(url))
                .args(["-fsSLI", "-o", "/dev/null", "-w", "%{url_effective}"])
                .arg(url);
            Ok(run(&mut c, url)?.trim().to_string())
        }
        Tool::Wget => {
            // `--spider -S` prints every response's headers to stderr; the
            // last `Location:` is the final hop, and none means no redirect.
            let out = Command::new("wget")
                .args(["-q", "-S", "--spider"])
                .arg(url)
                .stdin(Stdio::null())
                .output()
                .map_err(|e| unreachable(url, &e.to_string()))?;
            let text = String::from_utf8_lossy(&out.stderr);
            if !out.status.success() {
                return Err(unreachable(url, text.trim()));
            }
            let last = text.lines().filter_map(location_header).next_back();
            Ok(last.unwrap_or_else(|| url.to_string()))
        }
    }
}

/// The URL out of a `Location:` header line, matched case-insensitively —
/// RFC 9110 header names are, and a proxy may lowercase what GitHub sends.
fn location_header(line: &str) -> Option<String> {
    let line = line.trim();
    let (name, value) = line.split_once(':')?;
    name.eq_ignore_ascii_case("location")
        .then(|| value.trim().to_string())
}

/// The TLS-hardening flags curl gets for an `https://` URL — the same set
/// the README's one-liner and `install.sh` use. A plain-`http` loopback
/// fixture URL gets none, or curl would refuse it.
fn tls_args(url: &str) -> &'static [&'static str] {
    if url.starts_with("https://") {
        &["--proto", "=https", "--proto-redir", "=https", "--tlsv1.2"]
    } else {
        &[]
    }
}

/// Run a fetch command to completion, returning its stdout. A spawn
/// failure or non-zero exit becomes `Error::Unavailable` naming the URL
/// and the tool's stderr.
fn run(cmd: &mut Command, url: &str) -> Result<String> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| unreachable(url, &e.to_string()))?;
    if !out.status.success() {
        return Err(unreachable(
            url,
            String::from_utf8_lossy(&out.stderr).trim(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn unreachable(url: &str, detail: &str) -> Error {
    Error::Unavailable(format!("could not fetch {url}: {detail}"))
}

#[derive(Clone, Copy)]
enum Tool {
    Curl,
    Wget,
}

/// `curl` if it runs, else `wget`, else an error. Probed per call: two
/// `--version` executions are cheap next to the download that follows.
fn tool() -> Result<Tool> {
    if probe("curl") {
        return Ok(Tool::Curl);
    }
    if probe("wget") {
        return Ok(Tool::Wget);
    }
    Err(Error::Unavailable(
        "neither curl nor wget is on PATH; install one to fetch releases".into(),
    ))
}

fn probe(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}
