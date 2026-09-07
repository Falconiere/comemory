#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Unit tests for [`comemory::cloud::api_url`].
//!
//! `set_var` is mandatory-`unsafe` in Rust 2024; each use carries an
//! explicit `SAFETY:` line. Serialized with other env-mutating suites via
//! `.config/nextest.toml`.

use comemory::cloud::api_url::{DEFAULT_API_URL, resolve};

#[test]
fn resolve_defaults_to_production_api() {
    // SAFETY: nextest serializes this suite — set_var/remove_var cannot race.
    unsafe {
        std::env::remove_var("COMEMORY_API");
    }
    assert_eq!(resolve(None).unwrap(), DEFAULT_API_URL);
}

#[test]
fn resolve_prefers_cli_over_env_and_strips_slash() {
    // SAFETY: nextest serializes this suite — set_var/remove_var cannot race.
    unsafe {
        std::env::set_var("COMEMORY_API", "https://env.example/");
    }
    assert_eq!(
        resolve(Some("https://cli.example/")).unwrap(),
        "https://cli.example"
    );
    assert_eq!(resolve(None).unwrap(), "https://env.example");
    // SAFETY: nextest serializes this suite — set_var/remove_var cannot race.
    unsafe {
        std::env::remove_var("COMEMORY_API");
    }
}
