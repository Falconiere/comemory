#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    dead_code
)]
//! The pinned pre-replication engine: the real `v0.43.2` release binary, the
//! last release whose migrations end before `0022_replica_journal`.
//!
//! The asset for this host is downloaded once from the GitHub release into
//! `target/legacy-engine/`, checked against the SHA-256 committed in
//! `scripts/replication/legacy-engine.json`, and must print its own version.
//! A host with no published asset, a missing network or a checksum mismatch
//! FAILS the test with the command that would fetch it — the recovery suites
//! never skip, and never fall back to whatever `comemory` is on `PATH`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use comemory::utilities::file_lock::FileLock;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The version the pinned binary must report.
pub const VERSION: &str = "0.43.2";

/// The pin file, read at test time so the checksum lives in one place.
const PIN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/scripts/replication/legacy-engine.json"
);

/// The release target triple for this host, or a failure naming the host.
fn host_target() -> &'static str {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("aarch64", "macos") => "aarch64-apple-darwin",
        ("x86_64", "linux") => "x86_64-unknown-linux-gnu",
        ("aarch64", "linux") => "aarch64-unknown-linux-gnu",
        (arch, os) => panic!(
            "no pinned v{VERSION} release asset for {arch}-{os}; the recovery suites run on \
             aarch64-apple-darwin, x86_64-unknown-linux-gnu and aarch64-unknown-linux-gnu"
        ),
    }
}

/// Where downloaded assets and their extracted binaries are cached.
fn cache_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("target/legacy-engine")
}

/// Lowercase hex SHA-256 of `path`'s bytes.
fn sha256_of(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("read downloaded asset");
    hex(&Sha256::digest(&bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One release asset as the pin file names it.
struct Asset {
    file: String,
    sha256: String,
    url: String,
    tag: String,
}

/// Read this host's asset from the pin file.
fn pinned_asset() -> Asset {
    let pin: Value =
        serde_json::from_str(&std::fs::read_to_string(PIN).expect("read pin")).expect("pin json");
    assert_eq!(
        pin["version"], VERSION,
        "the pin names the expected version"
    );
    let asset = &pin["assets"][host_target()];
    let file = asset["file"].as_str().expect("asset file").to_string();
    let tag = pin["tag"].as_str().expect("tag").to_string();
    let url = format!(
        "{}/{tag}/{file}",
        pin["base_url"].as_str().expect("base url")
    );
    Asset {
        sha256: asset["sha256"].as_str().expect("asset sha256").to_string(),
        file,
        url,
        tag,
    }
}

/// The pinned binary, downloaded and verified on first use. Concurrent test
/// processes serialize on a lock beside the cache, so one download serves
/// every suite.
pub fn binary() -> PathBuf {
    let asset = pinned_asset();
    let dir = cache_root().join(&asset.tag).join(host_target());
    std::fs::create_dir_all(&dir).expect("create legacy cache");
    let _lock = FileLock::acquire(&dir.join(".lock"), "legacy-engine").expect("lock cache");
    let bin = dir.join("comemory");
    if !bin.exists() {
        let archive = download(&dir, &asset);
        unpack(&dir, &archive, &asset.file, &bin);
    }
    verified(bin)
}

/// The verified archive in `dir`, fetched when absent or not the pinned bytes.
fn download(dir: &Path, asset: &Asset) -> PathBuf {
    let archive = dir.join(&asset.file);
    if !archive.exists() || sha256_of(&archive) != asset.sha256 {
        let partial = dir.join(format!("{}.partial", asset.file));
        let status = Command::new("curl")
            .args(["-fsSL", "--retry", "3", "-o"])
            .arg(&partial)
            .arg(&asset.url)
            .status()
            .expect("spawn curl");
        assert!(
            status.success(),
            "could not download the pinned legacy engine; fetch it with:\n  curl -fL -o {} {}",
            archive.display(),
            asset.url
        );
        std::fs::rename(&partial, &archive).expect("place archive");
    }
    assert_eq!(
        sha256_of(&archive),
        asset.sha256,
        "{} does not match scripts/replication/legacy-engine.json",
        archive.display()
    );
    archive
}

/// Extract the release archive and move its binary to `bin`.
fn unpack(dir: &Path, archive: &Path, file: &str, bin: &Path) {
    let unpack = dir.join("unpack");
    let _ = std::fs::remove_dir_all(&unpack);
    std::fs::create_dir_all(&unpack).expect("create unpack dir");
    let status = Command::new("tar")
        .arg("-xJf")
        .arg(archive)
        .arg("-C")
        .arg(&unpack)
        .status()
        .expect("spawn tar");
    assert!(
        status.success(),
        "tar could not unpack {}",
        archive.display()
    );
    let extracted = unpack
        .join(file.trim_end_matches(".tar.xz"))
        .join("comemory");
    std::fs::rename(&extracted, bin).expect("place legacy binary");
}

/// `bin` after proving it is the pinned release, not whatever sat there.
fn verified(bin: PathBuf) -> PathBuf {
    let out = Command::new(&bin)
        .arg("--version")
        .output()
        .expect("run legacy --version");
    let printed = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        printed.trim(),
        format!("comemory {VERSION}"),
        "{} is not the pinned release",
        bin.display()
    );
    bin
}

/// Run the legacy binary over `data_dir` with `args`, returning its output.
pub fn run(data_dir: &Path, args: &[&str]) -> Output {
    Command::new(binary())
        .env("COMEMORY_DATA_DIR", data_dir)
        .env("HOME", data_dir)
        .args(args)
        .output()
        .expect("run legacy comemory")
}

/// Run the legacy binary with `--json` and parse its stdout, failing loudly
/// on a non-zero exit.
pub fn run_json(data_dir: &Path, args: &[&str]) -> Value {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    let out = run(data_dir, &full);
    assert!(
        out.status.success(),
        "legacy comemory {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("legacy json stdout")
}
