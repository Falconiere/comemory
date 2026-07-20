//! Learning-loop evaluation: golden sets, metrics, the eval runner,
//! reformulation mining, and blend-weight tuning.

pub mod bandit;
pub(crate) mod bandit_rng;
pub mod golden;
pub mod metrics;
pub mod mine;
pub mod runner;
pub mod tune;
