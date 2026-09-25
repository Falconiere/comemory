#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Pure classification tests for this module's own `classify`; the real
//! verify/repair/fail behavior against real coordinators is
//! `tests/replica_daemon_2.rs`/`replica_daemon_4.rs` (#257).

use clap::Parser as _;

use crate::cli::daemon_preflight::{Classification, classify};
use crate::cli::{Cli, Cmd};

fn cmd(args: &[&str]) -> Cmd {
    let mut full = vec!["comemory"];
    full.extend_from_slice(args);
    Cli::try_parse_from(full).expect("parse").cmd
}

#[test]
fn lifecycle_and_introspection_commands_are_exempt() {
    for args in [
        vec!["serve"],
        vec!["completions", "zsh"],
        vec!["upgrade", "--check"],
        vec!["doctor"],
        vec!["auth", "status"],
        vec!["sync", "--action", "status"],
        vec!["sync", "daemon", "ensure"],
        vec!["sync", "daemon", "status"],
        vec!["sync", "daemon", "run"],
    ] {
        assert_eq!(
            classify(&cmd(&args)),
            Classification::Exempt,
            "{args:?} must be exempt"
        );
    }
}

#[test]
fn logout_is_best_effort() {
    assert_eq!(
        classify(&cmd(&["auth", "logout"])),
        Classification::BestEffort
    );
}

#[test]
fn ordinary_data_commands_and_login_and_mcp_and_watch_are_required() {
    for args in [
        vec!["save", "body"],
        vec!["search", "query"],
        vec!["list"],
        vec!["auth", "login"],
        vec!["sync", "--action", "auto"],
        vec!["sync"],
        vec!["mcp"],
        vec!["watch"],
        vec!["gc"],
    ] {
        assert_eq!(
            classify(&cmd(&args)),
            Classification::Required,
            "{args:?} must require a verified coordinator"
        );
    }
}
