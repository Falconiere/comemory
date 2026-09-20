#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! GitHub repository identity parsing against the repository's real origin.

use super::canonical_github_repository;

#[test]
fn current_repository_remote_forms_share_one_canonical_identity() {
    for remote in [
        "git@github.com:Falconiere/comemory.git",
        "ssh://git@github.com/Falconiere/comemory.git",
        "https://github.com/Falconiere/comemory.git",
    ] {
        assert_eq!(
            canonical_github_repository(remote).as_deref(),
            Some("falconiere/comemory"),
            "remote {remote}"
        );
    }
}

#[test]
fn unsupported_or_ambiguous_remotes_have_no_identity() {
    for remote in [
        "http://github.com/Falconiere/comemory.git",
        "https://gitlab.com/Falconiere/comemory.git",
        "https://github.com/Falconiere/comemory/issues",
        "git@github.com:Falconiere/comemory/extra.git",
        "git@github.com:comemory.git",
        "https://user@github.com/Falconiere/comemory.git",
        "https://github.com:443/Falconiere/comemory.git",
    ] {
        assert_eq!(canonical_github_repository(remote), None, "remote {remote}");
    }
}
