#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! A loopback stand-in for GitHub Releases, for the installer and
//! `comemory upgrade` tests. It answers the three request shapes
//! `install.sh` and `upgrade::release` rely on — `/latest` as a 302 to
//! `/tag/<latest>`, `/tag/<t>` as 200, `/download/<tag>/<file>` from a
//! directory — over a real socket, with real files, real tarballs, and real
//! checksums. Nothing is mocked in-process: the binary under test still
//! shells out to `curl`, which still speaks HTTP to this.
//!
//! Each consuming binary `#[path]`-includes this file directly, matching
//! `cli_bin.rs`.

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

/// A running fixture server. Lives until the process exits.
pub struct ReleaseServer {
    /// `http://127.0.0.1:<port>` — pass as `COMEMORY_RELEASES_URL`.
    pub base: String,
}

impl ReleaseServer {
    /// Serve `root` (holding `<tag>/<asset>` files) with `latest` as the
    /// tag `/latest` redirects to.
    pub fn start(root: PathBuf, latest: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        let base = format!("http://127.0.0.1:{port}");
        let latest = latest.to_string();
        let thread_base = base.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = handle(stream, &root, &latest, &thread_base);
            }
        });
        Self { base }
    }
}

/// Read one request, drain its headers, write one response, close.
fn handle(mut stream: TcpStream, root: &Path, latest: &str, base: &str) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header == "\r\n" || header == "\n" {
            break;
        }
    }
    let (status, extra, body) = route(&path, root, latest, base);
    let mut head = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for line in extra {
        head.push_str(&line);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if method != "HEAD" {
        stream.write_all(&body)?;
    }
    stream.flush()
}

fn route(
    path: &str,
    root: &Path,
    latest: &str,
    base: &str,
) -> (&'static str, Vec<String>, Vec<u8>) {
    if path == "/latest" {
        return (
            "302 Found",
            vec![format!("Location: {base}/tag/{latest}")],
            Vec::new(),
        );
    }
    if path.starts_with("/tag/") {
        return (
            "200 OK",
            vec!["Content-Type: text/html".to_string()],
            b"<html>release</html>".to_vec(),
        );
    }
    if let Some(rest) = path.strip_prefix("/download/")
        && let Ok(bytes) = std::fs::read(root.join(rest))
    {
        return (
            "200 OK",
            vec!["Content-Type: application/octet-stream".to_string()],
            bytes,
        );
    }
    ("404 Not Found", Vec::new(), b"not found".to_vec())
}

/// The cargo-dist target triple for this host, or `None` where comemory
/// publishes no build (the tests then skip rather than fake a triple the
/// installer would refuse).
pub fn host_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        _ => None,
    }
}

/// Lay out one release under `root/<tag>/`, exactly the assets a real one
/// carries that the installer touches: `comemory-<target>.tar.xz` holding
/// `comemory-<target>/comemory` — a tiny script answering `--version` with
/// `comemory <version>` — its `.sha256` sidecar in cargo-dist's
/// `<hash> *<file>` form, and the repo's own `install.sh`.
pub fn stage_release(root: &Path, tag: &str, target: &str) {
    let version = tag.trim_start_matches('v');
    let dir = root.join(tag);
    let pkg_name = format!("comemory-{target}");
    let pkg = dir.join(&pkg_name);
    std::fs::create_dir_all(&pkg).expect("create package dir");
    let stub = pkg.join("comemory");
    std::fs::write(
        &stub,
        format!("#!/bin/sh\nprintf 'comemory {version}\\n'\n"),
    )
    .expect("write stub binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
            .expect("chmod stub");
    }
    let archive = format!("{pkg_name}.tar.xz");
    let status = Command::new("tar")
        .arg("-cJf")
        .arg(dir.join(&archive))
        .arg("-C")
        .arg(&dir)
        .arg(&pkg_name)
        .status()
        .expect("spawn tar");
    assert!(status.success(), "tar -cJf failed for {archive}");
    let bytes = std::fs::read(dir.join(&archive)).expect("read archive");
    let hash = Sha256::digest(&bytes)
        .iter()
        .fold(String::new(), |mut acc, b| {
            let _ = write!(acc, "{b:02x}");
            acc
        });
    std::fs::write(
        dir.join(format!("{archive}.sha256")),
        format!("{hash} *{archive}\n"),
    )
    .expect("write sidecar");
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh"),
        dir.join("install.sh"),
    )
    .expect("copy install.sh");
}

/// Whether the tools the fixture and the installer need are on PATH. The
/// tests skip (return early) when they are not. `sh` is probed with `-c`:
/// dash rejects `--version`.
pub fn tooling_present() -> bool {
    let probes: [(&str, &[&str]); 4] = [
        ("tar", &["--version"]),
        ("xz", &["--version"]),
        ("curl", &["--version"]),
        ("sh", &["-c", "exit 0"]),
    ];
    probes.iter().all(|(tool, args)| {
        Command::new(tool)
            .args(*args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}
