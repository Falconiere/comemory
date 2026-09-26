#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory sync daemon` end to end (real coordinators, real sockets) is
//! `tests/replica_daemon.rs` (#257); this file only proves clap accepts the
//! full lifecycle verb set, including the hidden deprecated aliases.

use clap::Parser as _;
use comemory::cli::Cli;

fn parses(args: &[&str]) -> bool {
    let mut full = vec!["comemory"];
    full.extend_from_slice(args);
    Cli::try_parse_from(full).is_ok()
}

#[test]
fn every_lifecycle_verb_and_the_hidden_aliases_parse() {
    for verb in [
        "ensure",
        "status",
        "restart",
        "repair",
        "run",
        "stop",
        "uninstall",
        "install",
        "start",
    ] {
        assert!(parses(&["sync", "daemon", verb]), "{verb} must parse");
    }
}

#[test]
fn no_no_daemon_flag_exists_anywhere_on_sync() {
    assert!(!parses(&["sync", "--no-daemon"]));
    assert!(!parses(&["sync", "daemon", "ensure", "--no-daemon"]));
}
