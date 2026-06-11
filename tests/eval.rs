//! Test-binary shim for the eval module. Submodules live in tests/eval/.

#[path = "eval/golden.rs"]
mod golden;

#[path = "eval/metrics.rs"]
mod metrics;

#[path = "eval/mine.rs"]
mod mine;

#[path = "eval/runner.rs"]
mod runner;

#[path = "eval/tune.rs"]
mod tune;
