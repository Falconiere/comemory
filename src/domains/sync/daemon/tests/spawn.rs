#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`super::spawn`] is exercised end to end (real detached process, real
//! log files) by `tests/replica_daemon.rs`'s `Kind::Process` scenarios;
//! nothing here needs a pure unit test of its own.
