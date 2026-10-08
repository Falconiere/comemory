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
use std::process::{Command, Output};

fn run_supervisor_child(
    case: &str,
    override_value: Option<&str>,
    home: Option<&std::path::Path>,
) -> Output {
    let executable = std::env::current_exe().unwrap();
    let list = Command::new(&executable).arg("--list").output().unwrap();
    let test_name = String::from_utf8_lossy(&list.stdout)
        .lines()
        .filter_map(|line| line.strip_suffix(": test"))
        .find(|name| name.ends_with("::supervisor_environment_child"))
        .unwrap()
        .to_owned();
    let mut command = Command::new(executable);
    command
        .arg("--exact")
        .arg(test_name)
        .arg("--nocapture")
        .env("COMEMORY_SUPERVISOR_TEST_CASE", case)
        .env_remove("COMEMORY_DAEMON_SUPERVISOR");
    if let Some(value) = override_value {
        command.env("COMEMORY_DAEMON_SUPERVISOR", value);
    }
    if let Some(home) = home {
        command.env("HOME", home);
    }
    command.output().unwrap()
}

fn assert_supervisor_child(case: &str, override_value: Option<&str>) {
    let output = run_supervisor_child(case, override_value, None);
    assert!(
        output.status.success(),
        "child case {case} failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

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
        assert_supervisor_child("detect", Some(value));
    }
    assert_supervisor_child("detect", Some("nonsense"));
}

#[test]
fn no_override_picks_this_hosts_native_backend() {
    assert_supervisor_child("native", None);
}

#[test]
fn supervisor_environment_child() {
    let Ok(case) = std::env::var("COMEMORY_SUPERVISOR_TEST_CASE") else {
        return;
    };
    match case.as_str() {
        "detect" => {
            let kind = comemory::domains::sync::daemon::supervisor::detect();
            match std::env::var("COMEMORY_DAEMON_SUPERVISOR") {
                Ok(value) if value.eq_ignore_ascii_case("nonsense") => {
                    assert!(kind.is_err());
                }
                Ok(value) => assert_eq!(kind.unwrap().as_str(), value.to_ascii_lowercase()),
                Err(error) => panic!("child override missing: {error}"),
            }
        }
        "native" => {
            let kind = comemory::domains::sync::daemon::supervisor::detect().unwrap();
            #[cfg(target_os = "macos")]
            assert_eq!(kind.as_str(), "launchd");
            #[cfg(target_os = "linux")]
            assert!(matches!(kind.as_str(), "systemd" | "process"));
        }
        "temp" => {
            let (_dir, canonical) = canonical();
            assert_eq!(for_data_dir(Kind::Launchd, &canonical), Kind::Process);
            assert_eq!(for_data_dir(Kind::Systemd, &canonical), Kind::Process);
            assert_eq!(for_data_dir(Kind::External, &canonical), Kind::External);
            let home = std::path::Path::new("/Users/someone/.comemory");
            assert_eq!(for_data_dir(Kind::Launchd, home), Kind::Launchd);
        }
        "roots" => {
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
            let sibling = std::path::Path::new("/tmpfs-data/comemory");
            assert_eq!(for_data_dir(Kind::Launchd, sibling), Kind::Launchd);
        }
        "detect_for" => {
            let (_dir, canonical) = canonical();
            let kind = detect_for(&Paths::new(&canonical)).unwrap();
            assert_eq!(kind, Kind::Process, "a temp directory reports `process`");
        }
        "invalid_detect_for" => {
            let (_dir, canonical) = canonical();
            assert!(detect_for(&Paths::new(&canonical)).is_err());
        }
        "explicit" => {
            let (_dir, canonical) = canonical();
            assert_eq!(for_data_dir(Kind::Launchd, &canonical), Kind::Launchd);
            assert_eq!(detect_for(&Paths::new(&canonical)).unwrap(), Kind::Launchd);
        }
        "missing" => {
            let (_dir, canonical) = canonical();
            let missing = canonical.join("not/yet/created");
            assert_eq!(for_data_dir(Kind::Launchd, &missing), Kind::Process);
            if let Ok(rest) = missing.strip_prefix("/private/var") {
                let spelled = std::path::Path::new("/var").join(rest);
                assert!(!spelled.exists());
                let kind = detect_for(&Paths::new(&spelled)).unwrap();
                assert_eq!(kind, Kind::Process, "{}", spelled.display());
            }
        }
        "escaped" => {
            let (_dir, canonical) = canonical();
            let mut escaped = canonical.join("not-created");
            for _ in 0..100 {
                escaped.push("..");
            }
            escaped.push("etc/comemory-data");
            let expected = std::fs::canonicalize("/etc")
                .expect("system config directory exists")
                .join("comemory-data");
            assert_eq!(super::resolve(&escaped), expected);
        }
        "home" => {
            let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
            let data = home.join(".comemory");
            std::fs::create_dir_all(&data).unwrap();
            let canonical_home = std::fs::canonicalize(&data).unwrap();
            let inside = for_data_dir(Kind::Launchd, &canonical_home);
            let outside = for_data_dir(Kind::Launchd, &home.parent().unwrap().join("scratch"));
            assert_eq!(inside, Kind::Launchd, "a home directory keeps its unit");
            assert_eq!(
                outside,
                Kind::Process,
                "a sibling of the home is still temp"
            );
        }
        _ => panic!("unknown child case {case}"),
    }
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
    assert_supervisor_child("temp", None);
}

#[test]
fn every_canonical_spelling_of_a_system_temp_root_is_throwaway() {
    // macOS resolves `/var/tmp` to `/private/var/tmp` before the directory is
    // judged, so the resolved spelling must count as much as the literal one.
    assert_supervisor_child("roots", None);
}

#[test]
fn detect_for_names_the_backend_that_will_really_supervise_the_directory() {
    assert_supervisor_child("detect_for", None);
    assert_supervisor_child("invalid_detect_for", Some("nonsense"));
}

#[test]
fn an_explicit_override_still_gets_a_native_unit_for_a_temp_directory() {
    assert_supervisor_child("explicit", Some("launchd"));
}

#[test]
fn a_directory_not_yet_created_is_judged_by_its_nearest_existing_ancestor() {
    assert_supervisor_child("missing", None);
}

#[test]
fn a_missing_path_that_walks_out_of_temp_is_not_throwaway() {
    assert_supervisor_child("escaped", None);
}

#[test]
fn a_directory_inside_the_home_is_never_throwaway() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("alice");
    let output = run_supervisor_child("home", None, Some(&home));
    assert!(
        output.status.success(),
        "home case failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
