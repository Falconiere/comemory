#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::domains::sync::daemon::identity`].

use comemory::config::paths::Paths;
use comemory::domains::sync::daemon::identity::{
    BinaryIdentity, SOCKET_ID_LEN, UNIT_ID_LEN, canonical_data_dir, data_dir_id,
};

#[test]
fn a_symlinked_data_dir_resolves_to_the_same_canonical_dir_and_id() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let a = canonical_data_dir(&Paths::new(real)).unwrap();
    let b = canonical_data_dir(&Paths::new(link)).unwrap();
    assert_eq!(a, b);
    assert_eq!(data_dir_id(&a, UNIT_ID_LEN), data_dir_id(&b, UNIT_ID_LEN));
    assert_eq!(data_dir_id(&a, UNIT_ID_LEN).len(), UNIT_ID_LEN);
    assert_eq!(data_dir_id(&a, SOCKET_ID_LEN).len(), SOCKET_ID_LEN);
    assert!(data_dir_id(&a, SOCKET_ID_LEN).starts_with(&data_dir_id(&a, UNIT_ID_LEN)));
}

#[test]
fn two_directories_get_two_ids() {
    let root = tempfile::tempdir().unwrap();
    let (one, two) = (root.path().join("one"), root.path().join("two"));
    std::fs::create_dir(&one).unwrap();
    std::fs::create_dir(&two).unwrap();
    assert_ne!(
        data_dir_id(&one, UNIT_ID_LEN),
        data_dir_id(&two, UNIT_ID_LEN)
    );
}

#[test]
fn a_missing_data_dir_is_unavailable() {
    let root = tempfile::tempdir().unwrap();
    let err = canonical_data_dir(&Paths::new(root.path().join("nope"))).unwrap_err();
    assert!(err.to_string().contains("cannot resolve"), "{err}");
}

#[test]
fn the_current_binary_is_this_build() {
    let me = BinaryIdentity::current().unwrap();
    assert_eq!(me.version, env!("CARGO_PKG_VERSION"));
    assert!(me.path.is_absolute() && me.path.exists());
}

// ---------------------------------------------------------------------------
// #258 D3c: preflight replaces a coordinator only when the caller is the
// file on disk at the coordinator's own path and the coordinator runs
// another file. Real files, real `stat` identities.
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};

use super::{file_id, homebrew_opt_link, preflight_replaces};
use crate::domains::sync::daemon::readiness::Readiness;

/// A real file at `dir/name` with its `<dev>:<ino>`.
fn real_file(dir: &Path, name: &str) -> (PathBuf, String) {
    let path = dir.join(name);
    std::fs::write(&path, name).unwrap();
    let id = file_id(&path).unwrap();
    (path, id)
}

fn readiness(binary: &Path, version: &str, binary_file: Option<&str>) -> Readiness {
    serde_json::from_value(serde_json::json!({
        "protocol": 1, "version": version, "binary": binary,
        "binary_file": binary_file, "pid": 1, "instance": "i",
        "started_at": "2026-09-27T00:00:00Z", "data_dir": "/d", "socket": "/d/s",
        "supervisor": "process", "store": "absent", "auth": {"state": "logged_out"},
        "sync": {"state": "idle", "interval_secs": 5, "passes": 0, "queued": false,
                 "channel": "off"}
    }))
    .unwrap()
}

fn caller(path: &Path, version: &str, file: &str) -> BinaryIdentity {
    BinaryIdentity {
        version: version.into(),
        path: path.to_path_buf(),
        file: Some(file.into()),
    }
}

#[test]
fn a_caller_that_is_the_new_file_replaces_the_coordinator_on_the_old_one() {
    let dir = tempfile::tempdir().unwrap();
    let (_, old) = real_file(dir.path(), "old");
    let (path, new) = real_file(dir.path(), "comemory");
    let running = readiness(&path, "0.50.0", Some(&old));
    assert!(preflight_replaces(
        &running,
        &caller(&path, "0.50.0", &new),
        Some(&new)
    ));
}

#[test]
fn an_older_process_still_running_the_replaced_file_never_evicts() {
    let dir = tempfile::tempdir().unwrap();
    let (_, old) = real_file(dir.path(), "old");
    let (path, new) = real_file(dir.path(), "comemory");
    // The coordinator already runs the new file; the caller is the old one.
    let running = readiness(&path, "0.50.0", Some(&new));
    assert!(!preflight_replaces(
        &running,
        &caller(&path, "0.50.0", &old),
        Some(&new)
    ));
}

#[test]
fn the_same_file_or_another_path_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let (path, id) = real_file(dir.path(), "comemory");
    let (other, other_id) = real_file(dir.path(), "copy");
    let me = caller(&path, "0.50.0", &id);
    assert!(!preflight_replaces(
        &readiness(&path, "0.50.0", Some(&id)),
        &me,
        Some(&id)
    ));
    assert!(!preflight_replaces(
        &readiness(&other, "0.50.0", Some(&other_id)),
        &me,
        Some(&id)
    ));
}

#[test]
fn a_coordinator_predating_binary_file_is_compared_by_version() {
    let dir = tempfile::tempdir().unwrap();
    let (path, id) = real_file(dir.path(), "comemory");
    let me = caller(&path, "0.50.1", &id);
    assert!(preflight_replaces(
        &readiness(&path, "0.50.0", None),
        &me,
        Some(&id)
    ));
    assert!(!preflight_replaces(
        &readiness(&path, "0.50.1", None),
        &me,
        Some(&id)
    ));
}

// ---------------------------------------------------------------------------
// homebrew-tap#1: Homebrew keeps the old keg after `brew upgrade` (macOS), so
// preflight replaces a coordinator on a keg the `opt` link no longer names.
// Real directories, real files and a real `opt` symlink, laid out as brew does.
// ---------------------------------------------------------------------------

/// `<prefix>/Cellar/comemory/<keg>/bin/comemory` for each keg, `opt/comemory`
/// linked to `linked`; returns the canonical prefix.
fn brew_prefix(root: &Path, kegs: &[&str], linked: &str) -> PathBuf {
    let prefix = root.join("homebrew");
    for keg in kegs {
        let bin = prefix.join("Cellar/comemory").join(keg).join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("comemory"), keg).unwrap();
    }
    std::fs::create_dir_all(prefix.join("opt")).unwrap();
    std::os::unix::fs::symlink(
        Path::new("../Cellar/comemory").join(linked),
        prefix.join("opt/comemory"),
    )
    .unwrap();
    std::fs::canonicalize(prefix).unwrap()
}

fn keg_bin(prefix: &Path, keg: &str) -> PathBuf {
    prefix
        .join("Cellar/comemory")
        .join(keg)
        .join("bin/comemory")
}

fn keg_caller(prefix: &Path, keg: &str, version: &str) -> BinaryIdentity {
    let path = keg_bin(prefix, keg);
    let id = file_id(&path).unwrap();
    caller(&path, version, &id)
}

#[test]
fn the_linked_keg_replaces_a_coordinator_on_the_keg_brew_kept() {
    let root = tempfile::tempdir().unwrap();
    let prefix = brew_prefix(root.path(), &["0.51.0", "0.52.0"], "0.52.0");
    let old = keg_bin(&prefix, "0.51.0");
    let running = readiness(&old, "0.51.0", file_id(&old).as_deref());
    let me = keg_caller(&prefix, "0.52.0", "0.52.0");
    assert!(preflight_replaces(&running, &me, me.file.as_deref()));
    // A revision bump is a new keg of the same version: replaced too.
    let root = tempfile::tempdir().unwrap();
    let prefix = brew_prefix(root.path(), &["0.51.0", "0.51.0_1"], "0.51.0_1");
    let old = keg_bin(&prefix, "0.51.0");
    let running = readiness(&old, "0.51.0", file_id(&old).as_deref());
    let me = keg_caller(&prefix, "0.51.0_1", "0.51.0");
    assert!(preflight_replaces(&running, &me, me.file.as_deref()));
}

#[test]
fn a_caller_on_a_keg_brew_no_longer_links_never_evicts() {
    let root = tempfile::tempdir().unwrap();
    let prefix = brew_prefix(root.path(), &["0.51.0", "0.52.0"], "0.52.0");
    let new = keg_bin(&prefix, "0.52.0");
    let running = readiness(&new, "0.52.0", file_id(&new).as_deref());
    let old_caller = keg_caller(&prefix, "0.51.0", "0.51.0");
    assert!(!preflight_replaces(
        &running,
        &old_caller,
        old_caller.file.as_deref()
    ));
}

#[test]
fn another_formula_prefix_or_a_non_keg_path_is_kept() {
    let root = tempfile::tempdir().unwrap();
    let prefix = brew_prefix(root.path(), &["0.51.0", "0.52.0"], "0.52.0");
    let me = keg_caller(&prefix, "0.52.0", "0.52.0");
    let other = tempfile::tempdir().unwrap();
    let other_prefix = brew_prefix(other.path(), &["0.51.0"], "0.51.0");
    let elsewhere = keg_bin(&other_prefix, "0.51.0");
    assert!(!preflight_replaces(
        &readiness(&elsewhere, "0.51.0", file_id(&elsewhere).as_deref()),
        &me,
        me.file.as_deref()
    ));
    let other_formula = prefix.join("Cellar/comemory-dev/0.51.0/bin/comemory");
    std::fs::create_dir_all(other_formula.parent().unwrap()).unwrap();
    std::fs::write(&other_formula, "dev").unwrap();
    assert!(!preflight_replaces(
        &readiness(&other_formula, "0.51.0", file_id(&other_formula).as_deref()),
        &me,
        me.file.as_deref()
    ));
    let (plain, plain_id) = real_file(root.path(), "comemory");
    assert!(!preflight_replaces(
        &readiness(&plain, "0.51.0", Some(&plain_id)),
        &me,
        me.file.as_deref()
    ));
}

#[test]
fn the_opt_link_is_named_only_for_the_linked_keg() {
    let root = tempfile::tempdir().unwrap();
    let prefix = brew_prefix(root.path(), &["0.51.0", "0.52.0"], "0.52.0");
    assert_eq!(
        homebrew_opt_link(&keg_bin(&prefix, "0.52.0")),
        Some(prefix.join("opt/comemory/bin/comemory"))
    );
    assert_eq!(homebrew_opt_link(&keg_bin(&prefix, "0.51.0")), None);
    let (plain, _) = real_file(root.path(), "comemory");
    assert_eq!(homebrew_opt_link(&plain), None);
}
