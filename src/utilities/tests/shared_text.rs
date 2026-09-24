#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The shared free-text policy over real query shapes: prose passes, machine
//! paths are stripped, and a secret withholds the field.

use crate::utilities::shared_text::{PATH, Shared, for_share};

/// AWS's documented example key id, assembled from parts so this file is not
/// itself a committed credential to the repository's secret gate.
fn example_key() -> String {
    format!("AKIA{}{}", "IOSFODNN7", "EXAMPLE")
}

#[test]
fn ordinary_prose_and_repository_paths_pass_unchanged() {
    assert_eq!(
        for_share("how does the replica journal order acceptances"),
        Shared::Kept("how does the replica journal order acceptances".to_string())
    );
    assert_eq!(
        for_share("where is src/store/feedback.rs used"),
        Shared::Kept("where is src/store/feedback.rs used".to_string()),
        "a repository-relative path means the same thing on every machine"
    );
}

#[test]
fn absolute_machine_paths_are_stripped_in_every_spelling() {
    assert_eq!(
        for_share("why does /Users/alice/work/app/src/main.rs panic"),
        Shared::Kept(format!("why does {PATH} panic"))
    );
    assert_eq!(
        for_share("open ~/notes/todo.md and (C:\\Users\\bob\\repo) then \\\\server\\share"),
        Shared::Kept(format!("open {PATH} and ({PATH}) then {PATH}"))
    );
    assert_eq!(
        for_share("the root / and /tmp alone are not locations"),
        Shared::Kept("the root / and /tmp alone are not locations".to_string())
    );
}

#[test]
fn a_secret_withholds_the_whole_field() {
    assert_eq!(
        for_share(&format!("rotate {} before release", example_key())),
        Shared::Withheld
    );
}

#[test]
fn a_secret_inside_a_path_still_withholds_the_whole_field() {
    // Stripping alone would already remove the key, but text that carried one
    // anywhere is withheld outright rather than shared around the hole.
    assert_eq!(
        for_share(&format!("read /home/ci/{}/config", example_key())),
        Shared::Withheld
    );
}
