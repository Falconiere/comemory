#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::sync::match_key`].

use comemory::sync::{
    AllowlistRepo, MatchOutcome, classify_repo, normalize_git_remote, normalize_repo_label,
};

fn codasignal_foo() -> Vec<AllowlistRepo> {
    vec![AllowlistRepo {
        full_name: "codasignal/foo".into(),
        name: "foo".into(),
    }]
}

#[test]
fn normalize_repo_label_lowercases_and_trims() {
    assert_eq!(normalize_repo_label("  CodaSignal/Foo  "), "codasignal/foo");
}

#[test]
fn classify_exact_full_name_allowed() {
    let outcome = classify_repo("CodaSignal/foo", &codasignal_foo());
    assert_eq!(outcome, MatchOutcome::Allowed);
}

#[test]
fn classify_unique_basename_allowed() {
    let outcome = classify_repo("foo", &codasignal_foo());
    assert_eq!(outcome, MatchOutcome::Allowed);
}

#[test]
fn classify_empty_label_skipped_personal() {
    assert_eq!(
        classify_repo("", &codasignal_foo()),
        MatchOutcome::SkippedPersonal
    );
    assert_eq!(
        classify_repo("   ", &codasignal_foo()),
        MatchOutcome::SkippedPersonal
    );
}

#[test]
fn classify_unknown_repo_skipped_not_in_org() {
    assert_eq!(
        classify_repo("my-dotfiles", &codasignal_foo()),
        MatchOutcome::SkippedNotInOrg
    );
    assert_eq!(
        classify_repo("other/bar", &codasignal_foo()),
        MatchOutcome::SkippedNotInOrg
    );
}

#[test]
fn classify_ambiguous_basename_skipped() {
    let allowlist = vec![
        AllowlistRepo {
            full_name: "org/cli".into(),
            name: "cli".into(),
        },
        AllowlistRepo {
            full_name: "other/cli".into(),
            name: "cli".into(),
        },
    ];
    assert_eq!(
        classify_repo("cli", &allowlist),
        MatchOutcome::SkippedAmbiguous
    );
    assert_eq!(classify_repo("org/cli", &allowlist), MatchOutcome::Allowed);
}

#[test]
fn normalize_git_remote_https_and_ssh() {
    assert_eq!(
        normalize_git_remote("https://github.com/CodaSignal/foo.git"),
        Some("codasignal/foo".into())
    );
    assert_eq!(
        normalize_git_remote("git@github.com:CodaSignal/foo.git"),
        Some("codasignal/foo".into())
    );
    assert_eq!(normalize_git_remote("not-a-remote"), None);
}
