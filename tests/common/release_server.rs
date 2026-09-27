#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    dead_code
)]
//! A loopback stand-in for GitHub Releases, for the installer and
//! `comemory upgrade` tests. It answers the three request shapes
//! `install.sh` and `domains::maintenance::upgrade::release` rely on —
//! `/latest` as a 302 to
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

use assert_cmd::cargo::cargo_bin;
use comemory::utilities::file_lock::FileLock;
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
    let completion_bin = cargo_bin("comemory");
    let quoted_completion_bin = completion_bin.to_string_lossy().replace('\'', "'\\''");
    std::fs::write(
        &stub,
        format!(
            "#!/bin/sh\ncase \"${{1:-}}\" in\n  --version) printf 'comemory {version}\\n' ;;\n  completions) exec '{quoted_completion_bin}' \"$@\" ;;\n  *) exit 64 ;;\nesac\n"
        ),
    )
    .expect("write stub binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
            .expect("chmod stub");
    }
    let archive = format!("{pkg_name}.tar.xz");
    tar_xz(&dir, &pkg_name, &archive, None);
    write_sha256_sidecar(&dir, &archive);
    copy_install_sh(&dir);
}

/// `tar -cJf` an already-populated package directory into `<dir>/<archive>`,
/// honoring `xz_opt` as `XZ_OPT` — the real binary is large enough that the
/// default xz preset dominates test time; the stub script is not.
fn tar_xz(dir: &Path, pkg_name: &str, archive: &str, xz_opt: Option<&str>) {
    let mut cmd = Command::new("tar");
    if let Some(opt) = xz_opt {
        cmd.env("XZ_OPT", opt);
    }
    let status = cmd
        .arg("-cJf")
        .arg(dir.join(archive))
        .arg("-C")
        .arg(dir)
        .arg(pkg_name)
        .status()
        .expect("spawn tar");
    assert!(status.success(), "tar -cJf failed for {archive}");
}

/// sha256 an archive already on disk and write its cargo-dist sidecar
/// (`<hash> *<file>` form) beside it.
fn write_sha256_sidecar(dir: &Path, archive: &str) {
    let bytes = std::fs::read(dir.join(archive)).expect("read archive");
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
}

/// Copy the repo's own `install.sh` beside a staged release, same as a real
/// GitHub Release asset list carries it.
fn copy_install_sh(dir: &Path) {
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh"),
        dir.join("install.sh"),
    )
    .expect("copy install.sh");
}

/// Where the cached real-binary archive for `target` lives: keyed by the
/// `comemory` binary's size and mtime, so a fresh build invalidates the
/// cache without this tracking a version string of its own.
/// `$CARGO_TARGET_TMPDIR` is set by cargo for every integration test binary;
/// anything else falls back to the OS temp dir.
fn real_release_cache_dir(bin: &Path, target: &str) -> PathBuf {
    let meta = std::fs::metadata(bin).expect("stat comemory binary");
    let mtime = meta
        .modified()
        .expect("mtime")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("mtime after epoch")
        .as_secs();
    let base = option_env!("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("release-cache")
        .join(format!("{}-{mtime}-{target}", meta.len()))
}

/// Build the real-binary archive + sidecar for `target` once per machine
/// build, guarded by a [`FileLock`] so concurrent nextest processes share
/// one build instead of each compressing the binary itself.
fn cached_real_archive(target: &str, pkg_name: &str, archive: &str) -> (PathBuf, PathBuf) {
    let bin = cargo_bin("comemory");
    let cache_dir = real_release_cache_dir(&bin, target);
    std::fs::create_dir_all(&cache_dir).expect("create cache dir");
    let lock = FileLock::acquire(&cache_dir.join(".lock"), "release-cache")
        .expect("acquire release cache lock");
    let archive_path = cache_dir.join(archive);
    let sidecar_path = cache_dir.join(format!("{archive}.sha256"));
    if !archive_path.is_file() || !sidecar_path.is_file() {
        let stage = cache_dir.join("stage");
        let _ = std::fs::remove_dir_all(&stage);
        let pkg = stage.join(pkg_name);
        std::fs::create_dir_all(&pkg).expect("create package dir");
        std::fs::copy(&bin, pkg.join("comemory")).expect("copy real binary");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(pkg.join("comemory"), std::fs::Permissions::from_mode(0o755))
                .expect("chmod real binary");
        }
        tar_xz(&stage, pkg_name, archive, Some("-0 -T0"));
        write_sha256_sidecar(&stage, archive);
        std::fs::rename(stage.join(archive), &archive_path).expect("move archive into cache");
        std::fs::rename(stage.join(format!("{archive}.sha256")), &sidecar_path)
            .expect("move sidecar into cache");
        let _ = std::fs::remove_dir_all(&stage);
    }
    drop(lock);
    (archive_path, sidecar_path)
}

/// Like [`stage_release`], but the tarball holds the real `comemory` binary
/// under test rather than a `--version` stub — for tests that need the
/// installer or `comemory upgrade` to run the actual binary (real exit
/// codes, real `--version` output) rather than a script pretending to be
/// one. The archive + sidecar are built once per binary build and cached;
/// every call after the first is a copy.
pub fn stage_real_release(root: &Path, tag: &str, target: &str) {
    let dir = root.join(tag);
    std::fs::create_dir_all(&dir).expect("create tag dir");
    let pkg_name = format!("comemory-{target}");
    let archive = format!("{pkg_name}.tar.xz");
    let (cached_archive, cached_sidecar) = cached_real_archive(target, &pkg_name, &archive);
    std::fs::copy(&cached_archive, dir.join(&archive)).expect("copy cached archive");
    std::fs::copy(&cached_sidecar, dir.join(format!("{archive}.sha256")))
        .expect("copy cached sidecar");
    copy_install_sh(&dir);
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
