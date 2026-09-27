#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::domains::sync::daemon::supervisor`].

use comemory::domains::sync::daemon::supervisor::{Kind, plan};

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
