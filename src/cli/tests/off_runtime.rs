#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirrors `src/cli/off_runtime.rs`.
//!
//! The bug this exists to prevent: `main` is `#[tokio::main]`, so a subcommand
//! body runs with a runtime handle current, and `reqwest::blocking` panics on
//! drop there. Both tests below build a multi-thread runtime — the same kind
//! `#[tokio::main]` builds — and make a real HTTP call against a real loopback
//! socket, which is the only way to reproduce it.

use comemory::cli::off_runtime::off_runtime;
use comemory::sync::client;

use crate::test_common::sync_platform_server::{SyncPlatformServer, SyncPlatformState};

#[test]
fn a_blocking_platform_call_survives_a_live_tokio_runtime() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let manifest = runtime
        .block_on(async { off_runtime(|| client::fetch_manifest(&server.base, &secret)) })
        .expect("a blocking platform call must work from inside the runtime");

    assert_eq!(manifest.buckets.len(), 256);
}

#[test]
fn an_error_from_the_call_propagates_rather_than_being_swallowed() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    server.update(|st| st.sync_unavailable = true);
    let secret = server.snapshot().secret;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let err = runtime
        .block_on(async { off_runtime(|| client::fetch_manifest(&server.base, &secret)) })
        .expect_err("a 500 must reach the caller, not be lost with the thread");
    assert!(err.to_string().contains("500"), "got: {err}");
}
