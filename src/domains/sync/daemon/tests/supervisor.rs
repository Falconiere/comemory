#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::domains::sync::daemon::supervisor`].

use comemory::config::Paths;
use comemory::domains::sync::daemon::supervisor::{Kind, detect_for, for_data_dir, plan};

fn canonical() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    (dir, canonical)
}

#[test]
fn launchd_and_systemd_units_are_named_by_the_data_directory_id() {
    let (_dir, canonical) = canonical();
    let launchd = plan(&canonical, Kind::Launchd).unwrap();
    let systemd = plan(&canonical, Kind::Systemd).unwrap();
    assert!(launchd.name.starts_with("io.comemory.sync."));
    assert!(launchd.path.ends_with(format!("{}.plist", launchd.name)));
    assert!(systemd.name.starts_with("comemory-sync-") && systemd.name.ends_with(".service"));
    assert_ne!(
        launchd.name, "io.comemory.sync",
        "must not collide with the legacy label"
    );
}

#[test]
fn two_directories_get_two_unit_names() {
    let (_a, a) = canonical();
    let (_b, b) = canonical();
    assert_ne!(
        plan(&a, Kind::Launchd).unwrap().name,
        plan(&b, Kind::Launchd).unwrap().name
    );
}

#[test]
fn process_and_external_units_carry_no_file_path() {
    let (_dir, canonical) = canonical();
    for kind in [Kind::Process, Kind::External, Kind::Unsupported] {
        assert_eq!(
            plan(&canonical, kind).unwrap().path,
            std::path::PathBuf::new()
        );
    }
}

#[test]
fn kind_as_str_round_trips_through_the_env_override() {
    for (kind, name) in [
        (Kind::Launchd, "launchd"),
        (Kind::Systemd, "systemd"),
        (Kind::Process, "process"),
        (Kind::External, "external"),
    ] {
        assert_eq!(kind.as_str(), name);
    }
}

#[test]
fn detect_honors_the_override_and_rejects_garbage() {
    for value in ["launchd", "systemd", "process", "external", "LAUNCHD"] {
        // SAFETY: this test is in the env-mutating nextest group (max-threads=1).
        unsafe { std::env::set_var("COMEMORY_DAEMON_SUPERVISOR", value) };
        let kind = comemory::domains::sync::daemon::supervisor::detect().unwrap();
        assert_eq!(kind.as_str(), value.to_ascii_lowercase());
    }
    // SAFETY: this test is in the env-mutating nextest group (max-threads=1).
    unsafe { std::env::set_var("COMEMORY_DAEMON_SUPERVISOR", "nonsense") };
    assert!(comemory::domains::sync::daemon::supervisor::detect().is_err());
    // SAFETY: this test is in the env-mutating nextest group (max-threads=1).
    unsafe { std::env::remove_var("COMEMORY_DAEMON_SUPERVISOR") };
}

#[test]
fn no_override_picks_this_hosts_native_backend() {
    // SAFETY: this test is in the env-mutating nextest group (max-threads=1).
    unsafe { std::env::remove_var("COMEMORY_DAEMON_SUPERVISOR") };
    let kind = comemory::domains::sync::daemon::supervisor::detect().unwrap();
    #[cfg(target_os = "macos")]
    assert_eq!(kind.as_str(), "launchd");
    #[cfg(target_os = "linux")]
    assert!(matches!(kind.as_str(), "systemd" | "process"));
}

/// A nonexistent GUI domain makes the real launchd reject both bootstrap
/// and bootout, without installing a job in the developer's session.
#[cfg(target_os = "macos")]
#[test]
fn launchd_activation_reports_failure_when_bootstrap_and_bootout_fail() {
    use comemory::domains::sync::daemon::supervisor::Unit;
    use comemory::domains::sync::daemon_templates::render_launch_agent_plist;

    let (dir, canonical) = canonical();
    let unit = Unit {
        name: "io.comemory.test.unavailable-domain".into(),
        path: dir.path().join("unavailable-domain.plist"),
    };
    std::fs::write(
        &unit.path,
        render_launch_agent_plist(
            &unit.name,
            std::path::Path::new("/usr/bin/true"),
            &canonical,
        ),
    )
    .unwrap();
    let error = super::bootstrap_or_replace(&unit, "gui/4294967295")
        .expect_err("failed bootstrap and bootout must permit process fallback");
    assert!(error.to_string().contains("launchctl bootout"), "{error}");
}

#[test]
fn a_temp_data_directory_is_never_given_a_native_unit() {
    // SAFETY: this test is in the env-mutating nextest group (max-threads=1).
    unsafe { std::env::remove_var("COMEMORY_DAEMON_SUPERVISOR") };
    let (_dir, canonical) = canonical();
    assert_eq!(for_data_dir(Kind::Launchd, &canonical), Kind::Process);
    assert_eq!(for_data_dir(Kind::Systemd, &canonical), Kind::Process);
    assert_eq!(for_data_dir(Kind::External, &canonical), Kind::External);
    let home = std::path::Path::new("/Users/someone/.comemory");
    assert_eq!(for_data_dir(Kind::Launchd, home), Kind::Launchd);
}

#[test]
fn every_canonical_spelling_of_a_system_temp_root_is_throwaway() {
    // SAFETY: this test is in the env-mutating nextest group (max-threads=1).
    unsafe { std::env::remove_var("COMEMORY_DAEMON_SUPERVISOR") };
    // macOS resolves `/var/tmp` to `/private/var/tmp` before the directory is
    // judged, so the resolved spelling must count as much as the literal one.
    for root in [
        "/tmp",
        "/private/tmp",
        "/var/tmp",
        "/private/var/tmp",
        "/private/var/folders/zz/abc/T",
    ] {
        let dir = std::path::Path::new(root).join("comemory-data");
        assert_eq!(for_data_dir(Kind::Launchd, &dir), Kind::Process, "{root}");
    }
    // Component-wise: a sibling that merely shares the prefix is not temp.
    let sibling = std::path::Path::new("/tmpfs-data/comemory");
    assert_eq!(for_data_dir(Kind::Launchd, sibling), Kind::Launchd);
}

#[test]
fn detect_for_names_the_backend_that_will_really_supervise_the_directory() {
    // SAFETY: this test is in the env-mutating nextest group (max-threads=1).
    unsafe { std::env::remove_var("COMEMORY_DAEMON_SUPERVISOR") };
    let (_dir, canonical) = canonical();
    let kind = detect_for(&Paths::new(&canonical)).unwrap();
    assert_eq!(kind, Kind::Process, "a temp directory reports `process`");
    // SAFETY: this test is in the env-mutating nextest group (max-threads=1).
    unsafe { std::env::set_var("COMEMORY_DAEMON_SUPERVISOR", "nonsense") };
    let rejected = detect_for(&Paths::new(&canonical));
    // SAFETY: this test is in the env-mutating nextest group (max-threads=1).
    unsafe { std::env::remove_var("COMEMORY_DAEMON_SUPERVISOR") };
    assert!(rejected.is_err(), "a bad override still surfaces");
}

#[test]
fn an_explicit_override_still_gets_a_native_unit_for_a_temp_directory() {
    let (_dir, canonical) = canonical();
    // SAFETY: this test is in the env-mutating nextest group (max-threads=1).
    unsafe { std::env::set_var("COMEMORY_DAEMON_SUPERVISOR", "launchd") };
    let forced = for_data_dir(Kind::Launchd, &canonical);
    let reported = detect_for(&Paths::new(&canonical)).unwrap();
    // SAFETY: this test is in the env-mutating nextest group (max-threads=1).
    // Restored before asserting so a failure cannot leak the override.
    unsafe { std::env::remove_var("COMEMORY_DAEMON_SUPERVISOR") };
    assert_eq!(forced, Kind::Launchd);
    assert_eq!(reported, Kind::Launchd);
}

#[test]
fn a_directory_not_yet_created_is_judged_by_its_nearest_existing_ancestor() {
    // SAFETY: this test is in the env-mutating nextest group (max-threads=1).
    unsafe { std::env::remove_var("COMEMORY_DAEMON_SUPERVISOR") };
    let (_dir, canonical) = canonical();
    let missing = canonical.join("not/yet/created");
    assert_eq!(for_data_dir(Kind::Launchd, &missing), Kind::Process);
    // `$TMPDIR` reached through the `/var` symlink; `status` never creates it.
    if let Ok(rest) = missing.strip_prefix("/private/var") {
        let spelled = std::path::Path::new("/var").join(rest);
        assert!(!spelled.exists());
        let kind = detect_for(&Paths::new(&spelled)).unwrap();
        assert_eq!(kind, Kind::Process, "{}", spelled.display());
    }
}

#[test]
fn a_directory_inside_the_home_is_never_throwaway() {
    let (dir, canonical) = canonical();
    let home = canonical.join("alice");
    std::fs::create_dir_all(home.join(".comemory")).unwrap();
    let previous = std::env::var_os("HOME");
    // SAFETY: this test is in the env-mutating nextest group (max-threads=1).
    unsafe {
        std::env::remove_var("COMEMORY_DAEMON_SUPERVISOR");
        std::env::set_var("HOME", &home);
    }
    let inside = for_data_dir(Kind::Launchd, &home.join(".comemory"));
    let outside = for_data_dir(Kind::Launchd, &canonical.join("scratch"));
    // SAFETY: restored before asserting so a failure cannot leak HOME.
    unsafe {
        match previous {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }
    drop(dir);
    assert_eq!(inside, Kind::Launchd, "a home directory keeps its unit");
    assert_eq!(
        outside,
        Kind::Process,
        "a sibling of the home is still temp"
    );
}
