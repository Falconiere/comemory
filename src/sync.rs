//! Cloud-sync client helpers: credentials, redaction, and the local skip filter.

pub mod auth_file;
pub mod auto;
pub mod client;
pub mod initial;
pub mod pull;
pub mod push;
pub mod redact;
pub mod skip_repos;
pub mod verify;

pub use auth_file::AuthFile;
pub use initial::{InitialSyncStats, run_initial_sync};
pub use redact::{Finding, RULE_SET_VERSION, findings, redact, scan, scan_with_override};
pub use skip_repos::{SkipMatcher, normalize_repo_label};
