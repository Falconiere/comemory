//! Shell out to `curl` (falling back to `wget`) for HTTP — no TLS stack in
//! the crate. Shared by [`crate::upgrade::release`] (asset download +
//! redirect resolve) and [`crate::cloud`] (JSON device-auth / mint calls).

use std::path::Path;
use std::process::{Command, Stdio};

use crate::prelude::*;
use crate::store::random_id::random_hex;

/// One outbound HTTP request. `body` is sent as-is when present (callers set
/// `Content-Type` themselves).
#[derive(Debug, Clone)]
pub struct Request<'a> {
    /// HTTP method (`GET`, `POST`, …).
    pub method: &'a str,
    /// Absolute URL.
    pub url: &'a str,
    /// Extra request headers as `(name, value)` pairs.
    pub headers: &'a [(&'a str, &'a str)],
    /// Optional raw body (JSON string, form body, …).
    pub body: Option<&'a str>,
}

/// Status line + response body from [`exchange`].
#[derive(Debug, Clone)]
pub struct Response {
    /// HTTP status code.
    pub status: u16,
    /// Raw response body (may be empty).
    pub body: String,
}

/// Perform `req`, returning status + body. Non-2xx is **not** an error —
/// callers inspect [`Response::status`] (device-token poll needs the 400
/// `authorization_pending` body). Spawn / tool failures become
/// [`Error::Unavailable`].
pub fn exchange(req: &Request<'_>) -> Result<Response> {
    match tool()? {
        Tool::Curl => exchange_curl(req),
        Tool::Wget => exchange_wget(req),
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
    run_ok(&mut cmd, url).map(drop)
}

/// Follow redirects from `url` and return where they end.
pub fn final_url(url: &str) -> Result<String> {
    match tool()? {
        Tool::Curl => {
            let mut c = Command::new("curl");
            c.args(tls_args(url))
                .args(["-fsSLI", "-o", "/dev/null", "-w", "%{url_effective}"])
                .arg(url);
            Ok(run_ok(&mut c, url)?.trim().to_string())
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

fn exchange_curl(req: &Request<'_>) -> Result<Response> {
    validate_headers(req.headers)?;
    let tmp = tempfile_path()?;
    let mut cmd = Command::new("curl");
    cmd.args(tls_args(req.url))
        .args(["-sS", "-X", req.method, "-o"])
        .arg(&tmp)
        .args(["-w", "%{http_code}"]);
    for (name, value) in req.headers {
        cmd.arg("-H").arg(format!("{name}: {value}"));
    }
    if let Some(body) = req.body {
        cmd.arg("--data-binary").arg(body);
    }
    cmd.arg(req.url);
    let status_text = run_ok(&mut cmd, req.url);
    let body = read_body_file(&tmp, req.url);
    if let Err(e) = std::fs::remove_file(&tmp) {
        tracing::debug!(error = %e, path = %tmp.display(), "fetch temp body cleanup failed");
    }
    let status_text = status_text?;
    let body = body?;
    let status: u16 = status_text.trim().parse().map_err(|_| {
        Error::Unavailable(format!(
            "could not parse HTTP status from curl for {}: got {status_text:?}",
            req.url
        ))
    })?;
    Ok(Response { status, body })
}

fn exchange_wget(req: &Request<'_>) -> Result<Response> {
    validate_headers(req.headers)?;
    let tmp = tempfile_path()?;
    let mut cmd = Command::new("wget");
    cmd.args(["-q", "-S", "-O"]).arg(&tmp);
    for (name, value) in req.headers {
        cmd.arg("--header").arg(format!("{name}: {value}"));
    }
    match (req.method, req.body) {
        ("GET", None) => {}
        ("POST", Some(body)) => {
            cmd.arg("--post-data").arg(body);
        }
        ("POST", None) => {
            cmd.arg("--post-data").arg("");
        }
        (method, _) => {
            return Err(Error::Unavailable(format!(
                "wget fallback cannot perform {method} for {}; install curl",
                req.url
            )));
        }
    }
    cmd.arg(req.url);
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| unreachable(req.url, &e.to_string()))?;
    let headers = String::from_utf8_lossy(&out.stderr);
    let body = read_body_file(&tmp, req.url);
    if let Err(e) = std::fs::remove_file(&tmp) {
        tracing::debug!(error = %e, path = %tmp.display(), "fetch temp body cleanup failed");
    }
    let body = body?;
    let status = http_status_from_wget_headers(&headers).ok_or_else(|| {
        unreachable(
            req.url,
            if headers.trim().is_empty() {
                "wget returned no status headers"
            } else {
                headers.trim()
            },
        )
    })?;
    Ok(Response { status, body })
}

/// Reject header names/values that could inject additional HTTP headers.
///
/// Names reject CR/LF/':'/whitespace; values reject CR/LF/NUL. Blocking CR and
/// LF individually also blocks `"\r\n"` sequences passed through curl `-H` /
/// wget `--header`.
fn validate_headers(headers: &[(&str, &str)]) -> Result<()> {
    for (name, value) in headers {
        if name
            .bytes()
            .any(|b| matches!(b, b'\r' | b'\n' | b':' | b' ' | b'\t' | 0))
        {
            return Err(Error::Unavailable(
                "HTTP header name must not contain CR, LF, ':', whitespace, or NUL".into(),
            ));
        }
        if value.bytes().any(|b| matches!(b, b'\r' | b'\n' | 0)) {
            return Err(Error::Unavailable(
                "HTTP header value must not contain CR, LF, or NUL".into(),
            ));
        }
    }
    Ok(())
}

/// Last `HTTP/x.y NNN` status in wget `-S` stderr (redirects print several).
fn http_status_from_wget_headers(headers: &str) -> Option<u16> {
    headers.lines().rev().find_map(|line| {
        let line = line.trim();
        let idx = line.find("HTTP/")?;
        let rest = &line[idx + "HTTP/".len()..];
        let code = rest.split_whitespace().nth(1)?;
        code.parse().ok()
    })
}

/// The URL out of a `Location:` header line, matched case-insensitively.
fn location_header(line: &str) -> Option<String> {
    let line = line.trim();
    let (name, value) = line.split_once(':')?;
    name.eq_ignore_ascii_case("location")
        .then(|| value.trim().to_string())
}

/// TLS-hardening flags curl gets for an `https://` URL. Plain `http`
/// (loopback fixtures) gets none, or curl would refuse it.
pub fn tls_args(url: &str) -> &'static [&'static str] {
    if url.starts_with("https://") {
        &["--proto", "=https", "--proto-redir", "=https", "--tlsv1.2"]
    } else {
        &[]
    }
}

fn run_ok(cmd: &mut Command, url: &str) -> Result<String> {
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
/// `--version` executions are cheap next to the request that follows.
fn tool() -> Result<Tool> {
    if probe("curl") {
        return Ok(Tool::Curl);
    }
    if probe("wget") {
        return Ok(Tool::Wget);
    }
    Err(Error::Unavailable(
        "neither curl nor wget is on PATH; install one to reach the network".into(),
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

/// Read the curl/wget body file. Missing/unreadable after a spawn is an
/// error (not a silent empty body).
fn read_body_file(path: &Path, url: &str) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| {
        Error::Unavailable(format!(
            "could not read HTTP body for {url} from {}: {e}",
            path.display()
        ))
    })
}

/// Unpredictable path under the process temp dir for curl/wget `-o` bodies
/// (CWE-377 — avoid timestamp/PID-only names in a shared temp dir).
///
/// Creates the file with `create_new` (`O_CREAT|O_EXCL`) so a pre-planted
/// symlink at the same path cannot redirect the write; curl/wget then
/// overwrite the regular file.
fn tempfile_path() -> Result<std::path::PathBuf> {
    let path = std::env::temp_dir().join(format!("comemory-fetch-{}", random_hex(16)?));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(&path).map_err(|e| {
        Error::Unavailable(format!(
            "could not create fetch temp body file {}: {e}",
            path.display()
        ))
    })?;
    Ok(path)
}
