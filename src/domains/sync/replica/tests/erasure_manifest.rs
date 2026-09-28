#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/sync/replica/erasure_manifest.rs` (#256,
//! B-4): real files under a real data directory, real fsyncs, and the chain
//! checked by hashing the bytes actually on disk — a truncated, torn or
//! edited manifest is not established, and two real processes appending at
//! once under `identity.lock` keep the chain whole.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use comemory::config::Paths;
use comemory::domains::sync::replica::erasure_manifest::{self, Entry, GENESIS, Manifest};
use comemory::domains::sync::replica::identity::{self, MANIFEST_FILE};
use comemory::utilities::digest::sha256_hex;

const WAIT: Duration = Duration::from_secs(30);

fn dir() -> (tempfile::TempDir, Paths, PathBuf) {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    let manifest = identity::file(&paths, MANIFEST_FILE);
    (home, paths, manifest)
}

fn digest(n: usize) -> String {
    sha256_hex(format!("payload {n}").as_bytes())
}

/// Append `n` entries named `key-0` … under the lock, one lock per append.
fn append_n(paths: &Paths, manifest: &Path, prefix: &str, n: usize) {
    for i in 0..n {
        let held = identity::lock(paths, WAIT).expect("lock");
        let digests = vec![digest(i)];
        erasure_manifest::append(
            &held,
            manifest,
            &Entry {
                kind: "memory",
                key: &format!("{prefix}-{i}"),
                digests: &digests,
                erased_at: "2026-09-25T10:00:00Z",
            },
        )
        .expect("append");
    }
}

fn read(manifest: &Path) -> Manifest {
    erasure_manifest::read(manifest)
        .expect("read")
        .expect("a manifest")
}

/// The raw lines on disk, without their newlines.
fn raw_lines(manifest: &Path) -> Vec<Vec<u8>> {
    let bytes = fs::read(manifest).expect("bytes");
    bytes
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

#[test]
fn appended_lines_chain_by_the_hash_of_the_bytes_on_disk() {
    let (_home, paths, manifest) = dir();
    assert!(
        erasure_manifest::read(&manifest).expect("read").is_none(),
        "no file, no manifest"
    );

    append_n(&paths, &manifest, "m", 3);

    let found = read(&manifest);
    assert!(found.intact);
    assert_eq!(found.lines.len(), 3);
    assert!(found.established(3));
    assert!(
        !found.established(4),
        "short of the count is not established"
    );
    let raw = raw_lines(&manifest);
    assert_eq!(found.lines[0].prev, GENESIS);
    assert_eq!(found.lines[1].prev, sha256_hex(&raw[0]));
    assert_eq!(found.lines[2].prev, sha256_hex(&raw[1]));
    assert_eq!(found.lines[2].key, "m-2");
    assert_eq!(found.lines[2].digests, vec![digest(2)]);
}

#[test]
fn a_truncated_manifest_is_not_established() {
    let (_home, paths, manifest) = dir();
    append_n(&paths, &manifest, "m", 3);
    let bytes = fs::read(&manifest).expect("bytes");

    // Whole last line gone: what is left chains, but it is short.
    let two = raw_lines(&manifest)[..2].concat().len() + 2;
    fs::write(&manifest, &bytes[..two]).expect("truncate");
    let short = read(&manifest);
    assert!(short.intact && short.lines.len() == 2);
    assert!(!short.established(3));

    // Cut mid-line: the torn tail is not intact at all.
    fs::write(&manifest, &bytes[..bytes.len() - 10]).expect("tear");
    let torn = read(&manifest);
    assert!(!torn.intact);
    assert_eq!(
        torn.lines.len(),
        2,
        "the lines before the tear still verify"
    );
    assert!(!torn.established(2), "a torn manifest is never established");
}

#[test]
fn an_edited_line_breaks_the_chain_at_the_line_after_it() {
    let (_home, paths, manifest) = dir();
    append_n(&paths, &manifest, "m", 3);
    let text = fs::read_to_string(&manifest).expect("text");
    let raw = raw_lines(&manifest);
    let second = String::from_utf8(raw[1].clone()).expect("utf8");

    // Drop the digest a merge would have erased — the line itself still parses.
    let edited = second.replace(&digest(1), &digest(99));
    fs::write(&manifest, text.replace(&second, &edited)).expect("edit");

    let found = read(&manifest);
    assert!(!found.intact, "the third line's prev no longer matches");
    assert_eq!(found.lines.len(), 2);
    assert!(!found.established(0));
}

#[test]
fn an_append_after_a_torn_tail_cuts_it_and_keeps_the_chain_whole() {
    let (_home, paths, manifest) = dir();
    append_n(&paths, &manifest, "m", 2);
    let mut bytes = fs::read(&manifest).expect("bytes");
    bytes.extend_from_slice(br#"{"v":1,"kind":"memory","key":"half"#);
    fs::write(&manifest, &bytes).expect("a crash mid-append");
    assert!(!read(&manifest).intact);

    append_n(&paths, &manifest, "n", 1);

    let found = read(&manifest);
    assert!(found.intact, "the torn half-line was cut before the append");
    let keys: Vec<&str> = found.lines.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(keys, vec!["m-0", "m-1", "n-0"]);
}

#[test]
fn create_writes_a_whole_chain_and_leaves_no_temporary_file() {
    let (_home, paths, manifest) = dir();
    let held = identity::lock(&paths, WAIT).expect("lock");
    let (a, b) = (vec![digest(1)], vec![digest(2), digest(3)]);
    let entries = [
        Entry {
            kind: "memory",
            key: "a1b2c3d4",
            digests: &a,
            erased_at: "2026-09-25T10:00:00Z",
        },
        Entry {
            kind: "document_revision",
            key: "f00d",
            digests: &b,
            erased_at: "2026-09-25T10:01:00Z",
        },
    ];

    erasure_manifest::create(&held, &manifest, &entries).expect("create");

    let found = read(&manifest);
    assert!(found.established(2));
    assert_eq!(found.lines[1].digests, b);
    let leftovers: Vec<String> = fs::read_dir(manifest.parent().expect("dir"))
        .expect("list")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|n| {
            Path::new(n)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("tmp"))
        })
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

/// Env var naming the data directory a second process works in.
const CHILD_DIR: &str = "COMEMORY_TEST_MANIFEST_CHILD_DIR";
/// Env var telling the second process to only try the lock, briefly.
const CHILD_PROBE: &str = "COMEMORY_TEST_MANIFEST_CHILD_PROBE";

/// Run this test binary's `test` in a second process, working in `paths`.
fn second_process(test: &str, paths: &Paths, probe: bool) -> Command {
    let test = format!(
        "{}::{test}",
        module_path!().trim_start_matches("comemory::")
    );
    let mut command = Command::new(std::env::current_exe().expect("test binary"));
    command
        .args(["--exact", &test, "--nocapture"])
        .env(CHILD_DIR, paths.data_dir());
    if probe {
        command.env(CHILD_PROBE, "1");
    }
    command
}

#[test]
fn identity_lock_excludes_a_second_process_until_it_is_released() {
    if let Some(child_dir) = std::env::var_os(CHILD_DIR) {
        // The second process: the lock must be refused while the first holds it.
        let paths = Paths::new(PathBuf::from(child_dir));
        let held = identity::lock(&paths, Duration::from_millis(300));
        let expect_busy = std::env::var_os(CHILD_PROBE).is_some();
        assert_eq!(
            matches!(held, Err(comemory::prelude::Error::Busy(_))),
            expect_busy,
            "{held:?}"
        );
        return;
    }
    let (_home, paths, _manifest) = dir();
    let test = "identity_lock_excludes_a_second_process_until_it_is_released";
    let held = identity::lock(&paths, WAIT).expect("lock");

    let refused = second_process(test, &paths, true).status().expect("run");
    drop(held);
    let granted = second_process(test, &paths, false).status().expect("run");

    assert!(refused.success(), "a second process took the held lock");
    assert!(granted.success(), "the released lock was not granted");
}
/// Appends each process makes.
const EACH: usize = 25;

#[test]
fn two_processes_appending_at_once_keep_the_chain_whole() {
    if let Some(child_dir) = std::env::var_os(CHILD_DIR) {
        // The second process: wait for the go signal, then append.
        let paths = Paths::new(PathBuf::from(child_dir));
        let go = paths.data_dir().join("go");
        fs::write(paths.data_dir().join("ready"), b"").expect("ready");
        let deadline = Instant::now() + WAIT;
        while !go.exists() {
            assert!(Instant::now() < deadline, "never told to go");
            std::thread::sleep(Duration::from_millis(5));
        }
        append_n(
            &paths,
            &identity::file(&paths, MANIFEST_FILE),
            "child",
            EACH,
        );
        return;
    }
    let (_home, paths, manifest) = dir();
    let mut child = second_process(
        "two_processes_appending_at_once_keep_the_chain_whole",
        &paths,
        false,
    )
    .spawn()
    .expect("spawn the second process");
    let deadline = Instant::now() + WAIT;
    while !paths.data_dir().join("ready").exists() {
        assert!(
            Instant::now() < deadline,
            "the second process never started"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    fs::write(paths.data_dir().join("go"), b"").expect("go");

    append_n(&paths, &manifest, "parent", EACH);
    let status = child.wait().expect("wait");
    assert!(status.success(), "the second process failed: {status}");

    let found = read(&manifest);
    assert!(found.intact, "interleaved appends broke the chain");
    assert!(found.established(u64::try_from(2 * EACH).expect("count")));
    let children = found
        .lines
        .iter()
        .filter(|l| l.key.starts_with("child-"))
        .count();
    assert_eq!(children, EACH, "every append of both processes landed");
}
