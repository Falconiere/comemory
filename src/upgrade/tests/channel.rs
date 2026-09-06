#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Install-channel detection: which paths read as Homebrew, when a cargo bin
//! directory counts as `cargo install`, and everything else as standalone.
//! The cargo-home branch goes through `detect_with`, so no test here touches
//! the process environment (and none needs `unsafe`).

use std::path::Path;

use comemory::upgrade::channel::{Channel, crates_toml_source, detect, detect_with};

#[test]
fn homebrew_paths_are_recognized() {
    for p in [
        "/opt/homebrew/Cellar/comemory/0.18.2/bin/comemory",
        "/usr/local/Cellar/comemory/0.18.2/bin/comemory",
        "/opt/homebrew/bin/comemory",
        "/home/linuxbrew/.linuxbrew/Cellar/comemory/0.18.2/bin/comemory",
    ] {
        assert_eq!(detect(Path::new(p)), Channel::Homebrew, "{p}");
    }
}

#[test]
fn a_directory_off_the_cargo_bin_is_standalone() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("comemory");
    assert_eq!(
        detect(&exe),
        Channel::Standalone {
            dir: dir.path().to_path_buf()
        }
    );
}

#[test]
fn crates_toml_source_reads_the_comemory_entry_only() {
    let text = "[v1]\n\
        \"cargo-nextest 0.9.72 (registry+https://github.com/rust-lang/crates.io-index)\" = [\"cargo-nextest\"]\n\
        \"comemory 0.18.2 (path+file:///root/projects/comemory)\" = [\"comemory\"]\n";
    assert_eq!(
        crates_toml_source(text).as_deref(),
        Some("path+file:///root/projects/comemory")
    );
    assert_eq!(crates_toml_source("[v1]\n"), None);
    assert_eq!(
        crates_toml_source("\"comemory-extra 1.0.0 (git+https://x)\" = [\"x\"]\n"),
        None,
        "a crate whose name merely starts with comemory must not match"
    );
}

#[test]
fn cargo_install_lists_the_recorded_source() {
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        home.path().join(".crates.toml"),
        "[v1]\n\"comemory 0.18.2 (git+https://github.com/Falconiere/comemory#abc)\" = [\"comemory\"]\n",
    )
    .unwrap();
    let listed = detect_with(&bin.join("comemory"), Some(home.path()));
    let elsewhere = detect_with(
        &home.path().join("other").join("comemory"),
        Some(home.path()),
    );
    let no_home = detect_with(&bin.join("comemory"), None);
    assert_eq!(
        listed,
        Channel::CargoInstall {
            source: "git+https://github.com/Falconiere/comemory#abc".into()
        }
    );
    assert_eq!(
        elsewhere,
        Channel::Standalone {
            dir: home.path().join("other")
        },
        "the .crates.toml entry only speaks for the cargo bin directory"
    );
    assert_eq!(
        no_home,
        Channel::Standalone { dir: bin.clone() },
        "without a cargo home nothing can be a cargo install"
    );
    assert_eq!(listed.label(), "cargo install");
    assert_eq!(elsewhere.label(), "standalone");
    assert_eq!(Channel::Homebrew.label(), "homebrew");
}

#[test]
fn a_cargo_bin_without_a_comemory_entry_is_standalone() {
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(home.path().join(".crates.toml"), "[v1]\n").unwrap();
    assert_eq!(
        detect_with(&bin.join("comemory"), Some(home.path())),
        Channel::Standalone { dir: bin },
        "the cargo-dist installer also drops into cargo bin without a .crates.toml entry"
    );
}
